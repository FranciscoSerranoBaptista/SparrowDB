use crate::commands::build::MetricsData;
use crate::docker::DockerManager;
use crate::metrics_sender::MetricsSender;
use crate::output::{Operation, Step, Verbosity};
use crate::port;
use crate::project::ProjectContext;
use crate::prompts;
use eyre::Result;
use std::time::Instant;

pub async fn run(
    instance_name: Option<String>,
    metrics_sender: &MetricsSender,
) -> Result<()> {
    let start_time = Instant::now();

    // Load project context
    let project = ProjectContext::find_and_load(None)?;

    // Get instance name - prompt if not provided
    let instance_name = match instance_name {
        Some(name) => name,
        None if prompts::is_interactive() => {
            let instances = project.config.list_instances_with_types();
            prompts::intro(
                "sparrow push",
                Some(
                    "This will build and redeploy your selected instance based on the configuration in sparrow.toml.",
                ),
            )?;
            prompts::select_instance(&instances)?
        }
        None => {
            let instances = project.config.list_instances();
            return Err(eyre::eyre!(
                "No instance specified. Available instances: {}",
                instances
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };

    let deploy_result = push_local_instance(&project, &instance_name, metrics_sender).await;

    let duration = start_time.elapsed().as_secs() as u32;
    let success = deploy_result.is_ok();
    let error_messages = deploy_result.as_ref().err().map(|e| e.to_string());

    let default_metrics = MetricsData { queries_string: String::new(), num_of_queries: 0 };
    let metrics_data = deploy_result.as_ref().unwrap_or(&default_metrics);

    let docker = DockerManager::new(&project);
    let is_redeploy = docker.instance_exists(&instance_name).unwrap_or(false);
    if is_redeploy {
        metrics_sender.send_redeploy_local_event(
            instance_name.clone(),
            metrics_data.queries_string.clone(),
            metrics_data.num_of_queries,
            duration,
            success,
            error_messages,
        );
    } else {
        metrics_sender.send_deploy_local_event(
            instance_name.clone(),
            metrics_data.queries_string.clone(),
            metrics_data.num_of_queries,
            duration,
            success,
            error_messages,
        );
    }

    deploy_result.map(|_| ())
}

async fn push_local_instance(
    project: &ProjectContext,
    instance_name: &str,
    metrics_sender: &MetricsSender,
) -> Result<MetricsData> {
    let op = Operation::new("Deploying", instance_name);

    let docker = DockerManager::new(project);

    // Check Docker availability
    DockerManager::check_runtime_available(docker.runtime)?;

    // Check port availability before building
    let instance_config = project.config.get_instance(instance_name)?;
    let requested_port = instance_config.port().unwrap_or(port::DEFAULT_PORT);
    let (actual_port, port_changed) = port::ensure_port_available(requested_port)?;

    if port_changed {
        crate::output::warning(&format!(
            "Port {} is in use. Using port {} instead.",
            requested_port, actual_port
        ));
    }

    // Build the instance first (this ensures it's up to date) and get metrics data
    let metrics_data =
        crate::commands::build::run_build_steps(&op, project, instance_name, None, metrics_sender)
            .await?;

    // If port changed, regenerate docker-compose with new port
    if port_changed {
        let compose_content = docker.generate_docker_compose(
            instance_name,
            instance_config.clone(),
            Some(actual_port),
        )?;
        let compose_path = project.docker_compose_path(instance_name);
        std::fs::write(&compose_path, compose_content)?;
    }

    // Start the instance
    let mut start_step = Step::with_messages("Starting instance", "Instance started");
    start_step.start();
    docker.start_instance(instance_name)?;
    start_step.done();

    op.success();

    let project_name = &project.config.project.name;
    if Verbosity::current().show_normal() {
        Operation::print_details(&[
            ("Local URL", &format!("http://localhost:{actual_port}")),
            (
                "Container",
                &format!("sparrow-{project_name}-{instance_name}"),
            ),
            (
                "Data volume",
                &project.instance_volume(instance_name).display().to_string(),
            ),
        ]);
    }

    Ok(metrics_data)
}
