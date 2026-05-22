//! `sparrow export` — write the results of a compiled HQL read query to a
//! JSON, CSV, or Parquet file.
//!
//! # Usage
//!
//! ```text
//! sparrow export users.json    --query GetAllUsers
//! sparrow export users.csv     --query GetAllUsers --key users
//! sparrow export snap.parquet  --query GetAllEvents --params '{"since":"2026-01-01"}'
//! ```
//!
//! SparrowDB query responses have the shape `{"var_name": [...], ...}`.
//! The exporter extracts the named array (auto-detected for single-variable
//! queries) and writes it to the output file.

use std::path::{Path, PathBuf};
use std::time::Instant;

use eyre::{Context, Result, bail};
use reqwest::{Client, header};
use serde_json::{Map, Value};

// ── Format detection ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
    Parquet,
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportFormat::Json => write!(f, "json"),
            ExportFormat::Csv => write!(f, "csv"),
            ExportFormat::Parquet => write!(f, "parquet"),
        }
    }
}

pub fn detect_format(path: &Path, override_fmt: Option<&str>) -> Result<ExportFormat> {
    if let Some(fmt) = override_fmt {
        return match fmt.to_ascii_lowercase().as_str() {
            "json" => Ok(ExportFormat::Json),
            "csv" => Ok(ExportFormat::Csv),
            "parquet" | "pq" => Ok(ExportFormat::Parquet),
            other => bail!("unknown format '{}' (valid: json, csv, parquet)", other),
        };
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" | "jsonl" => Ok(ExportFormat::Json),
        "csv" | "tsv" => Ok(ExportFormat::Csv),
        "parquet" | "pq" => Ok(ExportFormat::Parquet),
        other => bail!(
            "cannot infer format from extension '.{}' — use --format json|csv|parquet",
            other
        ),
    }
}

// ── Response extraction ───────────────────────────────────────────────────────

/// Extract the list of records from a SparrowDB query response.
///
/// All responses follow the shape `{"var_name": [...], ...}`.
///
/// - If `key` is given: extract `response[key]` as an array of objects.
/// - If `key` is `None` and there is exactly one key: use it automatically.
/// - If `key` is `None` and there are multiple keys: return an error asking
///   the caller to supply `--key`.
pub fn extract_records(
    response: Value,
    key: Option<&str>,
) -> Result<Vec<Map<String, Value>>> {
    let obj = match response {
        Value::Object(m) => m,
        other => bail!(
            "expected a JSON object from the query, got {}",
            type_name(&other)
        ),
    };

    let chosen_key: String = if let Some(k) = key {
        if !obj.contains_key(k) {
            bail!(
                "response has no key '{}'. Available keys: {}",
                k,
                obj.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        k.to_string()
    } else {
        match obj.len() {
            0 => bail!("query returned an empty response object"),
            1 => obj.keys().next().unwrap().clone(),
            _ => bail!(
                "query returned multiple keys ({}); use --key to pick one",
                obj.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        }
    };

    let arr = obj
        .into_iter()
        .find(|(k, _)| k == &chosen_key)
        .map(|(_, v)| v)
        .unwrap(); // safe: we checked the key exists above

    match arr {
        Value::Array(a) => a
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_object()
                    .cloned()
                    .ok_or_else(|| eyre::eyre!("record [{}] is not a JSON object", i))
            })
            .collect(),
        other => bail!(
            "key '{}' is not an array (got {})",
            chosen_key,
            type_name(&other)
        ),
    }
}

// ── File writers ──────────────────────────────────────────────────────────────

/// Write records as a JSON array.
pub fn write_json(records: &[Map<String, Value>], path: &Path, pretty: bool) -> Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let writer = std::io::BufWriter::new(file);
    if pretty {
        serde_json::to_writer_pretty(writer, records)
    } else {
        serde_json::to_writer(writer, records)
    }
    .with_context(|| format!("writing JSON to {}", path.display()))
}

