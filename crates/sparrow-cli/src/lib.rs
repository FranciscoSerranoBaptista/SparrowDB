// Library interface for sparrow-cli to enable testing
use clap::{Subcommand, ValueEnum};

pub mod cleanup;
pub mod commands;
pub mod config;
pub mod docker;
pub mod errors;
pub mod github_issue;
pub mod metrics_sender;
pub mod output;
pub mod port;
pub mod project;
pub mod prompts;
pub mod sse_client;
pub mod update;
pub mod utils;

#[derive(Subcommand)]
pub enum AuthAction {
    /// Login to Sparrow cloud
    Login,
    /// Logout from Sparrow cloud
    Logout,
    /// Rotate a cluster API key
    CreateKey {
        /// Cluster ID
        cluster: String,
    },
}

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
pub enum DashboardAction {
    /// Start the dashboard
    Start {
        /// Instance to connect to (from sparrow.toml)
        instance: Option<String>,

        /// Port to run dashboard on
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Sparrow host to connect to (e.g., localhost). Bypasses project config.
        #[arg(long)]
        host: Option<String>,

        /// Sparrow port to connect to. Used with --host.
        #[arg(long, default_value = "6969")]
        sparrow_port: u16,

        /// Run dashboard in foreground with logs
        #[arg(long)]
        attach: bool,

        /// Restart if dashboard is already running
        #[arg(long)]
        restart: bool,
    },
    /// Stop the dashboard
    Stop,
    /// Show dashboard status
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

#[derive(Subcommand, Clone)]
pub enum CloudDeploymentTypeCommand {
    /// Initialize SparrowDB Cloud deployment
    #[command(name = "cloud")]
    SparrowCloud {
        /// Region for SparrowDB cloud instance (default: us-east-1)
        #[arg(long, default_value = "us-east-1")]
        region: Option<String>,

        /// Instance name
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Initialize ECR deployment
    Ecr {
        /// Instance name
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Initialize Fly.io deployment
    Fly {
        /// Authentication type
        #[arg(long, default_value = "cli")]
        auth: String,

        /// volume size
        #[arg(long, default_value = "20")]
        volume_size: u16,

        /// vm size
        #[arg(long, default_value = "shared-cpu-4x")]
        vm_size: String,

        /// privacy
        #[arg(long, default_value = "false")]
        private: bool,

        /// Instance name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Initialize Local deployment
    Local {
        /// Instance name
        #[arg(short, long)]
        name: Option<String>,
    },
}

impl CloudDeploymentTypeCommand {
    pub fn name(&self) -> Option<String> {
        match self {
            CloudDeploymentTypeCommand::SparrowCloud { name, .. } => name.clone(),
            CloudDeploymentTypeCommand::Ecr { name } => name.clone(),
            CloudDeploymentTypeCommand::Fly { name, .. } => name.clone(),
            CloudDeploymentTypeCommand::Local { name } => name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigOutputFormat {
    Human,
    Json,
}

impl Default for ConfigOutputFormat {
    fn default() -> Self {
        Self::Human
    }
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Manage the active workspace selection
    Workspace {
        #[command(subcommand)]
        action: WorkspaceConfigAction,
    },
    /// Manage the project linked in sparrow.toml
    Project {
        #[command(subcommand)]
        action: ProjectConfigAction,
    },
    /// List local and remote clusters for a workspace or project
    Cluster {
        #[command(subcommand)]
        action: ClusterConfigAction,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceConfigAction {
    /// List accessible workspaces
    List {
        /// Include workspace members
        #[arg(long)]
        members: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Human)]
        format: ConfigOutputFormat,
    },
    /// Show the currently selected workspace
    Show {
        /// Output format
        #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Human)]
        format: ConfigOutputFormat,
    },
    /// Switch the active workspace
    Switch {
        /// Workspace slug by default, or workspace ID with --id
        workspace: Option<String>,

        /// Treat the selector as a workspace ID instead of a slug
        #[arg(long)]
        id: bool,
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

        /// Output format
        #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Human)]
        format: ConfigOutputFormat,
    },
    /// Show the project linked in sparrow.toml
    Show {
        /// Output format
        #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Human)]
        format: ConfigOutputFormat,
    },
    /// Switch the project linked in sparrow.toml
    Switch {
        /// Project name by default, or project ID with --id
        project: Option<String>,

        /// Treat the selector as a project ID instead of a project name
        #[arg(long)]
        id: bool,
    },
}

#[derive(Subcommand)]
pub enum ClusterConfigAction {
    /// List local instances plus live workspace/project clusters
    List {
        /// Workspace slug to inspect
        #[arg(long, conflicts_with = "workspace_id")]
        workspace: Option<String>,

        /// Workspace ID to inspect
        #[arg(long = "workspace-id", conflicts_with = "workspace")]
        workspace_id: Option<String>,

        /// Project name to narrow the remote results within the selected workspace
        #[arg(long, conflicts_with = "project_id")]
        project: Option<String>,

        /// Project ID to narrow the remote results
        #[arg(long = "project-id", conflicts_with = "project")]
        project_id: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Human)]
        format: ConfigOutputFormat,
    },
}

#[cfg(test)]
mod tests;
