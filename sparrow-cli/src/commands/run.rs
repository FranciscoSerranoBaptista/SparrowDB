use eyre::{eyre, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::project::ProjectContext;

/// Find the sparrow-container binary in a `helix build --bin <dir>` output directory.
/// Prefers `<dir>/release/sparrow-container` over `<dir>/debug/sparrow-container`.
pub fn resolve_binary(bin_dir: &Path) -> Result<PathBuf> {
    for profile in ["release", "debug"] {
        let candidate = bin_dir.join(profile).join("sparrow-container");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(eyre!(
        "No binary found in {}\nRun 'sparrow build --bin {}' first",
        bin_dir.display(),
        bin_dir.display()
    ))
}

/// Resolve the data directory to pass as SPARROW_DATA_DIR.
/// Priority: explicit `--data-dir` flag > project instance volume path > ~/.sparrow/user
pub fn resolve_data_dir(
    data_dir_override: Option<String>,
    project: Option<&ProjectContext>,
    instance_name: Option<&str>,
) -> String {
    if let Some(dir) = data_dir_override {
        return dir;
    }
    if let (Some(proj), Some(inst)) = (project, instance_name) {
        return proj.instance_volume(inst).to_string_lossy().into_owned();
    }
    dirs::home_dir()
        .map(|h| h.join(".sparrow").to_string_lossy().into_owned())
        .unwrap_or_else(|| "/tmp/sparrow-data".to_string())
}

pub async fn run(
    bin: String,
    instance: Option<String>,
    data_dir: Option<String>,
    port: Option<u16>,
) -> Result<()> {
    let bin_dir = Path::new(&bin);
    let binary_path = resolve_binary(bin_dir)?;

    let project = ProjectContext::find_and_load(None).ok();
    let inst = instance.as_deref().unwrap_or("dev");

    let data_dir_val = resolve_data_dir(
        data_dir,
        project.as_ref(),
        project.as_ref().map(|_| inst),
    );

    let port_val = port
        .or_else(|| {
            project
                .as_ref()
                .and_then(|p| p.config.get_instance(inst).ok())
                .and_then(|c| c.port())
        })
        .unwrap_or(6969);

    println!("Starting sparrow-container:");
    println!("  binary:   {}", binary_path.display());
    println!("  data dir: {data_dir_val}");
    println!("  port:     {port_val}");

    std::fs::create_dir_all(&data_dir_val)?;

    let status = Command::new(&binary_path)
        .env("SPARROW_DATA_DIR", &data_dir_val)
        .env("SPARROW_PORT", port_val.to_string())
        .status()
        .await
        .map_err(|e| eyre!("Failed to start {}: {e}", binary_path.display()))?;

    if !status.success() {
        return Err(eyre!(
            "sparrow-container exited with code {:?}",
            status.code()
        ));
    }
    Ok(())
}
