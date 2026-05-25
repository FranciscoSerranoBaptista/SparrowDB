use clap::{Parser, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use eyre::Result;
pub use sparrow_cli::{DataAction, MetricsAction, ProjectConfigAction};
use std::io::IsTerminal;
use std::path::PathBuf;
use tui_banner::{Align, Banner, ColorMode, Fill, Gradient, Palette};

mod cleanup;
mod commands;
mod config;
mod docker;
mod errors;
mod metrics_sender;
mod output;
mod port;
mod project;
mod prompts;
mod update;
mod utils;

#[derive(Parser)]
#[command(name = "Sparrow CLI")]
#[command(version)]
struct Cli {
    /// Suppress output (errors and final result only)
    #[arg(long, global = true)]
    quiet: bool,

    /// Show detailed output with timing information
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Sparrow project with sparrow.toml
    Init {
        /// Project directory (defaults to current directory)
        #[arg(short, long)]
        path: Option<String>,

        #[arg(short, long, default_value = "empty")]
        template: String,

        /// Queries directory path (defaults to ./db/)
        #[arg(short, long = "queries-path", default_value = "./db/")]
        queries_path: String,

        /// Instance name for the default local instance
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Add a new local instance to an existing Sparrow project
    Add {
        /// Instance name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Validate project configuration and queries
    Check {
        /// Instance to check (defaults to all instances)
        instance: Option<String>,
        /// Write generated code without validation (for debugging codegen bugs)
        #[arg(long)]
        debug_codegen: bool,
    },

    /// Compile project queries into the workspace
    Compile {
        /// Directory containing sparrow.toml (defaults to current directory or project root)
        #[arg(short, long)]
        path: Option<String>,

        /// Path to output compiled queries
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Build and compile project for an instance
    Build {
        /// Instance name to build (interactive selection if not provided)
        #[arg(short, long)]
        instance: Option<String>,
        /// Should build SparrowDB into a binary at the specified directory location
        #[arg(long)]
        bin: Option<String>,
    },

    /// Deploy/start an instance
    Push {
        /// Instance name to push (interactive selection if not provided)
        instance: Option<String>,
    },

    /// Start an instance (doesn't rebuild)
    Start {
        /// Instance name to start (interactive selection if not provided)
        instance: Option<String>,
    },

    /// Run a pre-built binary directly without Docker
    Run {
        /// Directory containing the built binary (output of `sparrow build --bin <dir>`)
        #[arg(long)]
        bin: String,
        /// Instance name for config lookup (port, data-dir defaults)
        #[arg(short, long)]
        instance: Option<String>,
        /// Override the data directory (sets SPARROW_DATA_DIR)
        #[arg(long)]
        data_dir: Option<String>,
        /// Override the port (sets SPARROW_PORT)
        #[arg(long)]
        port: Option<u16>,
    },

    /// Stop an instance
    Stop {
        /// Instance name to stop (interactive selection if not provided)
        instance: Option<String>,
    },

    /// Restart an instance (stop then start)
    Restart {
        /// Instance name to restart (interactive selection if not provided)
        instance: Option<String>,
    },

    /// Show status of all instances
    Status,

    /// View logs for an instance
    Logs {
        /// Instance name (interactive selection if not provided)
        instance: Option<String>,

        /// Stream live logs (non-interactive)
        #[arg(long, short = 'l')]
        live: bool,

        /// Query historical logs with time range
        #[arg(long, short = 'r')]
        range: bool,

        /// Start time (ISO 8601: 2024-01-15T10:00:00Z)
        #[arg(long, requires = "range")]
        start: Option<String>,

        /// End time (ISO 8601: 2024-01-15T11:00:00Z)
        #[arg(long, requires = "range")]
        end: Option<String>,
    },

    /// Prune containers, images and workspace (preserves volumes)
    Prune {
        /// Instance to prune (if not specified, prunes unused resources)
        instance: Option<String>,

        /// Prune all instances in project
        #[arg(short, long)]
        all: bool,
    },

    /// Delete an instance completely
    Delete {
        /// Instance name to delete
        instance: String,
    },

    /// Manage metrics collection
    Metrics {
        #[command(subcommand)]
        action: MetricsAction,
    },

    /// Manage database data (snapshot, clone, restore)
    Data {
        #[command(subcommand)]
        action: DataAction,
    },

    /// Pre-flight health checklist — check CLI, workspace, Docker, and instances
    Doctor {
        /// Output as JSON (for CI)
        #[arg(long)]
        json: bool,
    },

    /// Update to the latest version
    Update {
        /// Force update even if already on latest version
        #[arg(long)]
        force: bool,
    },

    /// Manage schema migrations (status / apply / list)
    Migrate {
        #[command(subcommand)]
        subcommand: commands::migrate::MigrateSubcommand,
    },

    /// Upgrade a v1 project to v2 format
    Upgrade {
        /// Project directory to upgrade (defaults to current directory)
        #[arg(short, long)]
        path: Option<String>,

        /// Directory to move .hx files to (defaults to ./db/)
        #[arg(short, long = "queries-dir", default_value = "./db/")]
        queries_dir: String,

        /// Name for the default local instance (defaults to "dev")
        #[arg(short, long, default_value = "dev")]
        instance_name: String,

        /// Port for local instance (defaults to 6969)
        #[arg(long, default_value = "6969")]
        port: u16,

        /// Show what would be changed without making changes
        #[arg(long)]
        dry_run: bool,

        /// Skip creating backup of v1 files
        #[arg(long)]
        no_backup: bool,
    },

    /// Backup instance at the given path
    Backup {
        /// Instance name to backup
        instance: String,

        /// Output directory for the backup. If omitted, ./backups/backup-<ts>/ will be used
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Bulk-import records from a JSON, CSV, or Parquet file into a running SparrowDB instance.
    ///
    /// Each record in the file is posted as a JSON object to `POST /<query>`.
    /// The object keys must match the named parameters of the HQL query.
    ///
    /// # Examples
    ///
    ///   sparrow import users.json   --query CreateUser
    ///   sparrow import users.csv    --query CreateUser --workers 16
    ///   sparrow import data.parquet --query ImportEvent --target http://prod:6969
    Import {
        /// Path to the import file (JSON array, CSV with header row, or Parquet)
        file: std::path::PathBuf,

        /// Name of the compiled HQL query to call for every record.
        /// Required unless --query-column is provided.
        #[arg(long, short = 'q', required_unless_present = "query_column")]
        query: Option<String>,

        /// Column/field name in the file whose value is the query to call for
        /// that specific record.  The column is stripped before posting so it
        /// is not sent as a query parameter.  --query is used as a fallback
        /// when the column is absent or empty.
        ///
        /// Example: a mixed node+edge file where each row has a `_query`
        /// column set to "CreateUser", "CreateProduct", or "ConnectPurchase".
        #[arg(long, short = 'c')]
        query_column: Option<String>,

        /// SparrowDB target URL
        #[arg(long, short = 't', default_value = "http://localhost:6969")]
        target: String,

        /// Number of concurrent HTTP workers
        #[arg(long, short = 'w', default_value_t = 8)]
        workers: usize,

        /// Auth token (or set SPARROW_TOKEN env var)
        #[arg(long, env = "SPARROW_TOKEN")]
        token: Option<String>,

        /// Parse the file and print a preview without sending any requests
        #[arg(long)]
        dry_run: bool,

        /// Override format detection (json | csv | parquet)
        #[arg(long, short = 'f')]
        format: Option<String>,

        /// What to do when a record fails: continue | abort
        #[arg(long, default_value = "continue")]
        on_error: String,
    },

    /// Export records from a running SparrowDB instance to a JSON, CSV, or Parquet file.
    ///
    /// Calls `POST /<query>` with an optional JSON body and writes the response
    /// records to the output file.  The output format is inferred from the file
    /// extension; use `--format` to override.
    ///
    /// # Examples
    ///
    ///   sparrow export users.json    --query GetAllUsers
    ///   sparrow export edges.csv     --query GetPurchases --key purchases
    ///   sparrow export snapshot.parquet --query Dump --params '{"limit":1000}'
    Export {
        /// Output file path (extension determines format: .json, .csv, .parquet / .pq)
        file: std::path::PathBuf,

        /// Name of the compiled HQL query to call
        #[arg(long, short = 'q')]
        query: String,

        /// JSON key in the response object whose array contains the records.
        /// Auto-detected when the response has exactly one key; required otherwise.
        #[arg(long, short = 'k')]
        key: Option<String>,

        /// SparrowDB target URL
        #[arg(long, short = 't', default_value = "http://localhost:6969")]
        target: String,

        /// Auth token (or set SPARROW_TOKEN env var)
        #[arg(long, env = "SPARROW_TOKEN")]
        token: Option<String>,

        /// JSON object to send as the request body (default: `{}`)
        #[arg(long, short = 'p')]
        params: Option<String>,

        /// Pretty-print JSON output (only applies to .json format)
        #[arg(long)]
        pretty: bool,

        /// Override format detection (json | csv | parquet)
        #[arg(long, short = 'f')]
        format: Option<String>,
    },

    /// Run a pre-production stress test against a running SparrowDB instance
    ///
    /// Requires the instance to be compiled with the People/Company/Jobs schema and queries.
    Stress {
        /// SparrowDB endpoint (with or without http://)
        #[arg(long, default_value = "localhost")]
        endpoint: String,

        /// SparrowDB port
        #[arg(long, default_value_t = 6969)]
        port: u16,

        /// Number of people to generate
        #[arg(long, default_value_t = 1000)]
        num_people: usize,

        /// Number of companies to generate
        #[arg(long, default_value_t = 50)]
        num_companies: usize,

        /// Number of job titles to generate
        #[arg(long, default_value_t = 30)]
        num_jobs: usize,

        /// Number of parallel workers
        #[arg(long, default_value_t = 10)]
        workers: usize,

        /// Progress print interval (every N people inserted)
        #[arg(long, default_value_t = 100)]
        progress_interval: usize,

        /// Skip write phases and only verify persisted data (use after a restart)
        #[arg(long, default_value_t = false)]
        verify_only: bool,
    },
}

/// Display the welcome banner and getting started guide
fn display_welcome(update_available: Option<String>) {
    let use_color = std::io::stdout().is_terminal();

    // Generate ASCII art banner using tui-banner

    if let Ok(banner) = Banner::new("> SPARROW DB") {
        let banner = banner
            .color_mode(ColorMode::TrueColor)
            .gradient(Gradient::vertical(Palette::from_hex(&[
                "#ff7f17", // light orange
                "#e36600", // orange
                "#8f4000", // dark orange
            ])))
            .fill(Fill::Keep)
            .dither()
            .targets("░▒▓")
            .checker(3)
            .align(Align::Center)
            .padding(3)
            .render();

        println!("{banner}");
    }

    // Version info
    let version = update::current_version();
    if use_color {
        println!(
            "  {} {}\n",
            "SparrowDB CLI".bold(),
            format!("v{}", version).dimmed()
        );
    } else {
        println!("  SparrowDB CLI v{}\n", version);
    }

    // Update notification (after banner and version)
    if let Some(latest_version) = update_available {
        if use_color {
            println!(
                "  │ Update available: v{} ➜ {}",
                version,
                format!("v{}", latest_version).green().bold()
            );
            println!(
                "  │ Run '{}' to upgrade\n",
                "sparrow update".truecolor(255, 165, 54).bold()
            );
        } else {
            println!("  | Update available: v{} ➜ v{}", version, latest_version);
            println!("  | Run 'sparrow update' to upgrade\n");
        }
    }

    // Getting Started section
    println!(
        "{}",
        if use_color {
            "Getting Started".bold().to_string()
        } else {
            "Getting Started".to_string()
        }
    );
    println!();
    print_command("sparrow init", "Create a new SparrowDB project", use_color);
    print_command("sparrow build", "Build your project", use_color);
    print_command("sparrow push", "Deploy/start an instance", use_color);

    println!();
    println!(
        "{}",
        if use_color {
            "Common Commands".bold().to_string()
        } else {
            "Common Commands".to_string()
        }
    );
    println!();
    print_command("sparrow status", "Show status of all instances", use_color);
    print_command("sparrow logs", "View logs for an instance", use_color);

    println!();
    println!(
        "{}",
        if use_color {
            "Help & Info".bold().to_string()
        } else {
            "Help & Info".to_string()
        }
    );
    println!();
    print_command("sparrow --help", "Show all available commands", use_color);
    print_command(
        "sparrow <command> --help",
        "Show help for a specific command",
        use_color,
    );

    println!();
    if use_color {
        println!(
            "  {} {}",
            "Docs:".dimmed(),
            "https://docs.helix-db.com"
                .truecolor(253, 169, 66)
                .underline()
        );
    } else {
        println!("  Docs: https://docs.helix-db.com");
    }
    println!();
}

fn print_command(cmd: &str, desc: &str, use_color: bool) {
    if use_color {
        println!(
            "  {}  {}",
            cmd.truecolor(255, 165, 54).bold(),
            desc.dimmed()
        );
    } else {
        println!("  {:30} {}", cmd, desc);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize error reporting
    color_eyre::install()?;

    // Initialize metrics sender
    let metrics_sender = metrics_sender::MetricsSender::new()?;

    // Send CLI install event (only first time)
    metrics_sender.send_cli_install_event_if_first_time();

    // Check for updates before processing commands
    let update_available = update::check_for_updates().await?;

    let cli = Cli::parse();

    // Set verbosity level from flags
    output::Verbosity::set(output::Verbosity::from_flags(cli.quiet, cli.verbose));

    let result = match cli.command {
        None => {
            display_welcome(update_available);
            Ok(())
        }
        Some(cmd) => match cmd {
            Commands::Init {
                path,
                template,
                queries_path,
                name,
            } => commands::init::run(path, template, queries_path, name).await,
            Commands::Add { name } => commands::add::run(name).await,
            Commands::Check { instance, debug_codegen } => {
                commands::check::run(instance, &metrics_sender, debug_codegen).await
            }
            Commands::Compile { output, path } => commands::compile::run(output, path).await,
            Commands::Build { instance, bin } => {
                commands::build::run(instance, bin, &metrics_sender)
                    .await
                    .map(|_| ())
            }
            Commands::Push { instance } => {
                commands::push::run(instance, &metrics_sender).await
            }
            Commands::Start { instance } => commands::start::run(instance).await,
            Commands::Run {
                bin,
                instance,
                data_dir,
                port,
            } => commands::run::run(bin, instance, data_dir, port).await,
            Commands::Stop { instance } => commands::stop::run(instance).await,
            Commands::Restart { instance } => commands::restart::run(instance).await,
            Commands::Status => commands::status::run().await,
            Commands::Logs {
                instance,
                live,
                range,
                start,
                end,
            } => commands::logs::run(instance, live, range, start, end).await,
            Commands::Prune { instance, all } => commands::prune::run(instance, all).await,
            Commands::Delete { instance } => commands::delete::run(instance).await,
            Commands::Metrics { action } => commands::metrics::run(action).await,
            Commands::Data { action } => commands::data::run(action).await,
            Commands::Doctor { json } => commands::doctor::run(json).await,
            Commands::Update { force } => commands::update::run(force).await,
            Commands::Migrate { subcommand } => commands::migrate::run(subcommand).await,
            Commands::Upgrade {
                path,
                queries_dir,
                instance_name,
                port,
                dry_run,
                no_backup,
            } => {
                commands::upgrade::run(path, queries_dir, instance_name, port, dry_run, no_backup)
                    .await
            }
            Commands::Backup { instance, output } => commands::backup::run(output, instance).await,
            Commands::Import {
                file,
                query,
                query_column,
                target,
                workers,
                token,
                dry_run,
                format,
                on_error,
            } => {
                use commands::import::OnError;
                let on_error = match on_error.to_ascii_lowercase().as_str() {
                    "abort" => OnError::Abort,
                    "continue" | _ => OnError::Continue,
                };
                commands::import::run(
                    file, query, query_column, target, workers,
                    token, dry_run, format, on_error,
                ).await
            }
            Commands::Export {
                file,
                query,
                key,
                target,
                token,
                params,
                pretty,
                format,
            } => commands::export::run(file, query, key, target, token, params, pretty, format).await,
            Commands::Stress {
                endpoint,
                port,
                num_people,
                num_companies,
                num_jobs,
                workers,
                progress_interval,
                verify_only,
            } => {
                commands::stress::run(
                    endpoint,
                    port,
                    num_people,
                    num_companies,
                    num_jobs,
                    workers,
                    progress_interval,
                    verify_only,
                )
                .await
            }
        },
    };

    // Shutdown metrics sender
    metrics_sender.shutdown().await?;

    // Handle result with proper error formatting
    if let Err(e) = result {
        if let Some(cli_error) = e.downcast_ref::<crate::errors::CliError>() {
            eprint!("{}", cli_error.render());
        } else if let Some(config_error) = e.downcast_ref::<crate::errors::ConfigError>() {
            eprint!("{}", config_error.to_cli_error().render());
        } else if let Some(project_error) = e.downcast_ref::<crate::errors::ProjectError>() {
            eprint!("{}", project_error.to_cli_error().render());
        } else if let Some(port_error) = e.downcast_ref::<crate::errors::PortError>() {
            eprint!("{}", port_error.to_cli_error().render());
        } else {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }

    Ok(())
}
