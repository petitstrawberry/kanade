use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::{Duration, Instant}};

use futures_util::{SinkExt, StreamExt};
use kanade_db::Database;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, instrument, warn};

use kanade_core::controller::Core;

use crate::{
    broadcaster::WsBroadcaster,
    command::{ClientMessage, ServerMessage, WsCommand, WsRequest, WsResponse},
};

pub struct WsServer {
    core: Arc<Core>,
    db_path: PathBuf,
    broadcaster: Arc<WsBroadcaster>,
    addr: SocketAddr,
}

impl WsServer {
    pub fn new(
        core: Arc<Core>,
        db_path: PathBuf,
        broadcaster: Arc<WsBroadcaster>,
        addr: SocketAddr,
    ) -> Self {
        Self { core, db_path, broadcaster, addr }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("WsServer: failed to bind");
        info!("WebSocket server listening on {}", self.addr);

        let core = self.core;
        let db_path = self.db_path;
        let broadcaster = self.broadcaster;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let ctrl = Arc::clone(&core);
                    let db_path = db_path.clone();
                    let rx = broadcaster.subscribe();
                    tokio::spawn(handle_connection(stream, peer, ctrl, db_path, rx));
                }
                Err(e) => {
                    error!("WsServer: accept error: {e}");
                }
            }
        }
    }
}

#[instrument(skip(stream, core, db_path, state_rx))]
async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    core: Arc<Core>,
    db_path: PathBuf,
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

    // Send current state snapshot immediately so the client
    // has nodes, queue, and active output without waiting for
    // the next state change broadcast.
    let snapshot = core.state_handle().read().await.clone();
    let node_summary = snapshot
        .nodes
        .iter()
        .map(|n| format!("{}:{}:{}", n.id, n.name, n.connected))
        .collect::<Vec<_>>()
        .join(", ");
    info!(peer = %peer, selected_node_id = ?snapshot.selected_node_id, nodes = %node_summary, "ws initial snapshot");
    if let Ok(json) = serde_json::to_string(&ServerMessage::State { state: snapshot }) {
        if ws_tx.send(Message::Text(json)).await.is_err() {
            return;
        }
    }

    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await;
    let mut last_seen = Instant::now();

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if last_seen.elapsed() > Duration::from_secs(90) {
                    warn!("WS client heartbeat timed out: {peer}");
                    break;
                }
                match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Ping(vec![]))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        warn!("WS ping send timed out for {peer}");
                        break;
                    }
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = Instant::now();
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Command(cmd)) => {
                                dispatch_command(cmd, &core).await;
                            }
                            Ok(ClientMessage::Request { req_id, req }) => {
                                info!("WS request from {peer}: {:?}", req);
                                let resp = handle_request(req, &core, &db_path).await;
                                let msg = ServerMessage::Response { req_id, data: resp };
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if ws_tx.send(Message::Text(json)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(e) => warn!("WS bad message from {peer}: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client disconnected: {peer}");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        last_seen = Instant::now();
                        match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Pong(payload))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => break,
                            Err(_) => break,
                        }
                    }
                    Some(Ok(_)) => {
                        last_seen = Instant::now();
                    }
                    Some(Err(e)) => {
                        warn!("WS error from {peer}: {e}");
                        break;
                    }
                }
            }
            result = state_rx.recv() => {
                match result {
                    Ok(json) => {
                        if ws_tx.send(Message::Text(json)).await.is_err() {
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

async fn dispatch_command(cmd: WsCommand, core: &Core) {
    info!("WS command: {:?}", cmd);
    let result = match cmd {
        WsCommand::Play => core.play().await,
        WsCommand::Pause => core.pause().await,
        WsCommand::Stop => core.stop().await,
        WsCommand::Next => core.next().await,
        WsCommand::Previous => core.previous().await,
        WsCommand::Seek { position_secs } => core.seek(position_secs).await,
        WsCommand::SetVolume { volume } => core.set_volume(volume).await,
        WsCommand::SetRepeat { repeat } => core.set_repeat(repeat).await,
        WsCommand::SetShuffle { shuffle } => core.set_shuffle(shuffle).await,
        WsCommand::SelectNode { node_id } => core.select_node(&node_id).await,
        WsCommand::AddToQueue { track } => core.add_to_queue(track).await,
        WsCommand::AddTracksToQueue { tracks } => core.add_tracks_to_queue(tracks).await,
        WsCommand::PlayIndex { index } => core.play_index(index).await,
        WsCommand::RemoveFromQueue { index } => core.remove_from_queue(index).await,
        WsCommand::MoveInQueue { from, to } => core.move_in_queue(from, to).await,
        WsCommand::ClearQueue => core.clear_queue().await,
        WsCommand::ReplaceAndPlay { tracks, index } => {
            core.set_queue(tracks, Some(index)).await
        }
    };
    if let Err(e) = result {
        warn!("WS dispatch error: {e}");
    }
}

async fn handle_request(
    req: WsRequest,
    core: &Core,
    db_path: &PathBuf,
) -> WsResponse {
    match req {
        WsRequest::GetAlbums => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_albums().ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::Albums { albums }
        }
        WsRequest::GetAlbumTracks { album_id } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_album_id(&album_id).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::AlbumTracks { tracks }
        }
        WsRequest::GetArtists => {
            let path = db_path.clone();
            let artists = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_artists().ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::Artists { artists }
        }
        WsRequest::GetArtistAlbums { artist } => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_albums_by_artist(&artist).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::ArtistAlbums { albums }
        }
        WsRequest::GetArtistTracks { artist } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_artist(&artist).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::ArtistTracks { tracks }
        }
        WsRequest::GetGenres => {
            let path = db_path.clone();
            let genres = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_genres().ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::Genres { genres }
        }
        WsRequest::GetGenreAlbums { genre } => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_albums_by_genre(&genre).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::GenreAlbums { albums }
        }
        WsRequest::GetGenreTracks { genre } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_genre(&genre).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::GenreTracks { tracks }
        }
        WsRequest::Search { query } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.search_tracks(&query).ok()
            }).await.unwrap_or(None).unwrap_or_default();
            WsResponse::SearchResults { tracks }
        }
        WsRequest::GetQueue => {
            let state = core.state_handle();
            let s = state.read().await;
            WsResponse::Queue {
                tracks: s.queue.clone(),
                current_index: s.current_index,
            }
        }
    }
}