/// Write records as CSV.  Headers are collected from the first record (in key
/// order), with any extra keys from later records appended.  Missing values
/// are written as empty cells.
pub fn write_csv(records: &[Map<String, Value>], path: &Path) -> Result<()> {
    if records.is_empty() {
        std::fs::write(path, b"")
            .with_context(|| format!("creating {}", path.display()))?;
        return Ok(());
    }

    // Stable header order: keys of first record, then any new keys from later records.
    let mut headers: Vec<String> = records[0].keys().cloned().collect();
    for rec in records.iter().skip(1) {
        for k in rec.keys() {
            if !headers.contains(k) {
                headers.push(k.clone());
            }
        }
    }

    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut wtr = csv::WriterBuilder::new().has_headers(true).from_writer(file);

    wtr.write_record(&headers)
        .with_context(|| "writing CSV header")?;

    for rec in records {
        let row: Vec<String> = headers
            .iter()
            .map(|h| json_value_to_csv(rec.get(h).unwrap_or(&Value::Null)))
            .collect();
        wtr.write_record(&row)
            .with_context(|| "writing CSV row")?;
    }

    wtr.flush().with_context(|| "flushing CSV writer")?;
    Ok(())
}

pub fn json_value_to_csv(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(), // arrays / nested objects → compact JSON string
    }
}

