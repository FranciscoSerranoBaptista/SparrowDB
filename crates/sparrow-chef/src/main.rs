use clap::{Parser, Subcommand};
use eyre::Result;

mod commands;
mod docker;
mod http;
mod prompts;
mod templates;

#[derive(Parser)]
#[command(
    name = "sparrowdb-chef",
    version,
    about = "Bootstrap a SparrowDB application for a coding agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap a new SparrowDB project (alias: cook)
    #[command(alias = "cook")]
    Chef {
        /// Skip prompts and run with defaults
        #[arg(short = 'a', long)]
        auto: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Commands::Chef { auto } => commands::chef::run(auto).await,
    }
}
