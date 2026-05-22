//! `sparrow import` — bulk-load records from JSON, CSV, or Parquet into a
//! running SparrowDB instance by calling a compiled HQL query once per record.
//!
//! # Usage
//!
//! ```text
//! sparrow import users.json  --query CreateUser   --target http://localhost:6969
//! sparrow import users.csv   --query CreateUser
//! sparrow import users.parquet --query CreateUser --workers 16
//! ```
//!
//! Every record in the file is posted as a JSON object to `POST /<query>`.
//! The object keys must match the named parameters of the HQL query.
//!
//! JSON files must be a top-level array: `[{...}, {...}, ...]`.
//! CSV files must have a header row; column names become parameter names.
//! Parquet files use column names as parameter names.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use eyre::{Context, Result, bail};
use futures_util::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, header};
use serde_json::{Map, Value};

// ── Format detection ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Json,
    Csv,
    Parquet,
}

impl std::fmt::Display for ImportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportFormat::Json => write!(f, "json"),
            ImportFormat::Csv => write!(f, "csv"),
            ImportFormat::Parquet => write!(f, "parquet"),
        }
    }
}

fn detect_format(path: &Path, override_fmt: Option<&str>) -> Result<ImportFormat> {
    if let Some(fmt) = override_fmt {
        return match fmt.to_ascii_lowercase().as_str() {
            "json" => Ok(ImportFormat::Json),
            "csv" => Ok(ImportFormat::Csv),
            "parquet" | "pq" => Ok(ImportFormat::Parquet),
            other => bail!("unknown format '{}' (valid: json, csv, parquet)", other),
        };
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "json" | "jsonl" => Ok(ImportFormat::Json),
        "csv" | "tsv" => Ok(ImportFormat::Csv),
        "parquet" | "pq" => Ok(ImportFormat::Parquet),
        other => bail!(
            "cannot infer format from extension '.{}' — use --format json|csv|parquet",
            other
        ),
    }
}

// ── Record readers ────────────────────────────────────────────────────────────

/// Read a JSON file that contains a top-level array of objects.
fn read_json(path: &Path) -> Result<Vec<Map<String, Value>>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let top: Value = serde_json::from_str(&content)
        .with_context(|| format!("parsing JSON from {}", path.display()))?;

    match top {
        Value::Array(arr) => arr
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_object()
                    .cloned()
                    .ok_or_else(|| eyre::eyre!("element [{}] is not a JSON object", i))
            })
            .collect(),
        _ => bail!(
            "{} must be a JSON array — got {}",
            path.display(),
            top.type_name()
        ),
    }
}

/// Read a CSV file.  Header row becomes parameter names; each subsequent row is
/// one record.  Values are type-inferred: integer → `Number`, float → `Number`,
/// `true`/`false` → `Bool`, empty → `Null`, everything else → `String`.
fn read_csv(path: &Path) -> Result<Vec<Map<String, Value>>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("opening CSV {}", path.display()))?;

    let headers: Vec<String> = rdr
        .headers()
        .with_context(|| "reading CSV headers")?
        .iter()
        .map(|s| s.to_string())
        .collect();

    if headers.is_empty() {
        bail!("CSV file has no headers");
    }

    let mut records = Vec::new();
    for (row_idx, result) in rdr.records().enumerate() {
        let row = result.with_context(|| format!("reading CSV row {}", row_idx + 2))?;
        let mut obj = Map::new();
        for (header, field) in headers.iter().zip(row.iter()) {
            obj.insert(header.clone(), infer_csv_value(field));
        }
        records.push(obj);
    }

    Ok(records)
}

