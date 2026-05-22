use crate::{output, project::ProjectContext};
use eyre::Result;

/// Entry point for `sparrow migrate <subcommand>`.
pub async fn run(subcommand: MigrateSubcommand) -> Result<()> {
    match subcommand {
        MigrateSubcommand::Status { instance } => status(instance).await,
        MigrateSubcommand::Apply { instance } => apply(instance).await,
        MigrateSubcommand::List { instance } => list(instance).await,
    }
}

#[derive(Debug, clap::Subcommand)]
pub enum MigrateSubcommand {
    /// Show the status of all registered schema migrations.
    Status {
        /// Instance name (uses the only configured instance if omitted).
        instance: Option<String>,
    },
    /// Apply pending migrations by restarting the instance so they run on startup.
    Apply {
        /// Instance name (uses the only configured instance if omitted).
        instance: Option<String>,
    },
    /// List all schema migrations compiled into the running binary.
    List {
        /// Instance name (uses the only configured instance if omitted).
        instance: Option<String>,
    },
}

async fn status(instance: Option<String>) -> Result<()> {
    let project = ProjectContext::find_and_load(None)
        .map_err(|e| eyre::eyre!("{e}"))?;
    let instance_name = resolve_instance(&project, instance)?;
    let url = instance_url(&project, &instance_name)?;

    let body = reqwest::get(format!("{url}/migrate_status"))
        .await
        .map_err(|e| eyre::eyre!("Failed to reach instance '{instance_name}': {e}"))?
        .text()
        .await?;

    output::info("Migration status:");
    println!("{}", pretty_print_json(&body));
    Ok(())
}

async fn apply(instance: Option<String>) -> Result<()> {
    output::info("Migrations run automatically on startup.");
    output::info("Restarting the instance to apply pending migrations...");

    let project = ProjectContext::find_and_load(None)
        .map_err(|e| eyre::eyre!("{e}"))?;
    let instance_name = resolve_instance(&project, instance)?;

    crate::commands::stop::run(Some(instance_name.clone())).await?;
    crate::commands::start::run(Some(instance_name)).await?;

    output::success("Instance restarted. Any pending migrations have been applied.");
    Ok(())
}

async fn list(instance: Option<String>) -> Result<()> {
    let project = ProjectContext::find_and_load(None)
        .map_err(|e| eyre::eyre!("{e}"))?;
    let instance_name = resolve_instance(&project, instance)?;
    let url = instance_url(&project, &instance_name)?;

    let body = reqwest::get(format!("{url}/migrate_list"))
        .await
        .map_err(|e| eyre::eyre!("Failed to reach instance '{instance_name}': {e}"))?
        .text()
        .await?;

    output::info("Compiled migrations:");
    println!("{}", pretty_print_json(&body));
    Ok(())
}

fn resolve_instance(project: &ProjectContext, instance: Option<String>) -> Result<String> {
    match instance {
        Some(name) => Ok(name),
        None => {
            let instances: Vec<_> = project.config.local.keys().cloned().collect();
            match instances.as_slice() {
                [single] => Ok(single.clone()),
                [] => Err(eyre::eyre!("No instances configured. Run 'sparrow init' first.")),
                _ => Err(eyre::eyre!(
                    "Multiple instances configured. Specify one with 'sparrow migrate <subcommand> <instance>'."
                )),
            }
        }
    }
}

fn instance_url(project: &ProjectContext, instance_name: &str) -> Result<String> {
    let instance = project.config.get_instance(instance_name)
        .map_err(|e| eyre::eyre!("{e}"))?;
    let port = instance.port().unwrap_or(6969);
    Ok(format!("http://localhost:{port}"))
}

fn pretty_print_json(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| body.to_string())
}
