//! kanade-node — Kanade output node binary.
//!
//! An output node connects to the Kanade server over WebSocket using the
//! kanade protocol, receives [`NodeCommand`] playback commands, and drives a
//! local MPD daemon via [`MpdRenderer`].  State changes observed from MPD are
//! reported back to the server as [`NodeStateUpdate`] messages so the server's
//! [`PlaybackState`] stays in sync.
//!
//! The server automatically assigns a unique identifier (UUID) to each
//! connected node.  The node only provides a human-readable name.
//!
//! # Resilience
//!
//! The node automatically reconnects to the server with exponential backoff
//! when the connection drops or the handshake fails.  The MPD state sync task
//! runs independently and is reused across reconnections.
//!
//! # Configuration (environment variables)
//!
//! | Variable          | Default               | Description                        |
//! |-------------------|-----------------------|------------------------------------|
//! | `NODE_NAME`       | `node`                | Human-readable name for this node  |
//! | `SERVER_ADDR`     | `127.0.0.1:8080`     | Kanade server address (host:port)   |
//! | `MPD_HOST`        | `127.0.0.1`           | Local MPD host                     |
//! | `MPD_PORT`        | `6600`                | Local MPD port                     |
//! | `LOCAL_PROXY_PORT`| `18080`               | Local HTTP media-proxy port        |

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use kanade_adapter_mpd::{MpdClient, MpdRenderer, MpdStateSync};
use kanade_core::{
    model::Node,
    ports::{AudioOutput, EventBroadcaster},
    state::PlaybackState,
};
use kanade_node_protocol::NodeRegistrationAck;
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeStateUpdate};
use tokio::sync::{mpsc, RwLock};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

mod proxy;

// ── NodeEventBroadcaster ──────────────────────────────────────────────────────

/// An [`EventBroadcaster`] that converts [`PlaybackState`] snapshots into
/// [`NodeStateUpdate`] messages and sends them to the server over the WebSocket
/// connection.
struct NodeEventBroadcaster {
    tx: tokio::sync::Mutex<mpsc::Sender<String>>,
    projection_generation: Arc<AtomicU64>,
}

impl NodeEventBroadcaster {
    fn new(tx: mpsc::Sender<String>, projection_generation: Arc<AtomicU64>) -> Self {
        Self {
            tx: tokio::sync::Mutex::new(tx),
            projection_generation,
        }
    }

    async fn retarget(&self, tx: mpsc::Sender<String>) {
        *self.tx.lock().await = tx;
    }
}