fn infer_csv_value(s: &str) -> Value {
    if s.is_empty() {
        return Value::Null;
    }
    if s.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if s.eq_ignore_ascii_case("null") || s.eq_ignore_ascii_case("none") {
        return Value::Null;
    }
    // Try integer first, then float
    if let Ok(n) = s.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

/// Read a Parquet file using polars, then convert every row to a
/// `serde_json::Map` by serialising the DataFrame to a JSON array and parsing
/// it back through serde_json.  This keeps the column-type handling inside
/// polars where it belongs.
fn read_parquet(path: &Path) -> Result<Vec<Map<String, Value>>> {
    use polars::prelude::*;

    let mut df = LazyFrame::scan_parquet(path, ScanArgsParquet::default())
        .with_context(|| format!("scanning parquet {}", path.display()))?
        .collect()
        .with_context(|| format!("loading parquet {}", path.display()))?;

    // Serialise the DataFrame to a JSON array using polars' built-in writer,
    // then parse each object into a serde_json Map.
    let mut buf: Vec<u8> = Vec::with_capacity(df.height() * 64);
    JsonWriter::new(&mut buf)
        .with_json_format(JsonFormat::Json)
        .finish(&mut df)
        .map_err(|e| eyre::eyre!("serialising parquet to JSON: {e}"))?;

    let rows: Vec<Map<String, Value>> = serde_json::from_slice(&buf)
        .with_context(|| "parsing polars JSON output")?;

    Ok(rows)
}

// ── HTTP transport ────────────────────────────────────────────────────────────

fn build_client(token: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder()
        .pool_max_idle_per_host(128)
        .tcp_nodelay(true);

    if let Some(tok) = token {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            header::HeaderValue::from_str(tok)
                .map_err(|_| eyre::eyre!("auth token contains invalid header characters"))?,
        );
        builder = builder.default_headers(headers);
    }

    builder.build().map_err(|e| eyre::eyre!("building HTTP client: {e}"))
}

fn normalize_url(raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", raw.trim_end_matches('/'))
    }
}

// ── Per-record query resolution ───────────────────────────────────────────────

/// Determine the HQL query name to use for a single record.
///
/// - If `query_column` is set: read the value from that field in the record
///   and **remove** the field so it is not forwarded as a query parameter.
///   Falls back to `query` when the column is absent or empty.
/// - If `query_column` is not set: use `query` directly.
///
/// Returns an error when no query name can be determined.
pub fn resolve_query(
    record: &mut Map<String, Value>,
    query: Option<&str>,
    query_column: Option<&str>,
) -> Result<String> {
    if let Some(col) = query_column {
        let val = record.remove(col).and_then(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s),
            _ => None,
        });
        match val {
            Some(q) => Ok(q),
            None => query
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    eyre::eyre!(
                        "record is missing the '{}' column and no --query fallback was given",
                        col
                    )
                }),
        }
    } else {
        query
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| eyre::eyre!("--query or --query-column is required"))
    }
}

// ── Error handling mode ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    /// Skip failed records and continue importing.
    Continue,
    /// Abort the import on the first failure (in-flight requests still complete).
    Abort,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn run(
    file: PathBuf,
    query: Option<String>,
    query_column: Option<String>,
    target: String,
    workers: usize,
    token: Option<String>,
    dry_run: bool,
    format_override: Option<String>,
    on_error: OnError,
) -> Result<()> {
    let fmt = detect_format(&file, format_override.as_deref())?;
    println!("Reading {} ({})", file.display(), fmt);

    let records: Vec<Map<String, Value>> = match fmt {
        ImportFormat::Json => read_json(&file)?,
        ImportFormat::Csv => read_csv(&file)?,
        ImportFormat::Parquet => read_parquet(&file)?,
    };

    let total = records.len();

    if total == 0 {
        println!("No records found in {}.", file.display());
        return Ok(());
    }

    println!("  {} records parsed", total);

    if dry_run {
        println!("(--dry-run: skipping HTTP requests)");
        let sample_count = total.min(3);
        println!("First {} record(s):", sample_count);
        for mut rec in records.into_iter().take(sample_count) {
            // Show which query would be called for each record
            match resolve_query(
                &mut rec,
                query.as_deref(),
                query_column.as_deref(),
            ) {
                Ok(q) => println!(
                    "  → {} {}", q,
                    serde_json::to_string(&rec).unwrap_or_default()
                ),
                Err(e) => println!("  ✗ {e}"),
            }
        }
        return Ok(());
    }

    let base_url = Arc::new(normalize_url(&target));
    let client = Arc::new(build_client(token.as_deref())?);
    let query = Arc::new(query);
    let query_column = Arc::new(query_column);

    // When using a static query, show the full URL up front.
    // When per-record routing is active, just show the base.
    if query_column.is_none() {
        let static_url = format!("{}/{}", base_url, query.as_deref().unwrap_or(""));
        println!("Importing → {} ({} workers)", static_url, workers);
    } else {
        println!("Importing → {} (<per-record routing>, {} workers)", base_url, workers);
    }

    let ok_count = Arc::new(AtomicU64::new(0));
    let err_count = Arc::new(AtomicU64::new(0));
    let aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let pb = Arc::new({
        let bar = ProgressBar::new(total as u64);
        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] \
                     {pos}/{len} ({per_sec}) {msg}",
                )
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"),
        );
        bar
    });

    let start = Instant::now();

    stream::iter(records.into_iter())
        .map(|mut record| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            let query = Arc::clone(&query);
            let query_column = Arc::clone(&query_column);
            let ok_count = Arc::clone(&ok_count);
            let err_count = Arc::clone(&err_count);
            let aborted = Arc::clone(&aborted);
            let pb = Arc::clone(&pb);
            async move {
                if aborted.load(Ordering::Relaxed) {
                    return;
                }

                let url = match resolve_query(
                    &mut record,
                    query.as_deref(),
                    query_column.as_deref(),
                ) {
                    Ok(q) => format!("{}/{}", base_url, q),
                    Err(e) => {
                        err_count.fetch_add(1, Ordering::Relaxed);
                        pb.println(format!("  ✗ {e}"));
                        if on_error == OnError::Abort {
                            aborted.store(true, Ordering::Relaxed);
                        }
                        pb.inc(1);
                        return;
                    }
                };

                match client.post(&url).json(&record).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        ok_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        err_count.fetch_add(1, Ordering::Relaxed);
                        pb.println(format!("  ✗ HTTP {status}: {body}"));
                        if on_error == OnError::Abort {
                            aborted.store(true, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        err_count.fetch_add(1, Ordering::Relaxed);
                        pb.println(format!("  ✗ connection error: {e}"));
                        if on_error == OnError::Abort {
                            aborted.store(true, Ordering::Relaxed);
                        }
                    }
                }

                pb.inc(1);
            }
        })
        .buffer_unordered(workers)
        .for_each(|()| async {})
        .await;

    pb.finish_and_clear();

    let elapsed = start.elapsed();
    let ok = ok_count.load(Ordering::Relaxed);
    let err = err_count.load(Ordering::Relaxed);
    let throughput = ok as f64 / elapsed.as_secs_f64().max(0.001);

    println!(
        "✓ {}/{} records imported  ({:.2}s, {:.0} rec/s)",
        ok,
        total,
        elapsed.as_secs_f64(),
        throughput
    );

    if err > 0 {
        eprintln!("{} record(s) failed to import.", err);
        std::process::exit(1);
    }

    Ok(())
}

