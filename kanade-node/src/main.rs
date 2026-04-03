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
//! | Variable        | Default               | Description                        |
//! |-----------------|-----------------------|------------------------------------|
//! | `NODE_NAME`     | `node`                | Human-readable name for this node  |
//! | `SERVER_ADDR`   | `ws://127.0.0.1:8082` | kanade server node endpoint        |
//! | `MPD_HOST`      | `127.0.0.1`           | Local MPD host                     |
//! | `MPD_PORT`      | `6600`                | Local MPD port                     |

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
    model::{Node, PlaybackStatus},
    ports::{AudioOutput, EventBroadcaster},
    state::PlaybackState,
};
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeRegistrationAck, NodeStateUpdate};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

// ── NodeEventBroadcaster ──────────────────────────────────────────────────────

/// An [`EventBroadcaster`] that converts [`PlaybackState`] snapshots into
/// [`NodeStateUpdate`] messages and sends them to the server over the WebSocket
/// connection.
struct NodeEventBroadcaster {
    tx: tokio::sync::Mutex<mpsc::Sender<String>>,
    projection_generation: Arc<AtomicU64>,
}

impl NodeEventBroadcaster {
    fn new(
        tx: mpsc::Sender<String>,
        projection_generation: Arc<AtomicU64>,
    ) -> Self {
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
                mpd_song_index: node.current_index,
                projection_generation: self.projection_generation.load(Ordering::Relaxed),
            };
            if let Ok(json) = serde_json::to_string(&update) {
                let tx = self.tx.lock().await;
                let _ = tx.send(json).await;
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
    let server_addr = std::env::var("SERVER_ADDR")
        .unwrap_or_else(|_| "ws://127.0.0.1:8082".to_string());
    let mpd_host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mpd_port: u16 = std::env::var("MPD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6600);

    info!("Kanade output node starting: name={node_name}, server={server_addr}");

    // ── Shared state (lives across reconnections) ─────────────────────────────
    let local_state: Arc<RwLock<PlaybackState>> = Arc::new(RwLock::new(PlaybackState {
        nodes: vec![Node {
            id: String::new(),
            name: node_name.clone(),
            output_ids: Vec::new(),
            queue: Vec::new(),
            current_index: None,
            status: PlaybackStatus::Stopped,
            position_secs: 0.0,
            volume: 50,
            shuffle: false,
            repeat: kanade_core::model::RepeatMode::Off,
        }],
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
    local_state: &Arc<RwLock<PlaybackState>>,
    projection_generation: &Arc<AtomicU64>,
    broadcaster: &Arc<NodeEventBroadcaster>,
) -> Result<()> {
    info!("Connecting to {server_addr} …");

    let (ws_stream, _) = tokio::time::timeout(
        Duration::from_secs(10),
        connect_async(server_addr),
    )
    .await
    .map_err(|_| anyhow::anyhow!("connection timed out"))??;
    info!("Connected");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // ── Handshake ─────────────────────────────────────────────────────────
    let registration = NodeRegistration {
        name: node_name.to_string(),
    };
    ws_tx
        .send(Message::Text(serde_json::to_string(&registration)?))
        .await?;

    let (node_id, media_base_url): (String, String) = loop {
        match tokio::time::timeout(Duration::from_secs(10), ws_rx.next()).await {
            Err(_) => return Err(anyhow::anyhow!("handshake timed out")),
            Ok(Some(Ok(Message::Text(text)))) => {
                match serde_json::from_str::<NodeRegistrationAck>(&text) {
                    Ok(ack) => break (ack.node_id, ack.media_base_url),
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

    info!("Registered: node_id={node_id}, media_base_url={media_base_url}");

    {
        let mut state = local_state.write().await;
        if let Some(node) = state.nodes.first_mut() {
            node.id = node_id.clone();
            node.output_ids = vec![node_id.clone()];
        }
    }

    // ── Retarget broadcaster to this session's channel ────────────────────
    {
        let (state_tx, mut state_rx) = mpsc::channel::<String>(64);
        broadcaster.retarget(state_tx).await;

        // ── Renderer (rebuilt each session in case media_base_url changed) ─
        let renderer = Arc::new(MpdRenderer::new(mpd_host, mpd_port, media_base_url));

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
                    if ws_tx.send(Message::Text(json)).await.is_err() {
                        error!("Failed to send state update");
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Execute a [`NodeCommand`] against the local [`MpdRenderer`].
async fn execute_command(
    cmd: NodeCommand,
    renderer: &Arc<MpdRenderer>,
    projection_generation: &Arc<AtomicU64>,
) {
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
