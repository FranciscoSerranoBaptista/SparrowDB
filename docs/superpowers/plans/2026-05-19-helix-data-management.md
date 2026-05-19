# helix data — DB Data Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `helix data snapshot|clone|restore` subcommands that manage HelixDB data directories independent of project instances or Docker.

**Architecture:** A new `DataAction` enum in `lib.rs` (following the `DashboardAction` pattern) dispatches to `commands/data.rs`. A `resolve_db_dir` helper maps either a project instance name or an arbitrary filesystem path to the actual database directory (containing `data.mdb` for LMDB, or `CURRENT` for RocksDB). Snapshot uses `heed3::copy_to_path` for a hot-safe LMDB copy; clone and restore use a simple recursive `copy_dir_all`. RocksDB directories (detected by a `CURRENT` file) fall back to a filesystem copy with a warning.

**Tech Stack:** Rust, clap (Subcommand derive), heed3 0.22 (`copy_to_path`, `CompactionOption`, `EnvFlags`), std::fs (recursive copy), tempfile (tests), chrono (timestamp generation)

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `helix-cli/src/lib.rs` | Modify | Add `DataAction` enum (after `DashboardAction`) |
| `helix-cli/src/main.rs` | Modify | `pub use helix_cli::DataAction`, `Commands::Data`, match arm |
| `helix-cli/src/commands/mod.rs` | Modify | `pub mod data;` |
| `helix-cli/src/commands/data.rs` | **Create** | `resolve_db_dir`, `copy_dir_all`, `snapshot_impl`, `clone_impl`, `restore_impl`, `run(action)` |
| `helix-cli/src/tests/mod.rs` | Modify | `pub mod data_tests;` |
| `helix-cli/src/tests/data_tests.rs` | **Create** | Unit tests for all public functions |

---

## Task 1: Scaffolding + Shared Utilities (`resolve_db_dir`, `copy_dir_all`)

**Files:**
- Create: `helix-cli/src/commands/data.rs`
- Modify: `helix-cli/src/commands/mod.rs`
- Modify: `helix-cli/src/lib.rs`
- Modify: `helix-cli/src/main.rs`
- Modify: `helix-cli/src/tests/mod.rs`
- Create: `helix-cli/src/tests/data_tests.rs`

### Background

`resolve_db_dir(source: &str, project: Option<&ProjectContext>) -> Result<PathBuf>` maps a user-supplied string to the actual database directory:

1. If `source` has no path separator AND `project` is `Some` AND `project.instance_volume(source).join("user")` contains `data.mdb` or `CURRENT` → return that `user` subdirectory.
2. Otherwise treat `source` as a filesystem path:
   - If `path` itself contains `data.mdb` or `CURRENT` → return `path`.
   - If `path/user` contains `data.mdb` or `CURRENT` → return `path/user` (handles `HELIX_DATA_DIR`-style paths).
   - Otherwise → `Err("No HelixDB database found at <path>. Expected data.mdb (LMDB) or CURRENT (RocksDB).")`.

`copy_dir_all(src: &Path, dst: &Path) -> Result<u64>` recursively copies a directory tree, returning the total bytes copied.

- [ ] **Step 1: Write the failing tests**

Create `helix-cli/src/tests/data_tests.rs`:

