// crates/sparrow-core/src/sparrow_gateway/settings.rs

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How a setting's current value was established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingSource {
    /// Value came from an environment variable at startup.
    Env,
    /// No env var was set; value is the compiled-in default.
    Default,
    /// Value was changed via `POST /settings` this session (ephemeral).
    Runtime,
}

impl SettingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingSource::Env => "env",
            SettingSource::Default => "default",
            SettingSource::Runtime => "runtime",
        }
    }
}

/// Hot-swappable operational settings for a running SparrowDB instance.
///
/// Mutable fields use `Arc<AtomicBool>` so changes are immediately visible
/// to all code paths that hold a clone of the Arc (including storage_core).
///
/// Immutable fields (like `worker_threads`) are read-only — included for
/// observability via `GET /settings` but rejected by `POST /settings`.
///
/// All changes via `POST /settings` are ephemeral. The next restart restores
/// env var values.
#[derive(Debug, Clone)]
pub struct RuntimeSettings {
    // ── mutable ───────────────────────────────────────────────────────────
    /// Skip BM25 index rebuild on writes. Equivalent to `SPARROW_SKIP_BM25_ON_WRITE=1`.
    pub skip_bm25_on_write: Arc<AtomicBool>,
    pub skip_bm25_source: Arc<Mutex<SettingSource>>,

    // ── immutable (observability only) ────────────────────────────────────
    /// Number of worker threads. Set at startup, not changeable at runtime.
    pub worker_threads: usize,
}

impl RuntimeSettings {
    /// Construct from environment variables, applying defaults where not set.
    pub fn from_env() -> Self {
        let (skip_value, skip_source) =
            match std::env::var("SPARROW_SKIP_BM25_ON_WRITE").as_deref() {
                Ok("1") | Ok("true") | Ok("True") | Ok("TRUE") => (true, SettingSource::Env),
                Ok(_) => (false, SettingSource::Env),
                Err(_) => (false, SettingSource::Default),
            };

        let worker_threads = std::env::var("SPARROW_WORKER_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|n| {
                let n = n.max(2);
                if n % 2 == 0 { n } else { n + 1 }
            })
            .unwrap_or_else(|| {
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
                    .max(1);
                let n = (cores * 4).min(64).max(2);
                if n % 2 == 0 { n } else { n + 1 }
            });

        RuntimeSettings {
            skip_bm25_on_write: Arc::new(AtomicBool::new(skip_value)),
            skip_bm25_source: Arc::new(Mutex::new(skip_source)),
            worker_threads,
        }
    }

    /// Toggle `skip_bm25_on_write` at runtime and mark source as "runtime".
    pub fn set_skip_bm25_on_write(&self, value: bool) {
        self.skip_bm25_on_write.store(value, Ordering::Relaxed);
        if let Ok(mut src) = self.skip_bm25_source.lock() {
            *src = SettingSource::Runtime;
        }
    }

    /// Serialize all settings to a JSON string for `GET /settings` response.
    pub fn to_json(&self) -> String {
        let skip_val = self.skip_bm25_on_write.load(Ordering::Relaxed);
        let skip_src = self
            .skip_bm25_source
            .lock()
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let workers = self.worker_threads;

        format!(
            r#"{{"settings":{{"skip_bm25_on_write":{{"value":{skip_val},"source":"{skip_src}","mutable":true}},"worker_threads":{{"value":{workers},"source":"env","mutable":false}}}}}}"#,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_reads_skip_bm25_default() {
        unsafe { std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE") };
        let s = RuntimeSettings::from_env();
        assert!(!s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "default");
    }

    #[test]
    fn from_env_reads_skip_bm25_from_env() {
        unsafe { std::env::set_var("SPARROW_SKIP_BM25_ON_WRITE", "1") };
        let s = RuntimeSettings::from_env();
        assert!(s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "env");
        unsafe { std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE") };
    }

    #[test]
    fn set_skip_bm25_changes_value_and_source() {
        unsafe { std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE") };
        let s = RuntimeSettings::from_env();
        s.set_skip_bm25_on_write(true);
        assert!(s.skip_bm25_on_write.load(Ordering::Relaxed));
        assert_eq!(s.skip_bm25_source.lock().unwrap().as_str(), "runtime");
    }

    #[test]
    fn worker_threads_is_even_and_at_least_2() {
        unsafe { std::env::remove_var("SPARROW_WORKER_THREADS") };
        let s = RuntimeSettings::from_env();
        assert!(s.worker_threads >= 2);
        assert_eq!(s.worker_threads % 2, 0);
    }

    #[test]
    fn to_json_includes_both_settings() {
        unsafe { std::env::remove_var("SPARROW_SKIP_BM25_ON_WRITE") };
        let s = RuntimeSettings::from_env();
        let json = s.to_json();
        assert!(json.contains("skip_bm25_on_write"));
        assert!(json.contains("worker_threads"));
        assert!(json.contains("\"mutable\":true"));
        assert!(json.contains("\"mutable\":false"));
    }
}
