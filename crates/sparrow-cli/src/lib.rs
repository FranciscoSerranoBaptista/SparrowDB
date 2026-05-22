// Library interface for sparrow-cli to enable testing
use clap::Subcommand;

pub mod cleanup;
pub mod commands;
pub mod config;
pub mod docker;
pub mod errors;
pub mod metrics_sender;
pub mod output;
pub mod port;
pub mod project;
pub mod prompts;
pub mod update;
pub mod utils;

#[derive(Subcommand)]
pub enum MetricsAction {
    /// Enable metrics collection
    Full,
    /// Disable metrics collection
    Basic,
    /// Disable metrics collection
    Off,
    /// Show metrics status
    Status,
}

#[derive(Subcommand)]
pub enum DataAction {
    /// Create a consistent snapshot of a database directory
    Snapshot {
        /// Source: project instance name (e.g. "dev") or filesystem path
        source: String,
        /// Output directory for the snapshot (default: ./backups/snapshot-<timestamp>/)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Copy a database directory to a new location
    Clone {
        /// Source: project instance name or filesystem path
        source: String,
        /// Destination directory path
        dest: String,
    },
    /// Restore a snapshot or clone into a destination directory
    Restore {
        /// Backup directory to restore from
        backup: String,
        /// Destination: project instance name or filesystem path
        dest: String,
        /// Overwrite destination even if it already contains data
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum ProjectConfigAction {
    /// List projects in the selected workspace
    List {
        /// Workspace slug by default, or workspace ID with --id
        workspace: Option<String>,

        /// Treat the workspace selector as a workspace ID instead of a slug
        #[arg(long)]
        id: bool,
    },
    /// Show the project linked in sparrow.toml
    Show,
    /// Switch the project linked in sparrow.toml
    Switch {
        /// Project name by default, or project ID with --id
        project: Option<String>,

        /// Treat the selector as a project ID instead of a project name
        #[arg(long)]
        id: bool,
    },
}
