use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
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
                    Some(Ok(_)) => {}
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
        WsCommand::Play { zone_id } => core.play_zone(&zone_id).await,
        WsCommand::Pause { zone_id } => core.pause_zone(&zone_id).await,
        WsCommand::Stop { zone_id } => core.stop_zone(&zone_id).await,
        WsCommand::Next { zone_id } => core.next_zone(&zone_id).await,
        WsCommand::Previous { zone_id } => core.previous_zone(&zone_id).await,
        WsCommand::Seek { zone_id, position_secs } => core.seek_zone(&zone_id, position_secs).await,
        WsCommand::SetVolume { zone_id, volume } => core.set_zone_volume(&zone_id, volume).await,
        WsCommand::SetRepeat { zone_id, repeat } => core.set_zone_repeat(&zone_id, repeat).await,
        WsCommand::SetShuffle { zone_id, shuffle } => core.set_zone_shuffle(&zone_id, shuffle).await,
        WsCommand::AddToQueue { zone_id, track } => core.add_to_zone_queue(&zone_id, track).await,
        WsCommand::AddTracksToQueue { zone_id, tracks } => core.add_tracks_to_zone_queue(&zone_id, tracks).await,
        WsCommand::PlayIndex { zone_id, index } => core.play_zone_index(&zone_id, index).await,
        WsCommand::RemoveFromQueue { zone_id, index } => core.remove_from_zone_queue(&zone_id, index).await,
        WsCommand::ClearQueue { zone_id } => core.clear_zone_queue(&zone_id).await,
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
        WsRequest::GetQueue { zone_id } => {
            let state = core.state_handle();
            let s = state.read().await;
            match s.zone(&zone_id) {
                Some(zone) => WsResponse::Queue {
                    tracks: zone.queue.clone(),
                    current_index: zone.current_index,
                },
                None => WsResponse::Queue {
                    tracks: vec![],
                    current_index: None,
                },
            }
        }
    }
}
