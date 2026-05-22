use crate::docker::DockerManager;
use crate::output::{Operation, Step};
use crate::project::ProjectContext;
use crate::utils::{print_confirm, print_lines, print_newline, print_warning};
use eyre::Result;

pub async fn run(instance_name: String) -> Result<()> {
    let project = ProjectContext::find_and_load(None)?;

    // Validate instance exists
    project.config.get_instance(&instance_name)?;

    print_warning(&format!(
        "This will permanently delete instance '{instance_name}' and ALL its data!"
    ));
    print_lines(&[
        "- Docker containers and images",
        "- Persistent volumes (databases, files)",
        "This action cannot be undone.",
    ]);
    print_newline();

    let confirmed = print_confirm(&format!(
        "Are you sure you want to delete instance '{instance_name}'?"
    ))?;

    if !confirmed {
        crate::output::info("Deletion cancelled.");
        return Ok(());
    }

    let op = Operation::new("Deleting", &instance_name);

    let runtime = project.config.project.container_runtime;
    if DockerManager::check_runtime_available(runtime).is_ok() {
        let mut docker_step =
            Step::with_messages("Removing Docker resources", "Docker resources removed");
        docker_step.start();
        let docker = DockerManager::new(&project);
        docker.prune_instance(&instance_name, true)?;
        docker.remove_instance_images(&instance_name)?;
        docker_step.done();
    }

    let workspace = project.instance_workspace(&instance_name);
    if workspace.exists() {
        std::fs::remove_dir_all(&workspace)?;
        Step::verbose_substep("Removed workspace directory");
    }

    let volume = project.instance_volume(&instance_name);
    if volume.exists() {
        std::fs::remove_dir_all(&volume)?;
        Step::verbose_substep("Removed persistent volumes");
    }

    op.success();
    Ok(())
}
