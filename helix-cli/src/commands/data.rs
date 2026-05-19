//! Data management commands for HelixDB (snapshot, clone, restore)

use crate::DataAction;
use crate::project::ProjectContext;
use eyre::{Result, eyre};
use std::fs;
use std::path::{Path, PathBuf};

/// Check whether a directory looks like a HelixDB database directory.
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
        "No HelixDB database found at {}. Expected data.mdb (LMDB) or CURRENT (RocksDB).",
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

pub async fn run(_action: DataAction) -> Result<()> {
    todo!("implemented in later tasks")
}
