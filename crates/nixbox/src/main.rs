use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "nixbox", version, about)]
struct Cli {}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_writer(std::io::stderr)
        .init();
    nixbox_tui::run().await
}
