use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use kanade_core::{
    controller::Core,
    model::{PlaybackStatus, Zone},
    ports::EventBroadcaster,
};
use kanade_scanner::spawn_background_scan;
use tracing::info;

use kanade_adapter_mpd::{MpdClient, MpdRenderer, MpdStateSync};
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

    let mpd_host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mpd_port: u16 = std::env::var("MPD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6600);

    let media_addr: SocketAddr = std::env::var("MEDIA_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8081".parse().unwrap());
    let media_public_base_url = std::env::var("MEDIA_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}", media_addr.port()));

    let mpd_output: Arc<dyn kanade_core::ports::AudioOutput> =
        Arc::new(MpdRenderer::new(
            mpd_host.clone(),
            mpd_port,
            media_public_base_url,
        ));

    let (ws_broadcaster, _ws_rx) = WsBroadcaster::new(64);
    let oh_broadcaster = OpenHomeBroadcaster::new();

    let broadcasters: Vec<Arc<dyn EventBroadcaster>> = vec![
        Arc::clone(&ws_broadcaster) as Arc<dyn EventBroadcaster>,
        Arc::clone(&oh_broadcaster) as Arc<dyn EventBroadcaster>,
    ];

    let core = Core::new(
        vec![("mpd".to_string(), mpd_output)],
        broadcasters.clone(),
    );

    let default_zone = Zone {
        id: "default".to_string(),
        name: "Default".to_string(),
        output_ids: vec!["mpd".to_string()],
        queue: Vec::new(),
        current_index: None,
        status: PlaybackStatus::Stopped,
        position_secs: 0.0,
        volume: 50,
        shuffle: false,
        repeat: kanade_core::model::RepeatMode::Off,
    };
    core.add_zone(default_zone).await;

    let core = Arc::new(core);

    let mut mpd_sync = MpdStateSync::new(
        mpd_host.clone(),
        mpd_port,
        MpdClient::new(mpd_host, mpd_port),
        core.state_handle(),
        broadcasters.clone(),
        core.queue_generation(),
    );
    tokio::spawn(async move {
        mpd_sync.run().await;
    });

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
        _ = ws_server.run() => {}
        _ = oh_server.run() => {}
    }

    Ok(())
}
