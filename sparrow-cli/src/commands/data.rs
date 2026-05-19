//! Data management commands for SparrowDB (snapshot, clone, restore)

use crate::DataAction;
use crate::output::{Operation, Step};
use crate::project::ProjectContext;
use eyre::{Result, eyre};
use heed3::{CompactionOption, EnvFlags, EnvOpenOptions};
use std::fs;
use std::path::{Path, PathBuf};

/// Check whether a directory looks like a SparrowDB database directory.
fn has_db_marker(dir: &Path) -> bool {
    dir.join("data.mdb").exists() || dir.join("CURRENT").exists()
}

/// Resolve a source string to the actual database directory.
///
/// - If `source` has no path separators AND `project` is `Some`, check whether
///   `project.instance_volume(source)/user` contains DB files and return that.
/// - Otherwise treat `source` as a filesystem path:
///   - If the path itself has DB markers, return it.
///   - If `path/user` has DB markers, return `path/user`.
///   - Otherwise return an error.
pub fn resolve_db_dir(source: &str, project: Option<&ProjectContext>) -> Result<PathBuf> {
    let is_name = !source.contains('/') && !source.contains('\\');

    if is_name {
        if let Some(proj) = project {
            let user_dir = proj.instance_volume(source).join("user");
            if has_db_marker(&user_dir) {
                return Ok(user_dir);
            }
        }
    }

    // Treat source as a filesystem path
    let path = PathBuf::from(source);

    if has_db_marker(&path) {
        return Ok(path);
    }

    let user_subdir = path.join("user");
    if has_db_marker(&user_subdir) {
        return Ok(user_subdir);
    }

    Err(eyre!(
        "No SparrowDB database found at {}. Expected data.mdb (LMDB) or CURRENT (RocksDB).",
        path.display()
    ))
}

/// Recursively copies `src` into `dst`, creating `dst` if needed.
/// Does NOT clear pre-existing files in `dst` — callers that need
/// a clean destination must remove its contents before calling this.
/// Returns total bytes copied.
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<u64> {
    fs::create_dir_all(dst)?;
    let mut total_bytes: u64 = 0;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            total_bytes += copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            total_bytes += fs::copy(&entry.path(), &dest_path)?;
        }
    }

    Ok(total_bytes)
}

/// Create a snapshot of a SparrowDB database directory.
///
/// - For LMDB (`data.mdb`): uses heed3's built-in hot-copy via `env.copy_to_path`.
/// - For RocksDB (`CURRENT`): falls back to a filesystem copy (requires instance to be stopped).
/// - Otherwise: returns an error.
pub fn snapshot_impl(db_dir: &Path, output: &Path) -> Result<()> {
    if db_dir.join("data.mdb").exists() {
        fs::create_dir_all(output)?;
        let env = unsafe {
            EnvOpenOptions::new()
                .flags(EnvFlags::READ_ONLY)
                .max_dbs(200)
                .max_readers(200)
                .open(db_dir)?
        };
        env.copy_to_path(output.join("data.mdb"), CompactionOption::Disabled)?;
    } else if db_dir.join("CURRENT").exists() {
        fs::create_dir_all(output)?;
        crate::output::info(
            "RocksDB detected: filesystem copy. Ensure the instance is stopped for a consistent backup.",
        );
        copy_dir_all(db_dir, output)?;
    } else {
        return Err(eyre!(
            "No SparrowDB database found at {}.",
            db_dir.display()
        ));
    }

    Ok(())
}

/// Restore a SparrowDB backup to a destination directory.
///
/// - If `dest` is non-empty and `force` is false, returns an error.
/// - If `force` is true, removes all existing contents of `dest` before copying.
pub fn restore_impl(backup: &Path, dest: &Path, force: bool) -> Result<()> {
    if dest.exists() {
        let is_empty = fs::read_dir(dest)?.next().is_none();
        if !is_empty {
            // Guard against wiping the source: backup must not be equal to or inside dest
            let canonical_backup = fs::canonicalize(backup)
                .unwrap_or_else(|_| backup.to_path_buf());
            let canonical_dest = fs::canonicalize(dest)
                .unwrap_or_else(|_| dest.to_path_buf());
            if canonical_backup == canonical_dest || canonical_backup.starts_with(&canonical_dest) {
                return Err(eyre!(
                    "Backup path {} is inside or equal to destination {}. Refusing to restore.",
                    backup.display(),
                    dest.display()
                ));
            }
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

/// Copy an entire SparrowDB database directory to a new location.
///
/// Refuses to overwrite a non-empty destination.
pub fn clone_impl(db_dir: &Path, dest: &Path) -> Result<()> {
    // Refuse to overwrite a non-empty destination
    if dest.exists() {
        let is_empty = fs::read_dir(dest)?.next().is_none();
        if !is_empty {
            return Err(eyre!(
                "Destination {} is not empty. Use 'sparrow data restore --force' to overwrite.",
                dest.display()
            ));
        }
    }
    copy_dir_all(db_dir, dest)?;
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
        DataAction::Restore { backup, dest, force } => {
            let backup_path = PathBuf::from(&backup);

            if !backup_path.exists() {
                return Err(eyre!("Backup path {} does not exist.", backup_path.display()));
            }

            // Resolve destination: if it's a project instance name with existing volume, use that
            let project = ProjectContext::find_and_load(None).ok();
            let dest_path = if let Some(proj) = project.as_ref() {
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
    }
}
