use std::{
    future::IntoFuture,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use kanade_core::{
    controller::Core,
    model::{Node, NodeType, RepeatMode},
    plugin::PluginBridge,
    ports::{EventBroadcaster, StatePersister},
    state::PlaybackState,
};
use kanade_scanner::spawn_background_scan;
use tracing::{info, warn};

use kanade_adapter_openhome::{OpenHomeBroadcaster, OpenHomeServer};
use kanade_adapter_ws::{build_router, AppState, MediaKeyStore, WsBroadcaster};

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

    let bind_addr: SocketAddr = std::env::var("BIND_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8080".parse().unwrap());

    let public_host = std::env::var("PUBLIC_HOST").ok();
    let media_public_base_url = match &public_host {
        Some(host) => {
            if host.starts_with("http://") || host.starts_with("https://") {
                host.clone()
            } else if host.contains(':') {
                format!("http://{}", host)
            } else {
                format!("https://{}", host)
            }
        }
        None => format!("http://127.0.0.1:{}", bind_addr.port()),
    };

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
                let bridge =
                    Arc::new(PluginBridge::new(vec![Arc::new(scrobbler)
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

        let (queue, current_index, selected_node_id, shuffle, repeat) = if let Some(saved_state) = saved {
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

            (queue, current_index, saved_state.active_output_id, saved_state.shuffle, repeat)
        } else {
            (Vec::new(), None, None, false, RepeatMode::Off)
        };

        let saved_nodes = db.load_all_node_states().unwrap_or_default();
        let nodes: Vec<Node> = saved_nodes
            .into_iter()
            .filter(|s| s.node_id != "__global__")
            .map(|s| {
                let queue: Vec<_> = s.queue_file_paths
                    .iter()
                    .filter_map(|path| db.get_track_by_path(path).ok().flatten())
                    .collect();
                let current_index = s.current_index.filter(|idx| *idx < queue.len());
                let repeat = match s.repeat.as_str() {
                    "one" => RepeatMode::One,
                    "all" => RepeatMode::All,
                    _ => RepeatMode::Off,
                };
                Node {
                    id: s.node_id,
                    name: String::new(),
                    connected: false,
                    status: kanade_core::model::PlaybackStatus::Stopped,
                    position_secs: 0.0,
                    volume: s.volume,
                    node_type: NodeType::Remote,
                    queue,
                    current_index,
                    repeat,
                    shuffle: s.shuffle,
                    device_id: None,
                }
            })
            .collect();

        Ok(PlaybackState {
            nodes,
            selected_node_id,
            queue,
            current_index,
            shuffle,
            repeat,
        })
    })
    .await??;

    core.restore_state(restored).await;

    let media_key_store = Arc::new(MediaKeyStore::new());

    let app_state = Arc::new(AppState {
        core: Arc::clone(&core),
        db_path: PathBuf::from(&db_path),
        broadcaster: Arc::clone(&ws_broadcaster),
        media_base_url: media_public_base_url,
        media_key_store,
    });
    let app = build_router(app_state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind");
    info!(addr = %bind_addr, "Kanade server listening");

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

    let oh_addr: SocketAddr = std::env::var("OH_ADDR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:8090".parse().unwrap());
    let oh_server = OpenHomeServer::new(Arc::clone(&core), Arc::clone(&oh_broadcaster), oh_addr);

    let core_for_cleanup = Arc::clone(&core);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            core_for_cleanup
                .cleanup_disconnected_nodes(Duration::from_secs(30))
                .await;
        }
    });

    let mdns_instance_name = std::env::var("MDNS_NAME").unwrap_or_else(|_| "Kanade".to_string());
    let _mdns = match mdns_sd::ServiceDaemon::new() {
        Ok(mdns) => {
            let mut properties: Vec<(&str, String)> = vec![
                ("version", "1.0".to_string()),
                ("ws_port", bind_addr.port().to_string()),
            ];
            if let Some(host) = &public_host {
                properties.push(("host", host.clone()));
            }
            match mdns_sd::ServiceInfo::new(
                "_kanade._tcp.local.",
                &mdns_instance_name,
                &format!(
                    "{}.local.",
                    mdns_instance_name.to_lowercase().replace(' ', "-")
                ),
                "",
                bind_addr.port(),
                properties.as_slice(),
            ) {
                Ok(info) => match mdns.register(info) {
                    Ok(_) => {
                        info!(instance = %mdns_instance_name, port = bind_addr.port(), "mDNS service registered")
                    }
                    Err(e) => warn!(error = %e, "failed to register mDNS service"),
                },
                Err(e) => warn!(error = %e, "failed to create mDNS service info"),
            }
            Some(mdns)
        }
        Err(e) => {
            warn!(error = %e, "failed to create mDNS daemon");
            None
        }
    };

    tokio::select! {
        _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).into_future() => {}
        _ = oh_server.run() => {}
    }

    Ok(())
}
