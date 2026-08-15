mod interactive;
mod rudder;

use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::prelude::*;

#[derive(Parser, Debug)]
#[command(version, about = "Control EoI boat systems over CAN")]
struct Cli {
    /// CAN interface
    #[arg(short = 'i', long, global = true, default_value_t = String::from("can0"))]
    can_interface: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Control the servo rudder controller
    #[command(subcommand)]
    Rudder(rudder::RudderCommand),
}

fn register_tracing_subscriber(level_filter: LevelFilter) {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(level_filter.into())
                .from_env_lossy(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    register_tracing_subscriber(LevelFilter::INFO);
    let cli = Cli::parse();
    info!("CAN interface: {}", cli.can_interface);

    match cli.command {
        Command::Rudder(command) => rudder::run(command, &cli.can_interface).await,
    }
}
