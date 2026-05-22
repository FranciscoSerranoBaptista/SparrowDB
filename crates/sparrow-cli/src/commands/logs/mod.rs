mod cli;
mod log_source;
mod tui;

use crate::project::ProjectContext;
use crate::prompts;
use eyre::{Result, eyre};
use log_source::LogSource;

pub async fn run(
    instance: Option<String>,
    live: bool,
    range: bool,
    start: Option<String>,
    end: Option<String>,
) -> Result<()> {
    let project = ProjectContext::find_and_load(None)?;

    let instance_name = match instance {
        Some(name) => name,
        None if prompts::is_interactive() => {
            let instances = project.config.list_instances_with_types();
            prompts::intro("sparrow logs", Some("View logs for your instance\n"))?;
            prompts::select_instance(&instances)?
        }
        None => {
            let instances = project.config.list_instances();
            return Err(eyre!(
                "No instance specified. Available instances: {}",
                instances.into_iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    };

    let log_source = LogSource::from_instance(&project, &instance_name)?;

    if live {
        cli::stream_live(&log_source).await
    } else if range {
        cli::query_range(&log_source, start, end).await
    } else {
        tui::run(log_source, instance_name).await
    }
}
