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

    let op = Operation::new("Stopping", &instance_name);

    let docker = DockerManager::new(&project);
    DockerManager::check_runtime_available(docker.runtime)?;

    let mut stop_step = Step::with_messages("Stopping container", "Container stopped");
    stop_step.start();
    docker.stop_instance(&instance_name)?;
    stop_step.done();

    op.success();
    Ok(())
}
