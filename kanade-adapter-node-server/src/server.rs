use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use kanade_core::{controller::Core, model::Node};
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeRegistrationAck, NodeStateUpdate};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::output::RemoteNodeOutput;

const RECONNECT_GRACE_MS: u64 = 5000;

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

    let registration: NodeRegistration =
        match tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match ws_rx.next().await {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                        Ok(reg) => break reg,
                        Err(e) => {
                            warn!("Node {peer}: malformed registration message: {e}");
                            return None;
                        }
                    },
                    Some(Ok(Message::Close(_))) | None => {
                        info!("Node {peer}: disconnected before registering");
                        return None;
                    }
                    _ => continue,
                }
            }
        })
        .await
        {
            Ok(Some(registration)) => registration,
            Ok(None) => return,
            Err(_) => {
                warn!("Node {peer}: registration timed out");
                return;
            }
        };

    let display_name = registration
        .display_name
        .clone()
        .or(registration.name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("node-{}", Uuid::new_v4()));

    let node_id = registration
        .node_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| display_name.clone());

    info!("Output node registered: {node_id}");

    let ack = NodeRegistrationAck {
        node_id: node_id.clone(),
        media_base_url,
        media_auth_key: None,
        media_auth_key_id: None,
    };
    if ws_tx
        .send(Message::Text(
            serde_json::to_string(&ack).expect("ack serializable"),
        ))
        .await
        .is_err()
    {
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<NodeCommand>(64);
    let connection_id = Uuid::new_v4().to_string();
    let output: Arc<dyn kanade_core::ports::AudioOutput> = Arc::new(RemoteNodeOutput::new(cmd_tx));

    core.register_output(node_id.clone(), connection_id.clone(), Arc::clone(&output))
        .await;

    core.add_node(Node {
        id: node_id.clone(),
        name: display_name,
        ..Default::default()
    })
    .await;

    if let Err(e) = core.sync_connected_node_to_logical_state(&node_id).await {
        warn!("Node {peer}: failed to restore logical node state: {e}");
    }

    let selected_node_id = {
        let state = core.state_handle();
        let selected = state.read().await.selected_node_id.clone();
        selected
    };
    if let Some(selected_node) = selected_node_id {
        if selected_node != node_id {
            if let Err(e) = core.stop_node(&node_id).await {
                warn!("Node {peer}: failed to stop non-active output: {e}");
            }
        }
    }

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    ping_interval.tick().await;
    let mut last_seen = std::time::Instant::now();

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if last_seen.elapsed() > Duration::from_secs(90) {
                    warn!("Node {peer}: heartbeat timed out");
                    break;
                }
                match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Ping(vec![]))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        warn!("Node {peer}: ping send timed out");
                        break;
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        info!(node_id = %node_id, command = ?cmd, "node-server: forwarding command to node");
                        let json = match serde_json::to_string(&cmd) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!("Node {peer}: failed to serialize command: {e}");
                                continue;
                            }
                        };
                        match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Text(json))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => break,
                            Err(_) => {
                                warn!("Node {peer}: command send timed out");
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = std::time::Instant::now();
                        match serde_json::from_str::<NodeStateUpdate>(&text) {
                            Ok(update) => {
                                if !core.is_same_output(&node_id, &connection_id).await {
                                    warn!("Node {peer}: ignoring stale state update for {node_id}");
                                    continue;
                                }
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
                    Some(Ok(Message::Pong(_))) => {
                        last_seen = std::time::Instant::now();
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        last_seen = std::time::Instant::now();
                        match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Pong(payload))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => break,
                            Err(_) => break,
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        last_seen = std::time::Instant::now();
                    }
                    Some(Err(e)) => {
                        warn!("Node {peer}: WS error: {e}");
                        break;
                    }
                }
            }
        }
    }

    info!("Output node disconnected: {}", node_id);
    tokio::time::sleep(Duration::from_millis(RECONNECT_GRACE_MS)).await;
    if core.is_same_output(&node_id, &connection_id).await {
        core.unregister_output(&node_id, &connection_id).await;
        core.handle_node_disconnected(&node_id).await;
    }
}
