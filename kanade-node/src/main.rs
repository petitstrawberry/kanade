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
    tx: mpsc::Sender<String>,
}

#[async_trait]
impl EventBroadcaster for NodeEventBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        // MpdStateSync always operates on nodes[0]
        if let Some(node) = state.nodes.first() {
            let update = NodeStateUpdate {
                status: node.status,
                position_secs: node.position_secs,
                volume: node.volume,
                current_index: node.current_index,
            };
            if let Ok(json) = serde_json::to_string(&update) {
                let _ = self.tx.send(json).await;
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

    info!("Kanade output node starting: name={node_name}");
    info!("Connecting to server at {server_addr} …");

    // Connect to the server with retry
    let (ws_stream, _) = connect_async(&server_addr).await?;
    info!("Connected to server");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // ── Handshake ─────────────────────────────────────────────────────────────
    let registration = NodeRegistration {
        name: node_name.clone(),
    };
    let reg_json = serde_json::to_string(&registration)?;
    ws_tx.send(Message::Text(reg_json)).await?;

    // Wait for NodeRegistrationAck from server (carries the server-assigned node_id)
    let (node_id, media_base_url): (String, String) = loop {
        match ws_rx.next().await {
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<NodeRegistrationAck>(&text) {
                    Ok(ack) => break (ack.node_id, ack.media_base_url),
                    Err(e) => {
                        warn!("Unexpected message before ack: {e}");
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                error!("Server disconnected during handshake");
                return Ok(());
            }
            _ => continue,
        }
    };

    info!("Registration acknowledged; node_id={node_id}, media_base_url={media_base_url}");

    // ── Set up local MPD renderer ──────────────────────────────────────────────
    let renderer = Arc::new(MpdRenderer::new(
        mpd_host.clone(),
        mpd_port,
        media_base_url,
    ));

    // ── Set up local PlaybackState for MpdStateSync ────────────────────────────
    let local_state: Arc<RwLock<PlaybackState>> = Arc::new(RwLock::new(PlaybackState {
        nodes: vec![Node {
            id: node_id.clone(),
            name: node_name.clone(),
            output_ids: vec![node_id.clone()],
            queue: Vec::new(),
            current_index: None,
            status: PlaybackStatus::Stopped,
            position_secs: 0.0,
            volume: 50,
            shuffle: false,
            repeat: kanade_core::model::RepeatMode::Off,
        }],
    }));

    // Channel through which the broadcaster sends serialised NodeStateUpdate JSON
    let (state_tx, mut state_rx) = mpsc::channel::<String>(64);

    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NodeEventBroadcaster { tx: state_tx });

    let queue_generation = Arc::new(AtomicU64::new(0));

    let mut mpd_sync = MpdStateSync::new(
        mpd_host.clone(),
        mpd_port,
        MpdClient::new(mpd_host, mpd_port),
        Arc::clone(&local_state),
        vec![Arc::clone(&broadcaster)],
        Arc::clone(&queue_generation),
    );

    // Spawn MpdStateSync background task
    tokio::spawn(async move {
        mpd_sync.run().await;
    });

    // ── Main loop ─────────────────────────────────────────────────────────────
    loop {
        tokio::select! {
            // Incoming NodeCommand from server
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<NodeCommand>(&text) {
                            Ok(cmd) => {
                                execute_command(cmd, &renderer, &queue_generation).await;
                            }
                            Err(e) => warn!("Unexpected message from server: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("Server disconnected");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        error!("WebSocket error: {e}");
                        break;
                    }
                }
            }
            // Outgoing NodeStateUpdate to server
            Some(json) = state_rx.recv() => {
                if ws_tx.send(Message::Text(json)).await.is_err() {
                    error!("Failed to send state update to server");
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Execute a [`NodeCommand`] against the local [`MpdRenderer`].
async fn execute_command(
    cmd: NodeCommand,
    renderer: &Arc<MpdRenderer>,
    queue_generation: &Arc<AtomicU64>,
) {
    let result = match cmd {
        NodeCommand::Play => renderer.play().await,
        NodeCommand::Pause => renderer.pause().await,
        NodeCommand::Stop => renderer.stop().await,
        NodeCommand::Seek { position_secs } => renderer.seek(position_secs).await,
        NodeCommand::SetVolume { volume } => renderer.set_volume(volume).await,
        NodeCommand::SetQueue { file_paths } => {
            queue_generation.fetch_add(1, Ordering::Relaxed);
            renderer.set_queue(&file_paths).await
        }
        NodeCommand::Add { file_paths } => renderer.add(&file_paths).await,
        NodeCommand::Remove { index } => renderer.remove(index).await,
        NodeCommand::MoveTrack { from, to } => renderer.move_track(from, to).await,
    };
    if let Err(e) = result {
        warn!("Command execution error: {e}");
    }
}
