use anyhow::Result;
use clap::Parser;
use kanade_tui::ws;
use tracing::info;

#[derive(Parser)]
#[command(name = "kanade-tui", about = "Terminal UI for Kanade music server")]
struct Cli {
    #[arg(short, long, env = "WS_URL", default_value = "127.0.0.1:8080")]
    server: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_path = std::env::var("LOG_PATH").unwrap_or_else(|_| "kanade-tui.log".to_string());
    let log_file = std::fs::File::create(&log_path)?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kanade=info".parse().unwrap()),
        )
        .with_writer(log_file)
        .init();

    info!("kanade-tui starting … connecting to {}", cli.server);

    let ws_url = if cli.server.contains("://") {
        cli.server.clone()
    } else {
        format!("ws://{}/ws", cli.server)
    };
    let (ws_rx, ws_tx) = ws::connect(&ws_url).await?;

    kanade_tui::run(ws_rx, ws_tx).await?;

    info!("kanade-tui shutting down.");
    Ok(())
}
