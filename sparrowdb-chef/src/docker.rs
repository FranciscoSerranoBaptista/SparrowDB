use eyre::{Result, eyre};
use std::path::Path;
#[cfg(test)]
use std::process::Command;

/// Build a `docker compose` command scoped to the given project directory.
/// Test-only: returns `std::process::Command` so tests can inspect args without spawning.
#[cfg(test)]
pub fn compose_command(project_dir: &Path, args: &[&str]) -> Command {
    let compose_file = project_dir.join("docker-compose.yml");
    let mut cmd = Command::new("docker");
    cmd.arg("compose").arg("-f").arg(compose_file).args(args);
    cmd
}

/// Run `docker compose up -d` in the given project directory.
/// Uses `tokio::process::Command` so it does not block the async runtime.
pub async fn compose_up(project_dir: &Path) -> Result<()> {
    let compose_file = project_dir.join("docker-compose.yml");
    let status = tokio::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&compose_file)
        .args(["up", "-d", "--wait", "--wait-timeout", "300"])
        .status()
        .await
        .map_err(|e| eyre!("failed to run docker compose: {e}\nIs Docker running?"))?;

    if !status.success() {
        return Err(eyre!(
            "docker compose up failed with code {:?}",
            status.code()
        ));
    }
    Ok(())
}

/// Poll `check_health` until it returns Ok or `max_attempts` is exceeded.
pub async fn wait_for_healthy(
    client: &crate::http::SparrowClient,
    max_attempts: u32,
    delay_ms: u64,
) -> Result<()> {
    for attempt in 1..=max_attempts {
        if client.check_health().await.is_ok() {
            return Ok(());
        }
        if attempt < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    }
    Err(eyre!(
        "SparrowDB did not become healthy after {max_attempts} attempts.\n\
         Check `docker compose logs` in your project directory."
    ))
}