/// Write records as Parquet using polars.  Serialises via JSON so that polars
/// infers column types from the data.
pub fn write_parquet(records: &[Map<String, Value>], path: &Path) -> Result<()> {
    use polars::prelude::*;

    if records.is_empty() {
        let mut df = DataFrame::empty();
        let file = std::fs::File::create(path)
            .with_context(|| format!("creating {}", path.display()))?;
        ParquetWriter::new(file)
            .finish(&mut df)
            .map_err(|e| eyre::eyre!("writing empty parquet: {e}"))?;
        return Ok(());
    }

    let json_bytes =
        serde_json::to_vec(records).with_context(|| "serialising records to JSON")?;
    let cursor = std::io::Cursor::new(json_bytes);
    let mut df = JsonReader::new(cursor)
        .finish()
        .map_err(|e| eyre::eyre!("building DataFrame from records: {e}"))?;

    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    ParquetWriter::new(file)
        .finish(&mut df)
        .map_err(|e| eyre::eyre!("writing parquet: {e}"))?;

    Ok(())
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn build_client(token: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder()
        .pool_max_idle_per_host(16)
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

// ── Entry point ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn run(
    file: PathBuf,
    query: String,
    key: Option<String>,
    target: String,
    token: Option<String>,
    params: Option<String>,
    pretty: bool,
    format_override: Option<String>,
) -> Result<()> {
    let fmt = detect_format(&file, format_override.as_deref())?;

    // Parse --params, defaulting to empty object {}
    let params_value: Value = match params.as_deref() {
        None | Some("") => Value::Object(Map::new()),
        Some(s) => serde_json::from_str(s)
            .with_context(|| format!("--params must be a JSON object, got: {s}"))?,
    };
    if !params_value.is_object() {
        bail!("--params must be a JSON object, e.g. '{{\"min_age\": 25}}'");
    }

    let url = format!("{}/{}", normalize_url(&target), query);
    let client = build_client(token.as_deref())?;

    println!("Querying {} …", url);
    let start = Instant::now();

    let resp = client
        .post(&url)
        .json(&params_value)
        .send()
        .await
        .with_context(|| format!("connecting to {url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("query failed with HTTP {status}: {body}");
    }

    let body: Value = resp
        .json()
        .await
        .with_context(|| "parsing response as JSON")?;

    let elapsed_query = start.elapsed();
    let records = extract_records(body, key.as_deref())?;

    println!(
        "  {} records received ({:.2}s)",
        records.len(),
        elapsed_query.as_secs_f64()
    );

    if records.is_empty() {
        println!("No records to write.");
        return Ok(());
    }

    let write_start = Instant::now();
    match fmt {
        ExportFormat::Json => write_json(&records, &file, pretty)?,
        ExportFormat::Csv => write_csv(&records, &file)?,
        ExportFormat::Parquet => write_parquet(&records, &file)?,
    }

    println!(
        "✓ Wrote {} records to {} ({:.2}s)",
        records.len(),
        file.display(),
        write_start.elapsed().as_secs_f64()
    );

    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Read;
    use tempfile::NamedTempFile;

    // ── format detection ─────────────────────────────────────────────────────

    #[test]
    fn detect_format_by_extension() {
        assert_eq!(detect_format(Path::new("out.json"), None).unwrap(), ExportFormat::Json);
        assert_eq!(detect_format(Path::new("out.csv"), None).unwrap(), ExportFormat::Csv);
        assert_eq!(
            detect_format(Path::new("out.parquet"), None).unwrap(),
            ExportFormat::Parquet
        );
    }

    #[test]
    fn detect_format_override() {
        assert_eq!(
            detect_format(Path::new("out.bin"), Some("csv")).unwrap(),
            ExportFormat::Csv
        );
    }

    // ── extract_records ──────────────────────────────────────────────────────

    #[test]
    fn extract_records_single_key_auto() {
        let resp = json!({"users": [{"name": "Alice"}, {"name": "Bob"}]});
        let recs = extract_records(resp, None).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["name"], json!("Alice"));
    }

    #[test]
    fn extract_records_explicit_key() {
        let resp = json!({"users": [{"name": "Alice"}], "count": [{"n": 1}]});
        let recs = extract_records(resp, Some("users")).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["name"], json!("Alice"));
    }

    #[test]
    fn extract_records_multi_key_without_key_errors() {
        let resp = json!({"users": [{"name": "Alice"}], "count": [{"n": 1}]});
        assert!(extract_records(resp, None).is_err());
    }

    #[test]
    fn extract_records_missing_key_errors() {
        let resp = json!({"users": [{"name": "Alice"}]});
        assert!(extract_records(resp, Some("products")).is_err());
    }

    #[test]
    fn extract_records_non_object_response_errors() {
        assert!(extract_records(json!([{"name": "Alice"}]), None).is_err());
        assert!(extract_records(json!("hello"), None).is_err());
    }

    #[test]
    fn extract_records_empty_array() {
        let resp = json!({"users": []});
        let recs = extract_records(resp, None).unwrap();
        assert!(recs.is_empty());
    }

    // ── write_json ───────────────────────────────────────────────────────────

    #[test]
    fn write_json_roundtrip() {
        let records: Vec<Map<String, Value>> = vec![[
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]
        .into_iter()
        .collect()];
        let f = NamedTempFile::with_suffix(".json").unwrap();
        write_json(&records, f.path(), false).unwrap();

        let mut content = String::new();
        std::fs::File::open(f.path())
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        let parsed: Vec<Map<String, Value>> = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["name"], json!("Alice"));
    }

    #[test]
    fn write_json_empty_produces_empty_array() {
        let f = NamedTempFile::with_suffix(".json").unwrap();
        write_json(&[], f.path(), false).unwrap();
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(content.trim(), "[]");
    }

    // ── write_csv ────────────────────────────────────────────────────────────

    #[test]
    fn write_csv_roundtrip() {
        let records: Vec<Map<String, Value>> = vec![
            [("name".to_string(), json!("Alice")), ("age".to_string(), json!(30))]
                .into_iter()
                .collect(),
            [("name".to_string(), json!("Bob")), ("age".to_string(), json!(25))]
                .into_iter()
                .collect(),
        ];
        let f = NamedTempFile::with_suffix(".csv").unwrap();
        write_csv(&records, f.path()).unwrap();

        let mut rdr = csv::Reader::from_path(f.path()).unwrap();
        let headers: Vec<String> =
            rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
        assert!(headers.contains(&"name".to_string()));
        assert!(headers.contains(&"age".to_string()));
        let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn write_csv_missing_field_writes_empty_cell() {
        let records: Vec<Map<String, Value>> = vec![
            [
                ("name".to_string(), json!("Alice")),
                ("email".to_string(), json!("a@b.com")),
            ]
            .into_iter()
            .collect(),
            [("name".to_string(), json!("Bob"))].into_iter().collect(),
        ];
        let f = NamedTempFile::with_suffix(".csv").unwrap();
        write_csv(&records, f.path()).unwrap();

        let mut rdr = csv::Reader::from_path(f.path()).unwrap();
        let headers: Vec<String> =
            rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
        let email_col = headers.iter().position(|h| h == "email").unwrap();
        let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(rows[1].get(email_col).unwrap(), "");
    }

    #[test]
    fn write_csv_empty_produces_empty_file() {
        let f = NamedTempFile::with_suffix(".csv").unwrap();
        write_csv(&[], f.path()).unwrap();
        assert_eq!(std::fs::read(f.path()).unwrap(), b"");
    }

    // ── json_value_to_csv ────────────────────────────────────────────────────

    #[test]
    fn csv_value_conversion() {
        assert_eq!(json_value_to_csv(&Value::Null), "");
        assert_eq!(json_value_to_csv(&json!(true)), "true");
        assert_eq!(json_value_to_csv(&json!(42)), "42");
        assert_eq!(json_value_to_csv(&json!("hello")), "hello");
    }
}
