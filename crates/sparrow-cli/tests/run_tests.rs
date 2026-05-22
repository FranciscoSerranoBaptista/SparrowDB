use sparrow_cli::commands::run::{resolve_binary, resolve_data_dir};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_resolve_binary_prefers_release_over_debug() {
    let tmp = TempDir::new().unwrap();
    let release_dir = tmp.path().join("release");
    let debug_dir = tmp.path().join("debug");
    fs::create_dir_all(&release_dir).unwrap();
    fs::create_dir_all(&debug_dir).unwrap();

    let release_bin = release_dir.join("sparrow-container");
    let debug_bin = debug_dir.join("sparrow-container");
    fs::write(&release_bin, b"").unwrap();
    fs::write(&debug_bin, b"").unwrap();

    let found = resolve_binary(tmp.path()).unwrap();
    assert_eq!(found, release_bin, "release binary should be preferred over debug");
}

#[test]
fn test_resolve_binary_falls_back_to_debug() {
    let tmp = TempDir::new().unwrap();
    let debug_dir = tmp.path().join("debug");
    fs::create_dir_all(&debug_dir).unwrap();
    let debug_bin = debug_dir.join("sparrow-container");
    fs::write(&debug_bin, b"").unwrap();

    let found = resolve_binary(tmp.path()).unwrap();
    assert_eq!(found, debug_bin);
}

#[test]
fn test_resolve_binary_errors_when_neither_exists() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_binary(tmp.path());
    assert!(result.is_err(), "should error when no binary found");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("No binary found"), "error should mention missing binary, got: {msg}");
}

#[test]
fn test_resolve_data_dir_uses_override_when_provided() {
    let override_dir = "/custom/data/path".to_string();
    let result = resolve_data_dir(Some(override_dir.clone()), None, None);
    assert_eq!(result, override_dir);
}

#[test]
fn test_resolve_data_dir_falls_back_to_sparrow_dir() {
    // No override, no project — should fall back to ~/.sparrow
    // sparrow-container will append /user, so database lands at ~/.sparrow/user
    let result = resolve_data_dir(None, None, None);
    assert!(
        result.ends_with(".sparrow") || result == "/tmp/sparrow-data",
        "fallback should end with .sparrow (sparrow-container appends /user), got: {result}"
    );
}
