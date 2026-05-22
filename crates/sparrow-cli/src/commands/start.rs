use crate::docker::DockerManager;
use crate::output::{Operation, Step, Verbosity};
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

    let op = Operation::new("Starting", &instance_name);

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

    let mut start_step = Step::with_messages("Starting container", "Container started");
    start_step.start();
    docker.start_instance(&instance_name)?;
    start_step.done();

    let instance_config = project.config.get_instance(&instance_name)?;
    let port = instance_config.port().unwrap_or(6969);

    op.success();

    let project_name = &project.config.project.name;
    if Verbosity::current().show_normal() {
        Operation::print_details(&[
            ("Local URL", &format!("http://localhost:{port}")),
            ("Container", &format!("sparrow_{project_name}_{instance_name}")),
            ("Data volume", &project.instance_volume(&instance_name).display().to_string()),
        ]);
    }

    Ok(())
}
