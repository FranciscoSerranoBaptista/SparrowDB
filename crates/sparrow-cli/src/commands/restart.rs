use crate::docker::DockerManager;
use crate::output::{Operation, Step};
use crate::project::ProjectContext;
use crate::prompts;
use eyre::Result;

pub async fn run(instance_name: Option<String>) -> Result<()> {
    let project = ProjectContext::find_and_load(None)?;

    let instance_name = match instance_name {
        Some(name) => name,
        None if prompts::is_interactive() => {
            let instances = project.config.list_instances_with_types();
            prompts::select_instance(&instances)?
        }
        None => {
            let instances = project.config.list_instances();
            return Err(eyre::eyre!(
                "No instance specified. Available instances: {}",
                instances.into_iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    };

    // Validate instance exists in config before touching Docker
    project.config.get_instance(&instance_name)?;

    let op = Operation::new("Restarting", &instance_name);

    let docker = DockerManager::new(&project);
    DockerManager::check_runtime_available(docker.runtime)?;

    let workspace = project.instance_workspace(&instance_name);
    if !workspace.join("docker-compose.yml").exists() {
        op.failure();
        let error = crate::errors::CliError::new(format!(
            "instance '{instance_name}' has not been built yet"
        ))
        .with_hint(format!(
            "run 'sparrow build {instance_name}' first to build the instance"
        ));
        return Err(eyre::eyre!("{}", error.render()));
    }

    let mut restart_step = Step::with_messages("Restarting container", "Container restarted");
    restart_step.start();
    docker.restart_instance(&instance_name)?;
    restart_step.done();

    op.success();
    Ok(())
}
