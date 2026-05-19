use crate::commands::data::{copy_dir_all, resolve_db_dir};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_resolve_db_dir_detects_lmdb_directly() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("data.mdb"), b"lmdb").unwrap();

    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path());
}

#[test]
fn test_resolve_db_dir_detects_lmdb_in_user_subdir() {
    let dir = tempdir().unwrap();
    let user_dir = dir.path().join("user");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join("data.mdb"), b"lmdb").unwrap();

    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, user_dir);
}

#[test]
fn test_resolve_db_dir_detects_rocksdb_directly() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("CURRENT"), b"MANIFEST-000001\n").unwrap();

    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path());
}

#[test]
fn test_resolve_db_dir_detects_rocksdb_in_user_subdir() {
    let dir = tempdir().unwrap();
    let user_dir = dir.path().join("user");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join("CURRENT"), b"MANIFEST-000001\n").unwrap();

    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, user_dir);
}

#[test]
fn test_resolve_db_dir_returns_error_for_empty_dir() {
    let dir = tempdir().unwrap();

    let err = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap_err();
    assert!(
        err.to_string().contains("No HelixDB database found"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_copy_dir_all_copies_flat_directory() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    fs::write(src.path().join("data.mdb"), b"hello world").unwrap();

    copy_dir_all(src.path(), dst.path()).unwrap();

    let copied = fs::read(dst.path().join("data.mdb")).unwrap();
    assert_eq!(copied, b"hello world");
}

#[test]
fn test_copy_dir_all_copies_nested_directories() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    let nested = src.path().join("sub");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("data.mdb"), b"nested data").unwrap();
    fs::write(src.path().join("CURRENT"), b"MANIFEST\n").unwrap();

    copy_dir_all(src.path(), dst.path()).unwrap();

    assert!(dst.path().join("CURRENT").exists());
    let nested_copy = fs::read(dst.path().join("sub").join("data.mdb")).unwrap();
    assert_eq!(nested_copy, b"nested data");
}

#[test]
fn test_copy_dir_all_returns_total_bytes() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    fs::write(src.path().join("file_a"), b"hello").unwrap(); // 5 bytes
    fs::write(src.path().join("file_b"), b"world!").unwrap(); // 6 bytes

    let total = copy_dir_all(src.path(), dst.path()).unwrap();
    assert_eq!(total, 11);
}