```rust
use helix_cli::commands::data::{copy_dir_all, resolve_db_dir};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_resolve_db_dir_detects_lmdb_directly() {
    let dir = tempdir().unwrap();
    fs::File::create(dir.path().join("data.mdb")).unwrap();
    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path().to_path_buf());
}

#[test]
fn test_resolve_db_dir_detects_lmdb_in_user_subdir() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("user")).unwrap();
    fs::File::create(dir.path().join("user").join("data.mdb")).unwrap();
    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path().join("user"));
}

#[test]
fn test_resolve_db_dir_detects_rocksdb_directly() {
    let dir = tempdir().unwrap();
    fs::File::create(dir.path().join("CURRENT")).unwrap();
    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path().to_path_buf());
}

#[test]
fn test_resolve_db_dir_detects_rocksdb_in_user_subdir() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("user")).unwrap();
    fs::File::create(dir.path().join("user").join("CURRENT")).unwrap();
    let result = resolve_db_dir(dir.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result, dir.path().join("user"));
}

#[test]
fn test_resolve_db_dir_returns_error_for_empty_dir() {
    let dir = tempdir().unwrap();
    let result = resolve_db_dir(dir.path().to_str().unwrap(), None);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("No HelixDB database found"));
}

#[test]
fn test_copy_dir_all_copies_flat_directory() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    fs::write(src.path().join("data.mdb"), b"test data").unwrap();

    copy_dir_all(src.path(), dst.path()).unwrap();

    let result = fs::read(dst.path().join("data.mdb")).unwrap();
    assert_eq!(result, b"test data");
}

#[test]
fn test_copy_dir_all_copies_nested_directories() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    fs::create_dir(src.path().join("sub")).unwrap();
    fs::write(src.path().join("sub").join("file.txt"), b"nested").unwrap();
    fs::write(src.path().join("top.txt"), b"top").unwrap();

    copy_dir_all(src.path(), dst.path()).unwrap();

    assert_eq!(fs::read(dst.path().join("top.txt")).unwrap(), b"top");
    assert_eq!(
        fs::read(dst.path().join("sub").join("file.txt")).unwrap(),
        b"nested"
    );
}

#[test]
fn test_copy_dir_all_returns_total_bytes() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    fs::write(src.path().join("a.txt"), b"hello").unwrap(); // 5 bytes
    fs::write(src.path().join("b.txt"), b"world!").unwrap(); // 6 bytes

    let bytes = copy_dir_all(src.path(), dst.path()).unwrap();
    assert_eq!(bytes, 11);
}
```

- [ ] **Step 2: Add test module entry**

In `helix-cli/src/tests/mod.rs`, append:

```rust
#[cfg(test)]
pub mod data_tests;
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests 2>&1 | tail -20
```

Expected: compile error — `helix_cli::commands::data` does not exist.

- [ ] **Step 4: Create `commands/data.rs` with utilities**

Create `helix-cli/src/commands/data.rs`:

```rust
use crate::project::ProjectContext;
use eyre::{eyre, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::DataAction;

pub fn resolve_db_dir(source: &str, project: Option<&ProjectContext>) -> Result<PathBuf> {
    // Try as a project instance name (no path separator, project available)
    if !source.contains('/') && !source.contains('\\') {
        if let Some(proj) = project {
            let user_path = proj.instance_volume(source).join("user");
            if user_path.join("data.mdb").exists() || user_path.join("CURRENT").exists() {
                return Ok(user_path);
            }
        }
    }

    let path = PathBuf::from(source);

    // Direct: path itself is the DB dir
    if path.join("data.mdb").exists() || path.join("CURRENT").exists() {
        return Ok(path);
    }

    // Parent: path/user is the DB dir (HELIX_DATA_DIR semantics)
    let user_path = path.join("user");
    if user_path.join("data.mdb").exists() || user_path.join("CURRENT").exists() {
        return Ok(user_path);
    }

    Err(eyre!(
        "No HelixDB database found at {}. Expected data.mdb (LMDB) or CURRENT (RocksDB).",
        path.display()
    ))
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<u64> {
    fs::create_dir_all(dst)?;
    let mut total_bytes = 0u64;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            total_bytes += copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            total_bytes += fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(total_bytes)
}

pub async fn run(_action: DataAction) -> Result<()> {
    todo!("implemented in later tasks")
}
```

- [ ] **Step 5: Add `pub mod data;` to `commands/mod.rs`**

Append to `helix-cli/src/commands/mod.rs`:

```rust
pub mod data;
```

- [ ] **Step 6: Add `DataAction` to `lib.rs`**

In `helix-cli/src/lib.rs`, after the `DashboardAction` enum (after line 75), add:

