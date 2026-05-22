use crate::config::ContainerRuntime;
use crate::docker::DockerManager;
use crate::project::ProjectContext;
use chrono::{DateTime, Utc};
use eyre::{Result, eyre};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

#[derive(Clone)]
pub enum LogSource {
    Local {
        container_name: String,
        runtime: ContainerRuntime,
    },
}

impl LogSource {
    pub fn from_instance(project: &ProjectContext, instance_name: &str) -> Result<Self> {
        project.config.get_instance(instance_name)?;
        let docker = DockerManager::new(project);
        let project_name = format!("sparrow-{}-{}", project.config.project.name, instance_name);
        let container_name = format!("{project_name}_app");
        Ok(LogSource::Local { container_name, runtime: docker.runtime })
    }

    pub async fn stream_live<F>(&self, mut on_line: F) -> Result<()>
    where
        F: FnMut(String),
    {
        match self {
            LogSource::Local { container_name, runtime } => {
                stream_local_logs(container_name, runtime, &mut on_line)
            }
        }
    }

    pub async fn query_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<String>> {
        match self {
            LogSource::Local { container_name, runtime } => {
                query_local_logs(container_name, runtime, start, end)
            }
        }
    }
}

fn stream_local_logs<F>(container_name: &str, runtime: &ContainerRuntime, on_line: &mut F) -> Result<()>
where
    F: FnMut(String),
{
    let mut child = Command::new(runtime.binary())
        .args(["logs", "-f", "--tail", "100", container_name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| eyre!("Failed to spawn {} logs command: {}", runtime.binary(), e))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stderr_handle = stderr.map(|s| {
        let reader = BufReader::new(s);
        std::thread::spawn(move || reader.lines().map_while(Result::ok).collect::<Vec<_>>())
    });

    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => on_line(line),
                Err(e) => return Err(eyre!("Error reading log line: {}", e)),
            }
        }
    }

    if let Some(handle) = stderr_handle
        && let Ok(lines) = handle.join()
    {
        for line in lines {
            on_line(line);
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(eyre!("Docker logs command failed with status: {}", status));
    }
    Ok(())
}

fn query_local_logs(
    container_name: &str,
    runtime: &ContainerRuntime,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<String>> {
    let since = start.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let until = end.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let output = Command::new(runtime.binary())
        .args(["logs", "--since", &since, "--until", &until, container_name])
        .output()
        .map_err(|e| eyre!("Failed to run {} logs command: {}", runtime.binary(), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!("Docker logs failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut lines: Vec<String> = stdout.lines().map(String::from).collect();
    lines.extend(stderr.lines().map(String::from));
    Ok(lines)
}
