use anyhow::Result;
use kanade_tui::ws;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let log_path = std::env::var("LOG_PATH").unwrap_or_else(|_| "kanade.log".to_string());
    let log_file = std::fs::File::create(&log_path)?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kanade=info".parse().unwrap()),
        )
        .with_writer(log_file)
        .init();

    info!("kanade-tui starting …");

    let ws_url = std::env::var("WS_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:8080".to_string());

    let (ws_rx, ws_tx) = ws::connect(&ws_url).await?;
    let event_rx = kanade_tui::spawn_event_task();

    kanade_tui::run(ws_rx, ws_tx, event_rx).await?;

    info!("kanade-tui shutting down.");
    Ok(())
}