#[async_trait]
impl EventBroadcaster for NodeEventBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        if let Some(node) = state.nodes.first() {
            let update = NodeStateUpdate {
                status: node.status,
                position_secs: node.position_secs,
                volume: node.volume,
                mpd_song_index: state.current_index,
                projection_generation: self.projection_generation.load(Ordering::Relaxed),
            };
            if let Ok(json) = serde_json::to_string(&update) {
                let tx = self.tx.lock().await;
                let _ = tx.try_send(json);
            }
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kanade_node=info,kanade_adapter_mpd=debug".parse().unwrap()),
        )
        .init();

    let node_name = std::env::var("NODE_NAME").unwrap_or_else(|_| "node".to_string());
    let server_addr_raw =
        std::env::var("SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let server_addr = if server_addr_raw.contains("://") {
        server_addr_raw
    } else {
        format!("ws://{server_addr_raw}/ws")
    };
    let mpd_host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mpd_port: u16 = std::env::var("MPD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6600);
    let proxy_port: u16 = std::env::var("LOCAL_PROXY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(18080);

    info!("Kanade output node starting: name={node_name}, server={server_addr}");

    // ── Local media proxy (lives for the lifetime of the process) ─────────────
    //
    // MPD receives loopback URLs (http://127.0.0.1:{proxy_port}/media/tracks/…).
    // When MPD fetches a queued URL the proxy re-signs it with the current
    // session key and issues an HTTP 302 redirect to the real Kanade server.
    // This ensures the signed URL is always fresh at the moment of playback,
    // regardless of how long the track has been sitting in the queue.
    let media_proxy = proxy::MediaProxy::new(proxy_port);
    {
        let proxy = media_proxy.clone();
        tokio::spawn(async move { proxy.run().await });
    }

    // ── Shared state (lives across reconnections) ─────────────────────────────
    let local_state: Arc<RwLock<PlaybackState>> = Arc::new(RwLock::new(PlaybackState {
        nodes: vec![Node {
            id: String::new(),
            name: node_name.clone(),
            ..Default::default()
        }],
        selected_node_id: None,
        queue: Vec::new(),
        current_index: None,
        shuffle: false,
        repeat: kanade_core::model::RepeatMode::Off,
    }));

    let projection_generation = Arc::new(AtomicU64::new(0));

    let broadcaster: Arc<NodeEventBroadcaster> = Arc::new(NodeEventBroadcaster::new(
        mpsc::channel::<String>(64).0,
        Arc::clone(&projection_generation),
    ));

    // Spawn MPD state sync once — it runs for the lifetime of the process.
    {
        let state = Arc::clone(&local_state);
        let gen = Arc::clone(&projection_generation);
        let bcast = Arc::downgrade(&broadcaster);
        let sync_mpd_host = mpd_host.clone();
        tokio::spawn(async move {
            let mut sync = MpdStateSync::new(
                sync_mpd_host.clone(),
                mpd_port,
                MpdClient::new(sync_mpd_host, mpd_port),
                state,
                vec![Arc::new(WeakBroadcaster(bcast)) as Arc<dyn EventBroadcaster>],
                gen,
            );
            sync.run().await;
        });
    }

    // ── Reconnect loop ───────────────────────────────────────────────────────
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        match run_session(
            &server_addr,
            &node_name,
            &mpd_host,
            mpd_port,
            &media_proxy,
            &local_state,
            &projection_generation,
            &broadcaster,
        )
        .await
        {
            Ok(()) => {
                // Clean close (shouldn't happen in normal operation, but handle
                // it the same as a disconnect).
                info!("Session ended; reconnecting in {backoff:?} …");
            }
            Err(e) => {
                warn!("Session error: {e}; reconnecting in {backoff:?} …");
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Delegates to the current [`NodeEventBroadcaster`] via a weak reference,
/// so the sync task can outlive individual sessions.
struct WeakBroadcaster(std::sync::Weak<NodeEventBroadcaster>);

#[async_trait]
impl EventBroadcaster for WeakBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        if let Some(b) = self.0.upgrade() {
            b.on_state_changed(state).await;
        }
    }
}

/// A single server session: connect, handshake, relay loop.
/// Returns when the connection drops for any reason.
async fn run_session(
    server_addr: &str,
    node_name: &str,
    mpd_host: &str,
    mpd_port: u16,
    media_proxy: &proxy::MediaProxy,
    local_state: &Arc<RwLock<PlaybackState>>,
    projection_generation: &Arc<AtomicU64>,
    broadcaster: &Arc<NodeEventBroadcaster>,
) -> Result<()> {
    info!("Connecting to {server_addr} …");

    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(server_addr))
        .await
        .map_err(|_| anyhow::anyhow!("connection timed out"))??;
    info!("Connected");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // ── Handshake ─────────────────────────────────────────────────────────
    let registration = NodeRegistration {
        node_id: Some(node_name.to_string()),
        display_name: Some(node_name.to_string()),
        name: None,
    };
    ws_tx
        .send(Message::Text(serde_json::to_string(&registration)?))
        .await?;

    let (node_id, media_base_url, media_hmac_auth): (String, String, Option<(String, [u8; 32])>) = loop {
        match tokio::time::timeout(Duration::from_secs(10), ws_rx.next()).await {
            Err(_) => return Err(anyhow::anyhow!("handshake timed out")),
            Ok(Some(Ok(Message::Text(text)))) => {
                match serde_json::from_str::<NodeRegistrationAck>(&text) {
                    Ok(ack) => {
                        let media_hmac_auth = parse_media_auth(&ack);
                        break (ack.node_id, ack.media_base_url, media_hmac_auth);
                    }
                    Err(e) => warn!("Unexpected message before ack: {e}"),
                }
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {
                return Err(anyhow::anyhow!("server closed during handshake"));
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("WS error during handshake: {e}")),
        }
    };

    info!(
        "Registered: node_id={node_id}, media_base_url={media_base_url}, media_auth={}",
        media_hmac_auth.is_some()
    );

    // Update the local proxy with the fresh session credentials.
    // The proxy re-signs URLs just-in-time, so MPD tracks added hours ago
    // are still fetchable when playback reaches them.
    media_proxy.update(media_base_url, media_hmac_auth).await;

    {
        let mut state = local_state.write().await;
        if let Some(node) = state.nodes.first_mut() {
            node.id = node_id.clone();
            node.connected = true;
        }
    }

    // ── Retarget broadcaster to this session's channel ────────────────────
    {
        let (state_tx, mut state_rx) = mpsc::channel::<String>(64);
        broadcaster.retarget(state_tx).await;

        // ── Renderer — always points at the local proxy ───────────────────
        //
        // The proxy URL (http://127.0.0.1:{proxy_port}) never changes, so the
        // renderer does not need the auth key itself.  Signing happens inside
        // the proxy at the moment MPD requests each track.
        let renderer = Arc::new(MpdRenderer::new(mpd_host, mpd_port, media_proxy.base_url()));
        if let Err(e) = renderer.clear().await {
            warn!("Failed to clear stale MPD queue: {e}");
        }

        // ── Relay loop ───────────────────────────────────────────────────
        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<NodeCommand>(&text) {
                                Ok(cmd) => {
                                    execute_command(cmd, &renderer, projection_generation).await;
                                }
                                Err(e) => warn!("Unexpected message from server: {e}"),
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            match timeout(Duration::from_secs(5), ws_tx.send(Message::Pong(payload))).await {
                                Ok(Ok(())) => {}
                                _ => {
                                    error!("Failed to reply to server ping");
                                    return Ok(());
                                }
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) | None => {
                            info!("Server disconnected");
                            return Ok(());
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            error!("WebSocket error: {e}");
                            return Ok(());
                        }
                    }
                }
                Some(json) = state_rx.recv() => {
                    match timeout(Duration::from_secs(5), ws_tx.send(Message::Text(json))).await {
                        Ok(Ok(())) => {}
                        _ => {
                            error!("Failed to send state update");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

fn parse_media_auth(ack: &NodeRegistrationAck) -> Option<(String, [u8; 32])> {
    let (Some(key_id), Some(key_hex)) = (&ack.media_auth_key_id, &ack.media_auth_key) else {
        return None;
    };

    let key_bytes = match hex::decode(key_hex) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!("invalid media_auth_key hex from server: {e}");
            return None;
        }
    };

    let key_len = key_bytes.len();
    let key_array: [u8; 32] = match key_bytes.try_into() {
        Ok(key) => key,
        Err(_) => {
            warn!(
                "invalid media_auth_key length from server: expected 32 bytes, got {}",
                key_len
            );
            return None;
        }
    };

    Some((key_id.clone(), key_array))
}

/// Execute a [`NodeCommand`] against the local [`MpdRenderer`].
async fn execute_command(
    cmd: NodeCommand,
    renderer: &Arc<MpdRenderer>,
    projection_generation: &Arc<AtomicU64>,
) {
    info!(command = ?cmd, "kanade-node: executing command");
    let result = match cmd {
        NodeCommand::Play => renderer.play().await,
        NodeCommand::Pause => renderer.pause().await,
        NodeCommand::Stop => renderer.stop().await,
        NodeCommand::Seek { position_secs } => renderer.seek(position_secs).await,
        NodeCommand::SetVolume { volume } => renderer.set_volume(volume).await,
        NodeCommand::SetQueue {
            file_paths,
            projection_generation: command_projection_generation,
        } => {
            let result = renderer
                .set_queue(&file_paths, command_projection_generation)
                .await;
            if result.is_ok() {
                projection_generation.store(command_projection_generation, Ordering::Relaxed);
            }
            result
        }
        NodeCommand::Add { file_paths } => renderer.add(&file_paths).await,
        NodeCommand::Remove { index } => renderer.remove(index).await,
        NodeCommand::MoveTrack { from, to } => renderer.move_track(from, to).await,
    };
    if let Err(e) = result {
        warn!("Command execution error: {e}");
    }
}
