use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use kanade_adapter_ws::command::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

pub async fn connect(
    addr: &str,
) -> Result<(
    mpsc::Receiver<ServerMessage>,
    mpsc::Sender<ClientMessage>,
)> {
    info!("Connecting to Kanade server at {addr} …");
    let (ws_stream, _) = connect_async(addr).await?;
    info!("Connected to Kanade server");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let (in_tx, in_rx) = mpsc::channel::<ServerMessage>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<ClientMessage>(64);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<ServerMessage>(&text) {
                                Ok(server_msg) => {
                                    if in_tx.send(server_msg).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse server message: {e}");
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            error!("Server disconnected");
                            break;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            error!("WS error: {e}");
                            break;
                        }
                    }
                }
                Some(cmd) = out_rx.recv() => {
                    let json = match serde_json::to_string(&cmd) {
                        Ok(j) => j,
                        Err(e) => {
                            warn!("Failed to serialize command: {e}");
                            continue;
                        }
                    };
                    if ws_tx.send(Message::Text(json)).await.is_err() {
                        error!("Failed to send command to server");
                        break;
                    }
                }
            }
        }
    });

    Ok((in_rx, out_tx))
}
