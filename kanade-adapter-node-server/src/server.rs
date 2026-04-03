use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use kanade_core::{
    controller::Core,
    model::{Node, PlaybackStatus, RepeatMode},
    ports::AudioOutput,
};
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeRegistrationAck, NodeStateUpdate};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::output::RemoteNodeOutput;

/// Listens for WebSocket connections from output nodes and wires each
/// connected node into the [`Core`] as a live [`AudioOutput`].
///
/// Bind address is controlled by the `NODE_ADDR` environment variable
/// (default `0.0.0.0:8082`).
pub struct NodeServer {
    core: Arc<Core>,
    addr: SocketAddr,
    media_base_url: String,
    restored_states: Arc<RwLock<HashMap<String, RestoredNodeState>>>,
}

#[derive(Debug, Clone)]
pub struct RestoredNodeState {
    pub queue: Vec<kanade_core::model::Track>,
    pub current_index: Option<usize>,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl NodeServer {
    pub fn new(
        core: Arc<Core>,
        addr: SocketAddr,
        media_base_url: impl Into<String>,
        restored_states: Arc<RwLock<HashMap<String, RestoredNodeState>>>,
    ) -> Self {
        Self {
            core,
            addr,
            media_base_url: media_base_url.into(),
            restored_states,
        }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("NodeServer: failed to bind");
        info!("Node server listening on {}", self.addr);

        let core = self.core;
        let media_base_url = self.media_base_url;
        let restored_states = self.restored_states;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let core = Arc::clone(&core);
                    let media_base_url = media_base_url.clone();
                    let restored_states = Arc::clone(&restored_states);
                    tokio::spawn(handle_node_connection(
                        stream,
                        peer,
                        core,
                        media_base_url,
                        restored_states,
                    ));
                }
                Err(e) => {
                    error!("NodeServer: accept error: {e}");
                }
            }
        }
    }
}

async fn handle_node_connection(
    stream: TcpStream,
    peer: SocketAddr,
    core: Arc<Core>,
    media_base_url: String,
    restored_states: Arc<RwLock<HashMap<String, RestoredNodeState>>>,
) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("Node WS handshake failed for {peer}: {e}");
            return;
        }
    };
    info!("Output node connected from {peer}");

    let (mut ws_tx, mut ws_rx) = ws.split();

    // ── Handshake: wait for NodeRegistration ─────────────────────────────────
    let registration: NodeRegistration = loop {
        match ws_rx.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                Ok(reg) => break reg,
                Err(e) => {
                    warn!("Node {peer}: malformed registration message: {e}");
                    return;
                }
            },
            Some(Ok(Message::Close(_))) | None => {
                info!("Node {peer}: disconnected before registering");
                return;
            }
            _ => continue,
        }
    };

    let node_id = registration.name.clone();

    core.unregister_output(&node_id).await;
    core.remove_node(&node_id).await;

    let restored = restored_states.write().await.remove(&node_id);

    info!("Output node registered: {node_id}");

    let ack = NodeRegistrationAck {
        node_id: node_id.clone(),
        media_base_url,
    };
    if ws_tx.send(Message::Text(serde_json::to_string(&ack).expect("ack serializable"))).await.is_err() {
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<NodeCommand>(64);
    let output: Arc<dyn AudioOutput> = Arc::new(RemoteNodeOutput::new(cmd_tx));

    core.register_output(node_id.clone(), Arc::clone(&output)).await;

    let (queue, current_index, volume, shuffle, repeat) = if let Some(restored) = restored {
        (
            restored.queue,
            restored.current_index,
            restored.volume,
            restored.shuffle,
            restored.repeat,
        )
    } else {
        (Vec::new(), None, 50, false, RepeatMode::Off)
    };

    core.add_node(Node {
        id: node_id.clone(),
        name: registration.name.clone(),
        output_ids: vec![node_id.clone()],
        queue,
        current_index,
        status: PlaybackStatus::Stopped,
        position_secs: 0.0,
        volume,
        shuffle,
        repeat,
    })
    .await;

    if let Err(e) = core.restore_node_output_state(&node_id).await {
        warn!("Node {peer}: failed to restore output state: {e}");
    }

    // ── Main loop: relay commands → node, state updates ← node ───────────────
    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let json = match serde_json::to_string(&cmd) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!("Node {peer}: failed to serialize command: {e}");
                                continue;
                            }
                        };
                        if ws_tx.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<NodeStateUpdate>(&text) {
                            Ok(update) => {
                                core.sync_node_state(
                                    &node_id,
                                    update.status,
                                    update.position_secs,
                                    update.volume,
                                    update.mpd_song_index,
                                    update.projection_generation,
                                )
                                .await;
                            }
                            Err(e) => warn!("Node {peer}: bad state update: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        warn!("Node {peer}: WS error: {e}");
                        break;
                    }
                }
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────
    info!("Output node disconnected: {}", node_id);
    core.unregister_output(&node_id).await;
    core.remove_node(&node_id).await;
}
