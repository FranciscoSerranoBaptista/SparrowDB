use cliclack;
use crate::cleanup::CleanupTracker;
use crate::config::{BuildMode, DbConfig, LocalInstanceConfig, StorageBackend};
use crate::errors::project_error;
use crate::output::Operation;
use crate::project::ProjectContext;
use crate::prompts;
use crate::utils::print_instructions;
use eyre::Result;
use std::env;

pub async fn run(name: Option<String>) -> Result<()> {
    let mut cleanup_tracker = CleanupTracker::new();

    let cwd = env::current_dir()?;
    let mut project_context = ProjectContext::find_and_load(Some(&cwd))?;

    let instance_name = match name {
        Some(n) => n,
        None if prompts::is_interactive() => {
            prompts::intro(
                "sparrow add",
                Some("This will add a new local instance to the SparrowDB project.\n"),
            )?;
            let default = project_context.config.project.name.clone();
            let n: String = cliclack::input("Instance name")
                .placeholder(&default)
                .default_input(&default)
                .interact()?;
            n
        }
        None => {
            return Err(project_error(
                "No instance name specified. In non-interactive mode, specify one with 'sparrow add <name>'."
            ).into());
        }
    };

    if project_context.config.local.contains_key(&instance_name) {
        return Err(project_error(format!(
            "Instance '{instance_name}' already exists in sparrow.toml"
        ))
        .with_hint("use a different instance name or remove the existing instance")
        .into());
    }

    let op = Operation::new("Adding", &instance_name);

    let config_path = project_context.root.join("sparrow.toml");
    cleanup_tracker.backup_config(&project_context.config, config_path.clone());

    let local_config = LocalInstanceConfig {
        port: None,
        build_mode: BuildMode::Dev,
        storage_backend: StorageBackend::Lmdb,
        db_config: DbConfig::default(),
    };
    project_context.config.local.insert(instance_name.clone(), local_config);

    project_context.config.save_to_file(&config_path)?;

    op.success();

    print_instructions(
        "Next steps:",
        &[
            &format!("Run 'sparrow build {instance_name}' to compile your project for this instance"),
            &format!("Run 'sparrow push {instance_name}' to start the '{instance_name}' instance"),
        ],
    );

    Ok(())
}
