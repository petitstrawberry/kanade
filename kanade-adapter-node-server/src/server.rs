use std::{net::SocketAddr, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use kanade_core::{
    controller::Core,
    model::{PlaybackStatus, RepeatMode, Zone},
    ports::AudioOutput,
};
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeRegistrationAck, NodeStateUpdate};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;

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
}

impl NodeServer {
    pub fn new(core: Arc<Core>, addr: SocketAddr, media_base_url: impl Into<String>) -> Self {
        Self {
            core,
            addr,
            media_base_url: media_base_url.into(),
        }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("NodeServer: failed to bind");
        info!("Node server listening on {}", self.addr);

        let core = self.core;
        let media_base_url = self.media_base_url;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let core = Arc::clone(&core);
                    let media_base_url = media_base_url.clone();
                    tokio::spawn(handle_node_connection(stream, peer, core, media_base_url));
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

    // ── Assign a server-generated UUID as the node (= zone) identifier ───────
    let node_id = Uuid::new_v4().to_string();

    info!(
        "Output node registered: name={}, assigned id={}",
        registration.name, node_id
    );

    // Send registration acknowledgement with the server-assigned node_id
    let ack = NodeRegistrationAck {
        node_id: node_id.clone(),
        media_base_url,
    };
    match serde_json::to_string(&ack) {
        Ok(json) => {
            if ws_tx.send(Message::Text(json)).await.is_err() {
                return;
            }
        }
        Err(e) => {
            error!("Node {peer}: failed to serialize ack: {e}");
            return;
        }
    }

    // ── Set up RemoteNodeOutput ───────────────────────────────────────────────
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<NodeCommand>(64);
    let output: Arc<dyn AudioOutput> = Arc::new(RemoteNodeOutput::new(cmd_tx));

    // Register output and create a zone for this node
    core.register_output(node_id.clone(), Arc::clone(&output)).await;
    core.add_zone(Zone {
        id: node_id.clone(),
        name: registration.name.clone(),
        output_ids: vec![node_id.clone()],
        queue: Vec::new(),
        current_index: None,
        status: PlaybackStatus::Stopped,
        position_secs: 0.0,
        volume: 50,
        shuffle: false,
        repeat: RepeatMode::Off,
    })
    .await;

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
                                core.sync_zone_state(
                                    &node_id,
                                    update.status,
                                    update.position_secs,
                                    update.volume,
                                    update.current_index,
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
    core.remove_zone(&node_id).await;
}