```rust
#[derive(Subcommand)]
pub enum DataAction {
    /// Create a consistent snapshot of a database directory
    Snapshot {
        /// Source: project instance name (e.g. "dev") or filesystem path
        source: String,

        /// Output directory for the snapshot (default: ./backups/snapshot-<timestamp>/)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Copy a database directory to a new location
    Clone {
        /// Source: project instance name or filesystem path
        source: String,

        /// Destination directory path
        dest: String,
    },
    /// Restore a snapshot or clone into a destination directory
    Restore {
        /// Backup directory to restore from
        backup: String,

        /// Destination: project instance name or filesystem path
        dest: String,

        /// Overwrite destination even if it already contains data
        #[arg(long)]
        force: bool,
    },
}
```

- [ ] **Step 7: Wire `Data` into `main.rs`**

In `helix-cli/src/main.rs`, add `DataAction` to the `pub use` block at the top:

```rust
pub use helix_cli::{
    AuthAction, CloudDeploymentTypeCommand, ClusterConfigAction, ConfigAction, ConfigOutputFormat,
    DashboardAction, DataAction, MetricsAction, ProjectConfigAction, WorkspaceConfigAction,
};
```

Add the `Data` variant to the `Commands` enum (after `Backup`):

```rust
/// Manage database data directories (snapshot, clone, restore)
Data {
    #[command(subcommand)]
    action: DataAction,
},
```

Add the match arm in `main()` (after the `Backup` arm):

```rust
Commands::Data { action } => commands::data::run(action).await,
```

- [ ] **Step 8: Run tests to verify they pass**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests 2>&1 | tail -20
```

Expected: `test result: ok. 8 passed; 0 failed`

- [ ] **Step 9: Verify cargo check**

```bash
cargo check --manifest-path helix-cli/Cargo.toml 2>&1 | tail -10
```

Expected: warning about `todo!()` in `run`, no errors.

- [ ] **Step 10: Commit**

```bash
git add helix-cli/src/commands/data.rs helix-cli/src/commands/mod.rs helix-cli/src/lib.rs helix-cli/src/main.rs helix-cli/src/tests/mod.rs helix-cli/src/tests/data_tests.rs
git commit -m "feat(helix-cli): scaffold helix data subcommand with resolve_db_dir and copy_dir_all utilities"
```

---

## Task 2: `helix data snapshot`

**Files:**
- Modify: `helix-cli/src/commands/data.rs`
- Modify: `helix-cli/src/tests/data_tests.rs`

### Background

`snapshot` creates a consistent point-in-time copy of a database. For LMDB (`data.mdb` present), it uses `heed3`'s `copy_to_path` which writes a compact, consistent `data.mdb` without requiring the database to be stopped — LMDB's MVCC guarantees read consistency. For RocksDB (`CURRENT` present), MVCC isn't available from the CLI (rocksdb crate isn't a CLI dependency), so we print a warning and do a filesystem copy; the user must stop the instance first for a consistent RocksDB backup.

Default output path when `--output` is omitted: `./backups/snapshot-<YYYYMMDD-HHMMSS>/`

`heed3` imports needed:
```rust
use heed3::{CompactionOption, EnvFlags, EnvOpenOptions};
```

- [ ] **Step 1: Write failing tests**

Append to `helix-cli/src/tests/data_tests.rs`:

```rust
use helix_cli::commands::data::snapshot_impl;

#[test]
fn test_snapshot_lmdb_creates_data_mdb_in_output() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    // Create a real LMDB environment so data.mdb exists
    let _env = unsafe {
        heed3::EnvOpenOptions::new()
            .map_size(1 * 1024 * 1024)
            .max_dbs(10)
            .open(src.path())
            .unwrap()
    };
    drop(_env);

    snapshot_impl(src.path(), dst.path()).unwrap();

    assert!(dst.path().join("data.mdb").exists());
}

