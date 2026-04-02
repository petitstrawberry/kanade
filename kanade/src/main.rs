//! Kanade — entry point.
//!
//! Wires together all crates according to the Hexagonal Architecture:
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────┐
//!  │                        Core                             │
//!  │  PlaybackState (Arc<RwLock<_>>) + CoreController        │
//!  └────────────┬────────────────────────────┬───────────────┘
//!               │ AudioRenderer (output port) │ EventBroadcaster (broadcast port)
//!  ┌────────────▼──────────┐    ┌────────────▼─────────────────────────────────┐
//!  │  kanade-adapter-mpd   │    │  kanade-adapter-ws   kanade-adapter-openhome │
//!  │  (MpdRenderer)        │    │  (WsBroadcaster)     (OpenHomeBroadcaster)   │
//!  └───────────────────────┘    └──────────────────────────────────────────────┘
//! ```

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use tracing::info;

use kanade_adapter_mpd::MpdRenderer;
use kanade_adapter_openhome::{OpenHomeBroadcaster, OpenHomeServer};
use kanade_adapter_ws::{WsBroadcaster, WsServer};
use kanade_core::controller::CoreController;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialise structured logging (reads RUST_LOG env var).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kanade=info,kanade_core=debug".parse().unwrap()),
        )
        .init();

    info!("Kanade starting …");

    // ------------------------------------------------------------------
    // 1. Output adapter — MPD
    // ------------------------------------------------------------------
    let mpd_host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mpd_port: u16 = std::env::var("MPD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6600);
    let renderer = Arc::new(MpdRenderer::new(mpd_host, mpd_port));

    // ------------------------------------------------------------------
    // 2. Broadcast adapters
    // ------------------------------------------------------------------
    let (ws_broadcaster, _ws_rx) = WsBroadcaster::new(64);
    let oh_broadcaster = OpenHomeBroadcaster::new();

    let broadcasters: Vec<Arc<dyn kanade_core::ports::EventBroadcaster>> = vec![
        Arc::clone(&ws_broadcaster) as Arc<dyn kanade_core::ports::EventBroadcaster>,
        Arc::clone(&oh_broadcaster) as Arc<dyn kanade_core::ports::EventBroadcaster>,
    ];

    // ------------------------------------------------------------------
    // 3. Core controller
    // ------------------------------------------------------------------
    let controller = Arc::new(CoreController::new(renderer, broadcasters));

    // ------------------------------------------------------------------
    // 4. WebSocket server
    // ------------------------------------------------------------------
    let ws_addr: SocketAddr = std::env::var("WS_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8080".parse().unwrap());
    let ws_server = WsServer::new(
        Arc::clone(&controller),
        Arc::clone(&ws_broadcaster),
        ws_addr,
    );

    // ------------------------------------------------------------------
    // 5. OpenHome HTTP server
    // ------------------------------------------------------------------
    let oh_addr: SocketAddr = std::env::var("OH_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8090".parse().unwrap());
    let oh_server = OpenHomeServer::new(
        Arc::clone(&controller),
        Arc::clone(&oh_broadcaster),
        oh_addr,
    );

    // ------------------------------------------------------------------
    // 6. Run all servers concurrently
    // ------------------------------------------------------------------
    tokio::select! {
        _ = ws_server.run() => {}
        _ = oh_server.run() => {}
    }

    Ok(())
}