// ── helper: type name for error messages ─────────────────────────────────────

trait TypeName {
    fn type_name(&self) -> &'static str;
}

impl TypeName for Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── format detection ─────────────────────────────────────────────────────

    #[test]
    fn detect_format_by_extension() {
        assert_eq!(detect_format(Path::new("a.json"), None).unwrap(), ImportFormat::Json);
        assert_eq!(detect_format(Path::new("a.csv"), None).unwrap(), ImportFormat::Csv);
        assert_eq!(detect_format(Path::new("a.parquet"), None).unwrap(), ImportFormat::Parquet);
        assert_eq!(detect_format(Path::new("a.pq"), None).unwrap(), ImportFormat::Parquet);
    }

    #[test]
    fn detect_format_override() {
        assert_eq!(
            detect_format(Path::new("data.bin"), Some("json")).unwrap(),
            ImportFormat::Json
        );
        assert_eq!(
            detect_format(Path::new("data.bin"), Some("CSV")).unwrap(),
            ImportFormat::Csv
        );
    }

    #[test]
    fn detect_format_unknown_extension_errors() {
        assert!(detect_format(Path::new("data.xlsx"), None).is_err());
    }

    // ── infer_csv_value ──────────────────────────────────────────────────────

    #[test]
    fn csv_value_inference() {
        assert_eq!(infer_csv_value(""), Value::Null);
        assert_eq!(infer_csv_value("null"), Value::Null);
        assert_eq!(infer_csv_value("true"), Value::Bool(true));
        assert_eq!(infer_csv_value("False"), Value::Bool(false));
        assert_eq!(infer_csv_value("42"), Value::Number(42_i64.into()));
        assert_eq!(
            infer_csv_value("3.14"),
            Value::Number(serde_json::Number::from_f64(3.14).unwrap())
        );
        assert_eq!(infer_csv_value("hello"), Value::String("hello".into()));
    }

    // ── read_json ────────────────────────────────────────────────────────────

    #[test]
    fn read_json_array() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"[{{"name":"Alice","age":30}},{{"name":"Bob","age":25}}]"#).unwrap();
        let recs = read_json(f.path()).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["name"], Value::String("Alice".into()));
        assert_eq!(recs[1]["age"], Value::Number(25_i64.into()));
    }

    #[test]
    fn read_json_rejects_non_array() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"name":"Alice"}}"#).unwrap();
        assert!(read_json(f.path()).is_err());
    }

    #[test]
    fn read_json_rejects_non_object_elements() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"[1, 2, 3]"#).unwrap();
        assert!(read_json(f.path()).is_err());
    }

    // ── read_csv ─────────────────────────────────────────────────────────────

    #[test]
    fn read_csv_basic() {
        let mut f = NamedTempFile::with_suffix(".csv").unwrap();
        write!(f, "name,age,active\nAlice,30,true\nBob,25,false\n").unwrap();
        let recs = read_csv(f.path()).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["name"], Value::String("Alice".into()));
        assert_eq!(recs[0]["age"], Value::Number(30_i64.into()));
        assert_eq!(recs[0]["active"], Value::Bool(true));
        assert_eq!(recs[1]["active"], Value::Bool(false));
    }

    #[test]
    fn read_csv_whitespace_trim() {
        let mut f = NamedTempFile::with_suffix(".csv").unwrap();
        write!(f, " name , age \n Alice , 30 \n").unwrap();
        let recs = read_csv(f.path()).unwrap();
        assert_eq!(recs[0]["name"], Value::String("Alice".into()));
    }

    // ── normalize_url ────────────────────────────────────────────────────────

    #[test]
    fn normalize_url_adds_scheme() {
        assert_eq!(normalize_url("localhost:6969"), "http://localhost:6969");
        assert_eq!(normalize_url("http://localhost:6969/"), "http://localhost:6969");
        assert_eq!(normalize_url("https://prod.example.com"), "https://prod.example.com");
    }

    // ── resolve_query ────────────────────────────────────────────────────────

    fn make_record(fields: &[(&str, &str)]) -> Map<String, Value> {
        fields
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    /// When --query-column names a column present in the record, that value
    /// becomes the query name.
    #[test]
    fn resolve_query_uses_column_value() {
        let mut rec = make_record(&[("_query", "CreateUser"), ("name", "Alice")]);
        let q = resolve_query(&mut rec, None, Some("_query")).unwrap();
        assert_eq!(q, "CreateUser");
    }

    /// The routing column must be stripped from the record so it is not sent
    /// as a query parameter.
    #[test]
    fn resolve_query_strips_column_from_record() {
        let mut rec = make_record(&[("_query", "CreateUser"), ("name", "Alice")]);
        resolve_query(&mut rec, None, Some("_query")).unwrap();
        assert!(!rec.contains_key("_query"), "_query column should be removed");
        assert!(rec.contains_key("name"), "other columns must remain");
    }

    /// When the routing column is absent but --query provides a fallback,
    /// the fallback is used (no error).
    #[test]
    fn resolve_query_falls_back_to_default_query() {
        let mut rec = make_record(&[("name", "Alice")]);
        let q = resolve_query(&mut rec, Some("CreateUser"), Some("_query")).unwrap();
        assert_eq!(q, "CreateUser");
    }

    /// When the routing column is absent AND there is no --query fallback,
    /// resolve_query must return an error.
    #[test]
    fn resolve_query_errors_when_column_missing_and_no_fallback() {
        let mut rec = make_record(&[("name", "Alice")]);
        let result = resolve_query(&mut rec, None, Some("_query"));
        assert!(result.is_err(), "should error when column missing and no fallback");
    }

    /// With no --query-column at all, resolve_query returns the static --query.
    #[test]
    fn resolve_query_static_when_no_column_flag() {
        let mut rec = make_record(&[("name", "Alice")]);
        let q = resolve_query(&mut rec, Some("CreateUser"), None).unwrap();
        assert_eq!(q, "CreateUser");
    }

    /// If neither --query nor --query-column is provided, resolve_query errors.
    #[test]
    fn resolve_query_errors_when_neither_provided() {
        let mut rec = make_record(&[("name", "Alice")]);
        let result = resolve_query(&mut rec, None, None);
        assert!(result.is_err());
    }

    /// An empty string in the routing column is treated as missing and falls
    /// back to --query.
    #[test]
    fn resolve_query_treats_empty_column_as_missing() {
        let mut rec = make_record(&[("_query", ""), ("name", "Alice")]);
        let q = resolve_query(&mut rec, Some("CreateUser"), Some("_query")).unwrap();
        assert_eq!(q, "CreateUser");
    }
}
