pub mod alert;
pub mod cli;
pub mod collector;
pub mod config;
pub mod engine;
pub mod models;
pub mod persistence;

use anyhow::{Context, Result};
use clap::Parser;
use cli::CliArgs;
use config::AppConfig;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = CliArgs::parse();
    let config = AppConfig::try_from(cli)?;

    let max_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let subscriber = FmtSubscriber::builder().with_max_level(max_level).finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("failed to initialize tracing subscriber")?;

    tracing::info!(?config, "Loaded application configuration");

    engine::run(config).await
}
