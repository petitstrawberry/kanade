use std::{net::SocketAddr, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, instrument, warn};

use kanade_core::controller::CoreController;

use crate::{broadcaster::WsBroadcaster, command::WsCommand};

/// The WebSocket server — listens for client connections, parses incoming
/// JSON commands, and forwards the decoded commands to the [`CoreController`].
pub struct WsServer {
    controller: Arc<CoreController>,
    broadcaster: Arc<WsBroadcaster>,
    addr: SocketAddr,
}

impl WsServer {
    pub fn new(
        controller: Arc<CoreController>,
        broadcaster: Arc<WsBroadcaster>,
        addr: SocketAddr,
    ) -> Self {
        Self { controller, broadcaster, addr }
    }

    /// Start listening.  This runs indefinitely and should be spawned as a
    /// Tokio task.
    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("WsServer: failed to bind");
        info!("WebSocket server listening on {}", self.addr);

        let controller = self.controller;
        let broadcaster = self.broadcaster;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let ctrl = Arc::clone(&controller);
                    let rx = broadcaster.subscribe();
                    tokio::spawn(handle_connection(stream, peer, ctrl, rx));
                }
                Err(e) => {
                    error!("WsServer: accept error: {e}");
                }
            }
        }
    }
}

#[instrument(skip(stream, controller, state_rx))]
async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    controller: Arc<CoreController>,
    mut state_rx: tokio::sync::broadcast::Receiver<String>,
) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("WS handshake failed for {peer}: {e}");
            return;
        }
    };
    info!("WebSocket client connected: {peer}");

    let (mut ws_tx, mut ws_rx) = ws.split();

    loop {
        tokio::select! {
            // Inbound: command from the client
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WsCommand>(&text) {
                            Ok(cmd) => dispatch(cmd, &controller).await,
                            Err(e) => warn!("WS bad command from {peer}: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client disconnected: {peer}");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        warn!("WS error from {peer}: {e}");
                        break;
                    }
                }
            }
            // Outbound: push new state JSON to the client
            result = state_rx.recv() => {
                match result {
                    Ok(json) => {
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WS broadcaster lagged by {n} messages for {peer}");
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

/// Translate a parsed [`WsCommand`] into a [`CoreController`] call.
async fn dispatch(cmd: WsCommand, controller: &CoreController) {
    let result = match cmd {
        WsCommand::Play => controller.play().await,
        WsCommand::Pause => controller.pause().await,
        WsCommand::Stop => controller.stop().await,
        WsCommand::Next => controller.next().await,
        WsCommand::Previous => controller.previous().await,
        WsCommand::Seek { position_secs } => controller.seek(position_secs).await,
        WsCommand::SetVolume { volume } => controller.set_volume(volume).await,
        WsCommand::SetQueue { tracks, start_index } => {
            controller.set_queue(tracks, start_index).await
        }
        WsCommand::SetRepeat { repeat } => controller.set_repeat(repeat).await,
        WsCommand::SetShuffle { shuffle } => controller.set_shuffle(shuffle).await,
    };
    if let Err(e) = result {
        debug!("WS dispatch error: {e}");
    }
}
