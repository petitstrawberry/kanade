use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use kanade_core::{
    controller::Core,
    ports::EventBroadcaster,
};
use kanade_scanner::spawn_background_scan;
use tracing::info;

use kanade_adapter_node_server::NodeServer;
use kanade_adapter_openhome::{OpenHomeBroadcaster, OpenHomeServer};
use kanade_adapter_ws::{WsBroadcaster, WsServer};

use kanade_server_http::MediaServer;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kanade=info,kanade_core=debug".parse().unwrap()),
        )
        .init();

    info!("Kanade server starting …");

    let media_addr: SocketAddr = std::env::var("MEDIA_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8081".parse().unwrap());
    let media_public_base_url = std::env::var("MEDIA_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}", media_addr.port()));

    let (ws_broadcaster, _ws_rx) = WsBroadcaster::new(64);
    let oh_broadcaster = OpenHomeBroadcaster::new();

    let broadcasters: Vec<Arc<dyn EventBroadcaster>> = vec![
        Arc::clone(&ws_broadcaster) as Arc<dyn EventBroadcaster>,
        Arc::clone(&oh_broadcaster) as Arc<dyn EventBroadcaster>,
    ];

    // Core starts with no pre-registered outputs; output nodes connect at
    // runtime via the kanade protocol and are registered dynamically.
    let core = Arc::new(Core::new(vec![], broadcasters.clone()));

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "kanade.db".to_string());
    let _db = Arc::new(kanade_db::Database::open(&db_path)?);

    let media_server = MediaServer::new(PathBuf::from(&db_path), media_addr);
    tokio::spawn(async move {
        media_server.run().await;
    });

    let _scan_handle = if let Ok(music_dir) = std::env::var("MUSIC_DIR") {
        let scan_interval: u64 = std::env::var("SCAN_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        info!("library scanner spawned: dir={music_dir}, interval={scan_interval}s");

        Some(spawn_background_scan(
            PathBuf::from(&db_path),
            PathBuf::from(&music_dir),
            Duration::from_secs(scan_interval),
        ))
    } else {
        info!("MUSIC_DIR not set — library scanner disabled");
        None
    };

    let node_addr: SocketAddr = std::env::var("NODE_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8082".parse().unwrap());
    let node_server = NodeServer::new(
        Arc::clone(&core),
        node_addr,
        media_public_base_url,
    );

    let ws_addr: SocketAddr = std::env::var("WS_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8080".parse().unwrap());
    let ws_server = WsServer::new(
        Arc::clone(&core),
        PathBuf::from(&db_path),
        Arc::clone(&ws_broadcaster),
        ws_addr,
    );

    let oh_addr: SocketAddr = std::env::var("OH_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8090".parse().unwrap());
    let oh_server = OpenHomeServer::new(
        Arc::clone(&core),
        Arc::clone(&oh_broadcaster),
        oh_addr,
    );

    tokio::select! {
        _ = node_server.run() => {}
        _ = ws_server.run() => {}
        _ = oh_server.run() => {}
    }

    Ok(())
}

