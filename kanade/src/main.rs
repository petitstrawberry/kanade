use std::{net::SocketAddr, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

use anyhow::Result;
use kanade_core::{
    controller::Core,
    model::RepeatMode,
    plugin::PluginBridge,
    ports::{EventBroadcaster, StatePersister},
    state::PlaybackState,
};
use kanade_scanner::spawn_background_scan;
use tracing::info;

use kanade_adapter_openhome::{OpenHomeBroadcaster, OpenHomeServer};
use kanade_adapter_ws::{WsBroadcaster, WsServer};

use kanade_server_http::MediaServer;

mod persist;
use persist::DatabaseStatePersister;

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

    #[cfg(feature = "lastfm")]
    let broadcasters = {
        let mut b = broadcasters;
        match kanade_plugin_lastfm::LastFmScrobbler::from_env() {
            Ok(scrobbler) => {
                info!("Last.fm plugin loaded");
                let bridge = Arc::new(PluginBridge::new(vec![Arc::new(scrobbler)
                    as std::sync::Arc<dyn kanade_core::plugin::KanadePlugin>]));
                b.push(bridge as Arc<dyn EventBroadcaster>);
                b
            }
            Err(e) => {
                info!("Last.fm plugin disabled: {e}");
                b
            }
        }
    };

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "kanade.db".to_string());
    let db = Arc::new(Mutex::new(kanade_db::Database::open(&db_path)?));

    let mut core_instance = Core::new(vec![], broadcasters.clone());
    let persister: Arc<dyn StatePersister> = Arc::new(DatabaseStatePersister::new(Arc::clone(&db)));
    core_instance.add_persister(Arc::clone(&persister));
    let core = Arc::new(core_instance);

    let db_for_restore = Arc::clone(&db);
    let restored = tokio::task::spawn_blocking(move || -> anyhow::Result<PlaybackState> {
        let db = db_for_restore
            .lock()
            .map_err(|e| anyhow::anyhow!("database mutex poisoned: {e}"))?;

        let saved = db.load_playback_state()?;

        if let Some(saved_state) = saved {
            let queue = saved_state
                .queue_file_paths
                .iter()
                .filter_map(|path| match db.get_track_by_path(path) {
                    Ok(track) => track,
                    Err(e) => {
                        tracing::warn!(file_path = %path, error = %e, "failed to load track while restoring state");
                        None
                    }
                })
                .collect::<Vec<_>>();

            let current_index = saved_state
                .current_index
                .filter(|idx| *idx < queue.len());

            let repeat = match saved_state.repeat.as_str() {
                "one" => RepeatMode::One,
                "all" => RepeatMode::All,
                _ => RepeatMode::Off,
            };

            Ok(PlaybackState {
                nodes: Vec::new(),
                selected_node_id: saved_state.active_output_id,
                queue,
                current_index,
                shuffle: saved_state.shuffle,
                repeat,
            })
        } else {
            Ok(PlaybackState {
                nodes: Vec::new(),
                selected_node_id: None,
                queue: Vec::new(),
                current_index: None,
                shuffle: false,
                repeat: RepeatMode::Off,
            })
        }
    })
    .await??;

    core.restore_state(restored).await;

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
        media_public_base_url,
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

    let core_for_cleanup = Arc::clone(&core);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            core_for_cleanup.cleanup_disconnected_nodes(Duration::from_secs(30)).await;
        }
    });

    tokio::select! {
        _ = ws_server.run() => {}
        _ = oh_server.run() => {}
    }

    Ok(())
}