#[test]
fn test_snapshot_rocksdb_copies_directory() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    // Simulate RocksDB layout
    fs::File::create(src.path().join("CURRENT")).unwrap();
    fs::write(src.path().join("000010.sst"), b"sst data").unwrap();
    fs::write(src.path().join("MANIFEST-000001"), b"manifest").unwrap();

    snapshot_impl(src.path(), dst.path()).unwrap();

    assert!(dst.path().join("CURRENT").exists());
    assert!(dst.path().join("000010.sst").exists());
    assert_eq!(
        fs::read(dst.path().join("000010.sst")).unwrap(),
        b"sst data"
    );
}

#[test]
fn test_snapshot_errors_if_source_is_not_a_db() {
    let src = tempdir().unwrap(); // empty, no data.mdb or CURRENT
    let dst = tempdir().unwrap();
    let result = snapshot_impl(src.path(), dst.path());
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests::test_snapshot 2>&1 | tail -10
```

Expected: compile error — `snapshot_impl` does not exist.

- [ ] **Step 3: Implement `snapshot_impl` and update `run`**

Replace the contents of `helix-cli/src/commands/data.rs` with:

```rust
use crate::project::ProjectContext;
use crate::output::{Operation, Step};
use crate::DataAction;
use eyre::{eyre, Result};
use heed3::{CompactionOption, EnvFlags, EnvOpenOptions};
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_db_dir(source: &str, project: Option<&ProjectContext>) -> Result<PathBuf> {
    if !source.contains('/') && !source.contains('\\') {
        if let Some(proj) = project {
            let user_path = proj.instance_volume(source).join("user");
            if user_path.join("data.mdb").exists() || user_path.join("CURRENT").exists() {
                return Ok(user_path);
            }
        }
    }

    let path = PathBuf::from(source);

    if path.join("data.mdb").exists() || path.join("CURRENT").exists() {
        return Ok(path);
    }

    let user_path = path.join("user");
    if user_path.join("data.mdb").exists() || user_path.join("CURRENT").exists() {
        return Ok(user_path);
    }

    Err(eyre!(
        "No HelixDB database found at {}. Expected data.mdb (LMDB) or CURRENT (RocksDB).",
        path.display()
    ))
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<u64> {
    fs::create_dir_all(dst)?;
    let mut total_bytes = 0u64;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            total_bytes += copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            total_bytes += fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(total_bytes)
}

pub fn snapshot_impl(db_dir: &Path, output: &Path) -> Result<()> {
    fs::create_dir_all(output)?;

    if db_dir.join("data.mdb").exists() {
        // LMDB: hot-safe copy via heed3
        let env = unsafe {
            EnvOpenOptions::new()
                .flags(EnvFlags::READ_ONLY)
                .max_dbs(200)
                .max_readers(200)
                .open(db_dir)?
        };
        env.copy_to_path(output.join("data.mdb"), CompactionOption::Disabled)?;
    } else if db_dir.join("CURRENT").exists() {
        // RocksDB: filesystem copy (requires stopped instance for consistency)
        crate::output::info(
            "RocksDB detected: filesystem copy. Ensure the instance is stopped for a consistent backup.",
        );
        copy_dir_all(db_dir, output)?;
    } else {
        return Err(eyre!(
            "No HelixDB database found at {}.",
            db_dir.display()
        ));
    }

    Ok(())
}

pub async fn run(action: DataAction) -> Result<()> {
    match action {
        DataAction::Snapshot { source, output } => {
            let project = ProjectContext::find_and_load(None).ok();
            let db_dir = resolve_db_dir(&source, project.as_ref())?;

            let output_dir = match output {
                Some(path) => PathBuf::from(path),
                None => {
                    let ts = chrono::Local::now()
                        .format("snapshot-%Y%m%d-%H%M%S")
                        .to_string();
                    PathBuf::from("backups").join(ts)
                }
            };

            let op = Operation::new("Snapshotting", &source);
            let mut step = Step::with_messages("Copying database", "Database copied");
            step.start();

            snapshot_impl(&db_dir, &output_dir)?;

            step.done();
            op.success();

            if crate::output::Verbosity::current().show_normal() {
                Operation::print_details(&[("Snapshot location", &output_dir.display().to_string())]);
            }

            Ok(())
        }

        DataAction::Clone { source, dest } => {
            todo!("implemented in Task 3")
        }

        DataAction::Restore { backup, dest, force } => {
            todo!("implemented in Task 4")
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests 2>&1 | tail -20
```

Expected: all data_tests pass (the todo arms don't run during these tests).

- [ ] **Step 5: Commit**

```bash
git add helix-cli/src/commands/data.rs helix-cli/src/tests/data_tests.rs
git commit -m "feat(helix-cli): implement helix data snapshot with LMDB hot-copy and RocksDB fallback"
```

---

## Task 3: `helix data clone`

**Files:**
- Modify: `helix-cli/src/commands/data.rs`
- Modify: `helix-cli/src/tests/data_tests.rs`

### Background

`clone` creates a full filesystem copy of a database directory that can be passed directly to `helix run --data-dir`. Unlike `snapshot`, it copies the entire tree (not just `data.mdb`) and always uses the filesystem-level copy, making the result a drop-in replacement of the original. Both LMDB and RocksDB work the same way.

`dest` is created if it doesn't exist. If it already exists and is non-empty, the command errors — use `restore --force` to overwrite.

- [ ] **Step 1: Write failing tests**

Append to `helix-cli/src/tests/data_tests.rs`:

```rust
use helix_cli::commands::data::clone_impl;

#[test]
fn test_clone_copies_entire_lmdb_directory() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    // Fake LMDB layout
    fs::write(src.path().join("data.mdb"), b"lmdb data").unwrap();
    fs::write(src.path().join("lock.mdb"), b"lock").unwrap();

    let dest_path = dst.path().join("cloned");
    clone_impl(src.path(), &dest_path).unwrap();

    assert_eq!(fs::read(dest_path.join("data.mdb")).unwrap(), b"lmdb data");
    assert_eq!(fs::read(dest_path.join("lock.mdb")).unwrap(), b"lock");
}

#[test]
fn test_clone_copies_rocksdb_directory() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    fs::File::create(src.path().join("CURRENT")).unwrap();
    fs::write(src.path().join("000001.sst"), b"sst").unwrap();

    let dest_path = dst.path().join("cloned");
    clone_impl(src.path(), &dest_path).unwrap();

    assert!(dest_path.join("CURRENT").exists());
    assert_eq!(fs::read(dest_path.join("000001.sst")).unwrap(), b"sst");
}

#[test]
fn test_clone_errors_if_dest_is_non_empty() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    fs::File::create(src.path().join("data.mdb")).unwrap();
    fs::File::create(dst.path().join("existing.txt")).unwrap(); // dest already has content

    let result = clone_impl(src.path(), dst.path());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not empty"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests::test_clone 2>&1 | tail -10
```

Expected: compile error — `clone_impl` does not exist.

- [ ] **Step 3: Implement `clone_impl` and the `Clone` arm in `run`**

Add `clone_impl` to `helix-cli/src/commands/data.rs` (after `snapshot_impl`):

```rust
pub fn clone_impl(db_dir: &Path, dest: &Path) -> Result<()> {
    // Refuse to overwrite a non-empty destination
    if dest.exists() {
        let is_empty = fs::read_dir(dest)?.next().is_none();
        if !is_empty {
            return Err(eyre!(
                "Destination {} is not empty. Use 'helix data restore --force' to overwrite.",
                dest.display()
            ));
        }
    }

    copy_dir_all(db_dir, dest)?;
    Ok(())
}
```

Replace the `DataAction::Clone` arm in `run`:

```rust
DataAction::Clone { source, dest } => {
    let project = ProjectContext::find_and_load(None).ok();
    let db_dir = resolve_db_dir(&source, project.as_ref())?;
    let dest_path = PathBuf::from(&dest);

    let op = Operation::new("Cloning", &source);
    let mut step = Step::with_messages("Copying database", "Database cloned");
    step.start();

    clone_impl(&db_dir, &dest_path)?;

    step.done();
    op.success();

    if crate::output::Verbosity::current().show_normal() {
        Operation::print_details(&[
            ("Source",      &db_dir.display().to_string()),
            ("Destination", &dest_path.display().to_string()),
        ]);
    }

    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests 2>&1 | tail -20
```

Expected: all data_tests pass.

- [ ] **Step 5: Commit**

```bash
git add helix-cli/src/commands/data.rs helix-cli/src/tests/data_tests.rs
git commit -m "feat(helix-cli): implement helix data clone for full directory copy"
```

---

## Task 4: `helix data restore`

**Files:**
- Modify: `helix-cli/src/commands/data.rs`
- Modify: `helix-cli/src/tests/data_tests.rs`

### Background

`restore` copies a backup/snapshot directory into a destination database directory. The destination can be a project instance name or a filesystem path. Unlike `clone`, restore is designed to _overwrite_ existing data — it requires `--force` if the destination already contains files, and always prints a warning to stop the running instance first.

`restore_impl(backup: &Path, dest: &Path, force: bool)`:
- If `dest` exists and is non-empty AND `force` is false → error with "use --force to overwrite".
- If `dest` is non-empty AND `force` is true → remove all existing files in `dest`, then copy.
- Then `copy_dir_all(backup, dest)`.

Note: unlike `clone`, `restore` maps `dest` through `resolve_db_dir` **only if** the instance already has data. If restoring to a fresh path (no `data.mdb`/`CURRENT` yet), we use the path as-is (this is how you restore into a new empty instance).

- [ ] **Step 1: Write failing tests**

Append to `helix-cli/src/tests/data_tests.rs`:

```rust
use helix_cli::commands::data::restore_impl;

#[test]
fn test_restore_copies_backup_to_empty_dest() {
    let backup = tempdir().unwrap();
    let dest = tempdir().unwrap();

    fs::write(backup.path().join("data.mdb"), b"restored data").unwrap();

    restore_impl(backup.path(), dest.path(), false).unwrap();

    assert_eq!(
        fs::read(dest.path().join("data.mdb")).unwrap(),
        b"restored data"
    );
}

#[test]
fn test_restore_errors_on_non_empty_dest_without_force() {
    let backup = tempdir().unwrap();
    let dest = tempdir().unwrap();

    fs::write(backup.path().join("data.mdb"), b"new data").unwrap();
    fs::write(dest.path().join("data.mdb"), b"existing data").unwrap();

    let result = restore_impl(backup.path(), dest.path(), false);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("--force"));
}

#[test]
fn test_restore_overwrites_with_force() {
    let backup = tempdir().unwrap();
    let dest = tempdir().unwrap();

    fs::write(backup.path().join("data.mdb"), b"new data").unwrap();
    fs::write(dest.path().join("data.mdb"), b"old data").unwrap();
    fs::write(dest.path().join("stale.txt"), b"stale").unwrap();

    restore_impl(backup.path(), dest.path(), true).unwrap();

    assert_eq!(fs::read(dest.path().join("data.mdb")).unwrap(), b"new data");
    // stale file should be gone after forced overwrite
    assert!(!dest.path().join("stale.txt").exists());
}

#[test]
fn test_restore_preserves_nested_structure() {
    let backup = tempdir().unwrap();
    let dest = tempdir().unwrap();

    fs::create_dir(backup.path().join("sub")).unwrap();
    fs::write(backup.path().join("CURRENT"), b"current").unwrap();
    fs::write(backup.path().join("sub").join("sst.sst"), b"sst").unwrap();

    restore_impl(backup.path(), dest.path(), false).unwrap();

    assert!(dest.path().join("CURRENT").exists());
    assert!(dest.path().join("sub").join("sst.sst").exists());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests::test_restore 2>&1 | tail -10
```

Expected: compile error — `restore_impl` does not exist.

- [ ] **Step 3: Implement `restore_impl` and the `Restore` arm in `run`**

Add `restore_impl` to `helix-cli/src/commands/data.rs` (after `clone_impl`):

```rust
pub fn restore_impl(backup: &Path, dest: &Path, force: bool) -> Result<()> {
    if dest.exists() {
        let is_empty = fs::read_dir(dest)?.next().is_none();
        if !is_empty {
            if !force {
                return Err(eyre!(
                    "Destination {} already contains data. Use --force to overwrite.",
                    dest.display()
                ));
            }
            // Remove existing contents before restoring
            for entry in fs::read_dir(dest)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
            }
        }
    }

    copy_dir_all(backup, dest)?;
    Ok(())
}
```

Replace the `DataAction::Restore` arm in `run`:

```rust
DataAction::Restore { backup, dest, force } => {
    let backup_path = PathBuf::from(&backup);

    if !backup_path.exists() {
        return Err(eyre!("Backup path {} does not exist.", backup_path.display()));
    }

    // Resolve destination: try as project instance first, then raw path
    let project = ProjectContext::find_and_load(None).ok();
    let dest_path = if let Some(proj) = project.as_ref() {
        // If it's an instance name with an existing volume, use that
        let candidate = proj.instance_volume(&dest).join("user");
        if candidate.exists() {
            candidate
        } else {
            PathBuf::from(&dest)
        }
    } else {
        PathBuf::from(&dest)
    };

    crate::output::info(
        "Warning: ensure the destination instance is stopped before restoring.",
    );

    let op = Operation::new("Restoring", &dest);
    let mut step = Step::with_messages("Restoring database", "Database restored");
    step.start();

    restore_impl(&backup_path, &dest_path, force)?;

    step.done();
    op.success();

    if crate::output::Verbosity::current().show_normal() {
        Operation::print_details(&[
            ("Backup",      &backup_path.display().to_string()),
            ("Destination", &dest_path.display().to_string()),
        ]);
    }

    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --manifest-path helix-cli/Cargo.toml data_tests 2>&1 | tail -20
```

Expected: all data_tests pass.

- [ ] **Step 5: Run the full test suite to check for regressions**

```bash
cargo test --manifest-path helix-cli/Cargo.toml 2>&1 | tail -20
```

Expected: 1 pre-existing failure (`test_init_preserves_existing_scaffold_files_non_interactive`), no new failures.

- [ ] **Step 6: Commit**

```bash
git add helix-cli/src/commands/data.rs helix-cli/src/tests/data_tests.rs
git commit -m "feat(helix-cli): implement helix data restore with --force overwrite support"
```

---

## Self-Review

**Spec coverage:**
- `helix data snapshot <source> [--output]` → Task 2 ✓
- `helix data clone <source> <dest>` → Task 3 ✓
- `helix data restore <backup> <dest> [--force]` → Task 4 ✓
- `resolve_db_dir` handles instance names and filesystem paths → Task 1 ✓
- LMDB hot-safe copy via heed3 → Task 2 ✓
- RocksDB fallback with warning → Task 2 ✓

**Placeholder scan:** None found.

**Type consistency:**
- `snapshot_impl(&Path, &Path) -> Result<()>` — used in Task 2 tests and run arm ✓
- `clone_impl(&Path, &Path) -> Result<()>` — used in Task 3 tests and run arm ✓
- `restore_impl(&Path, &Path, bool) -> Result<()>` — used in Task 4 tests and run arm ✓
- `resolve_db_dir(&str, Option<&ProjectContext>) -> Result<PathBuf>` — consistent across all tasks ✓
- `DataAction::Snapshot { source: String, output: Option<String> }` — matched correctly in run ✓
- `DataAction::Clone { source: String, dest: String }` — matched correctly ✓
- `DataAction::Restore { backup: String, dest: String, force: bool }` — matched correctly ✓

**Note on `crate::output::info`:** This function must exist. Checking existing usage in backup.rs — it uses `crate::output::info(...)`. Confirm this function is public in `output.rs` before Task 2. If it's not exported, replace with `eprintln!`.
