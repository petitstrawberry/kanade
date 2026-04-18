use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, State,
    },
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode},
    response::IntoResponse,
    routing::{any, get},
    Router,
};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use kanade_adapter_node_server::RemoteNodeOutput;
use kanade_core::{controller::Core, model::Node};
use kanade_db::Database;
use kanade_node_protocol::NodeRegistration;
use lofty::{prelude::*, probe::Probe};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::mpsc,
};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::{
    broadcaster::WsBroadcaster,
    command::{ClientMessage, ServerMessage, WsCommand, WsNodeCommand, WsRequest, WsResponse},
};

const RECONNECT_GRACE_MS: u64 = 5000;
const READ_CHUNK_SIZE: usize = 64 * 1024;

pub struct AppState {
    pub core: Arc<Core>,
    pub db_path: PathBuf,
    pub broadcaster: Arc<WsBroadcaster>,
    pub media_base_url: String,
}

enum ArtResult {
    FilePath(String),
    Embedded(String, Vec<u8>),
}

type WsSink = SplitSink<WebSocket, Message>;
type WsStream = SplitStream<WebSocket>;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", any(ws_handler))
        .route("/media/tracks/{track_id}", get(media_track_handler))
        .route("/media/art/{album_id}", get(media_art_handler))
        .route("/media/file/{*path}", get(media_file_handler))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
                .allow_headers([header::RANGE]),
        )
        .with_state(state)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state, peer))
}

#[instrument(skip(socket, state))]
async fn handle_ws_connection(socket: WebSocket, state: Arc<AppState>, peer: SocketAddr) {
    info!(peer = %peer, "WebSocket client connected");

    let (mut ws_tx, mut ws_rx) = socket.split();

    let first_message = match tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws_rx.next().await {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text)
                {
                    Ok(msg) => break Some(msg),
                    Err(e) => {
                        warn!(peer = %peer, error = %e, "WS bad first message");
                        return None;
                    }
                },
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(Message::Close(_))) | None => return None,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    warn!(peer = %peer, error = %e, "WS error before mode selection");
                    return None;
                }
            }
        }
    })
    .await
    {
        Ok(Some(msg)) => msg,
        Ok(None) => return,
        Err(_) => {
            warn!(peer = %peer, "WS mode selection timed out");
            return;
        }
    };

    match first_message {
        ClientMessage::NodeRegistration(registration) => {
            run_node_mode(peer, &state, &mut ws_tx, &mut ws_rx, registration).await;
        }
        ClientMessage::Command(cmd) => {
            run_ui_mode(
                peer,
                &state,
                &mut ws_tx,
                &mut ws_rx,
                Some(ClientMessage::Command(cmd)),
            )
            .await;
        }
        ClientMessage::Request { req_id, req } => {
            run_ui_mode(
                peer,
                &state,
                &mut ws_tx,
                &mut ws_rx,
                Some(ClientMessage::Request { req_id, req }),
            )
            .await;
        }
        ClientMessage::NodeStateUpdate(_) => {
            warn!(peer = %peer, "WS node state update before registration");
        }
    }
}

#[instrument(skip(state, headers))]
async fn media_track_handler(
    State(state): State<Arc<AppState>>,
    Path(track_id): Path<String>,
    headers: HeaderMap,
    method: Method,
) -> Response<Body> {
    let track_id = track_id.split('?').next().unwrap_or(&track_id).to_string();
    let db_path = state.db_path.clone();

    let track = match tokio::task::spawn_blocking(move || {
        let db = Database::open(&db_path).ok()?;
        db.get_track_by_id(&track_id).ok()?
    })
    .await
    {
        Ok(Some(track)) => track,
        Ok(None) => return simple_response(StatusCode::NOT_FOUND, "Track Not Found"),
        Err(e) => {
            warn!(error = %e, "track lookup join error");
            return simple_response(StatusCode::INTERNAL_SERVER_ERROR, "db error");
        }
    };

    serve_path_with_range(
        &method,
        &track.file_path,
        content_type_for_path(&track.file_path),
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        None,
    )
    .await
}

#[instrument(skip(state))]
async fn media_art_handler(
    State(state): State<Arc<AppState>>,
    Path(album_id): Path<String>,
    method: Method,
) -> Response<Body> {
    let album_id = album_id.split('?').next().unwrap_or(&album_id).to_string();
    info!(album_id = %album_id, "artwork request");

    let db_path = state.db_path.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<ArtResult, String> {
        let db = Database::open(&db_path).map_err(|e| e.to_string())?;
        let art_path = db.get_album_artwork_path(&album_id).map_err(|e| e.to_string())?;

        if let Some(ref path) = art_path {
            if std::path::Path::new(path).exists() {
                return Ok(ArtResult::FilePath(path.clone()));
            }
            warn!(path = %path, "artwork_path exists in DB but file missing, falling back to embedded");
        }

        let tracks = db.get_tracks_by_album_id(&album_id).map_err(|e| e.to_string())?;
        if let Some(track) = tracks.into_iter().next() {
            if let Some(picture) = extract_embedded_picture(&track.file_path) {
                let mime = picture
                    .mime_type()
                    .map(|m| m.as_str())
                    .unwrap_or("image/jpeg")
                    .to_string();
                let data = picture.data().to_vec();
                return Ok(ArtResult::Embedded(mime, data));
            }
        }

        Err("no artwork found".to_string())
    })
    .await;

    match result {
        Ok(Ok(ArtResult::FilePath(art_path))) => {
            info!(path = %art_path, "serving artwork from file");
            serve_path_with_range(
                &method,
                &art_path,
                content_type_for_path(&art_path),
                None,
                Some("public, max-age=86400"),
            )
            .await
        }
        Ok(Ok(ArtResult::Embedded(mime, data))) => {
            info!(mime = %mime, size = data.len(), "serving embedded artwork");
            bytes_response(
                StatusCode::OK,
                &method,
                &mime,
                Some("public, max-age=86400"),
                data,
            )
        }
        Ok(Err(e)) => {
            warn!(error = %e, "artwork not found");
            simple_response(StatusCode::NOT_FOUND, "Artwork Not Found")
        }
        Err(e) => {
            warn!(error = %e, "artwork lookup join error");
            simple_response(StatusCode::INTERNAL_SERVER_ERROR, "db error")
        }
    }
}

#[instrument(skip(_state, headers))]
async fn media_file_handler(
    State(_state): State<Arc<AppState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
    method: Method,
) -> Response<Body> {
    let decoded = percent_decode(&path);
    if decoded.is_empty() || decoded.contains("..") {
        return simple_response(StatusCode::BAD_REQUEST, "Bad Request");
    }

    serve_path_with_range(
        &method,
        &decoded,
        content_type_for_path(&decoded),
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        None,
    )
    .await
}

async fn run_ui_mode(
    peer: SocketAddr,
    state: &Arc<AppState>,
    ws_tx: &mut WsSink,
    ws_rx: &mut WsStream,
    first_message: Option<ClientMessage>,
) {
    let mut state_rx = state.broadcaster.subscribe();
    let mut local_node_id: Option<String> = None;
    let snapshot = state.core.state_handle().read().await.clone();
    let node_summary = snapshot
        .nodes
        .iter()
        .map(|n| format!("{}:{}:{}", n.id, n.name, n.connected))
        .collect::<Vec<_>>()
        .join(", ");

    info!(peer = %peer, selected_node_id = ?snapshot.selected_node_id, nodes = %node_summary, "ws initial snapshot");
    if let Ok(json) = serde_json::to_string(&ServerMessage::State { state: snapshot }) {
        if ws_tx.send(Message::Text(json.into())).await.is_err() {
            return;
        }
    }

    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await;
    let mut last_seen = Instant::now();

    if let Some(msg) = first_message {
        handle_ui_client_message(msg, peer, state, ws_tx, &mut local_node_id).await;
    }

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if last_seen.elapsed() > Duration::from_secs(90) {
                    warn!(peer = %peer, "WS client heartbeat timed out");
                    break;
                }

                match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Ping(Bytes::new()))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        warn!(peer = %peer, "WS ping send timed out");
                        break;
                    }
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = Instant::now();
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(msg) => handle_ui_client_message(msg, peer, state, ws_tx, &mut local_node_id).await,
                            Err(e) => warn!(peer = %peer, error = %e, "WS bad message"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(peer = %peer, "WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Ping(_))) | Some(Ok(Message::Binary(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Err(e)) => {
                        warn!(peer = %peer, error = %e, "WS error");
                        break;
                    }
                }
            }
            result = state_rx.recv() => {
                match result {
                    Ok(json) => {
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(peer = %peer, lagged = n, "WS broadcaster lagged");
                    }
                    Err(_) => break,
                }
            }
        }
    }

    if let Some(nid) = local_node_id {
        let _ = state.core.local_session_stop(&nid).await;
    }
}

async fn handle_ui_client_message(
    msg: ClientMessage,
    peer: SocketAddr,
    state: &Arc<AppState>,
    ws_tx: &mut WsSink,
    local_node_id: &mut Option<String>,
) {
    match msg {
        ClientMessage::Command(cmd) => {
            dispatch_command(cmd, &state.core, local_node_id).await;
        }
        ClientMessage::Request { req_id, req } => {
            info!(peer = %peer, request = ?req, "WS request");
            let resp = handle_request(req, &state.core, &state.db_path).await;
            let msg = ServerMessage::Response { req_id, data: resp };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = ws_tx.send(Message::Text(json.into())).await;
            }
        }
        ClientMessage::NodeRegistration(_) | ClientMessage::NodeStateUpdate(_) => {
            warn!(peer = %peer, "WS node message on UI connection");
        }
    }
}

async fn run_node_mode(
    peer: SocketAddr,
    state: &Arc<AppState>,
    ws_tx: &mut WsSink,
    ws_rx: &mut WsStream,
    registration: NodeRegistration,
) {
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

    let ack = ServerMessage::NodeRegistrationAck {
        ack: kanade_node_protocol::NodeRegistrationAck {
            node_id: node_id.clone(),
            media_base_url: state.media_base_url.clone(),
        },
    };
    let ack_json = match serde_json::to_string(&ack) {
        Ok(json) => json,
        Err(e) => {
            warn!(peer = %peer, error = %e, "Node failed to serialize ack");
            return;
        }
    };
    if ws_tx.send(Message::Text(ack_json.into())).await.is_err() {
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WsNodeCommand>(64);
    let connection_id = Uuid::new_v4().to_string();
    let output: Arc<dyn kanade_core::ports::AudioOutput> = Arc::new(RemoteNodeOutput::new(cmd_tx));

    state
        .core
        .register_output(node_id.clone(), connection_id.clone(), Arc::clone(&output))
        .await;

    state
        .core
        .add_node(Node {
            id: node_id.clone(),
            name: display_name,
            ..Default::default()
        })
        .await;

    if let Err(e) = state
        .core
        .sync_connected_node_to_logical_state(&node_id)
        .await
    {
        warn!(peer = %peer, error = %e, "Node failed to restore logical node state");
    }

    let selected_node_id = {
        let state_handle = state.core.state_handle();
        let selected_node_id = state_handle.read().await.selected_node_id.clone();
        selected_node_id
    };
    if let Some(selected_node) = selected_node_id {
        if selected_node != node_id {
            if let Err(e) = state.core.stop_node(&node_id).await {
                warn!(peer = %peer, error = %e, "Node failed to stop non-active output");
            }
        }
    }

    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await;
    let mut last_seen = Instant::now();

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if last_seen.elapsed() > Duration::from_secs(90) {
                    warn!(peer = %peer, node_id = %node_id, "Node heartbeat timed out");
                    break;
                }

                match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Ping(Bytes::new()))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        warn!(peer = %peer, node_id = %node_id, "Node ping send timed out");
                        break;
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let json = match serde_json::to_string(&cmd) {
                            Ok(json) => json,
                            Err(e) => {
                                warn!(peer = %peer, node_id = %node_id, error = %e, "Node failed to serialize command");
                                continue;
                            }
                        };

                        match tokio::time::timeout(Duration::from_secs(5), ws_tx.send(Message::Text(json.into()))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => break,
                            Err(_) => {
                                warn!(peer = %peer, node_id = %node_id, "Node command send timed out");
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
                        last_seen = Instant::now();
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::NodeStateUpdate(update)) => {
                                if !state.core.is_same_output(&node_id, &connection_id).await {
                                    warn!(peer = %peer, node_id = %node_id, "ignoring stale state update");
                                    continue;
                                }
                                state.core.sync_node_state(
                                    &node_id,
                                    update.status,
                                    update.position_secs,
                                    update.volume,
                                    update.mpd_song_index,
                                    update.projection_generation,
                                ).await;
                            }
                            Ok(ClientMessage::NodeRegistration(_)) => {
                                warn!(peer = %peer, node_id = %node_id, "duplicate registration ignored");
                            }
                            Ok(ClientMessage::Command(_) | ClientMessage::Request { .. }) => {
                                warn!(peer = %peer, node_id = %node_id, "received UI message in node mode");
                            }
                            Err(e) => warn!(peer = %peer, node_id = %node_id, error = %e, "bad state update"),
                        }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Ping(_))) | Some(Ok(Message::Binary(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        warn!(peer = %peer, node_id = %node_id, error = %e, "Node WS error");
                        break;
                    }
                }
            }
        }
    }

    info!(node_id = %node_id, "Output node disconnected");
    tokio::time::sleep(Duration::from_millis(RECONNECT_GRACE_MS)).await;
    if state.core.is_same_output(&node_id, &connection_id).await {
        state.core.unregister_output(&node_id, &connection_id).await;
        state.core.handle_node_disconnected(&node_id).await;
    }
}

async fn dispatch_command(cmd: WsCommand, core: &Core, local_node_id: &mut Option<String>) {
    info!(command = ?cmd, "WS command");

    let is_queue_op = matches!(
        &cmd,
        WsCommand::Play
            | WsCommand::Pause
            | WsCommand::Stop
            | WsCommand::Next
            | WsCommand::Previous
            | WsCommand::Seek { .. }
            | WsCommand::SetVolume { .. }
            | WsCommand::SetRepeat { .. }
            | WsCommand::SetShuffle { .. }
            | WsCommand::AddToQueue { .. }
            | WsCommand::AddTracksToQueue { .. }
            | WsCommand::PlayIndex { .. }
            | WsCommand::RemoveFromQueue { .. }
            | WsCommand::MoveInQueue { .. }
            | WsCommand::ClearQueue
            | WsCommand::ReplaceAndPlay { .. }
    );

    if is_queue_op {
        let state = core.state_handle();
        let sel_id = state.read().await.selected_node_id.clone();
        if let Some(ref sel) = sel_id {
            let is_local = state
                .read()
                .await
                .node(sel)
                .map(|n| n.node_type == kanade_core::model::NodeType::Local)
                .unwrap_or(false);
            if is_local && local_node_id.as_deref() != Some(sel.as_str()) {
                warn!(selected_node = %sel, "Rejected command on non-owned local node");
                return;
            }
        }
    }

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
        WsCommand::ReplaceAndPlay { tracks, index } => core.set_queue(tracks, Some(index)).await,
        WsCommand::LocalSessionStart {
            device_name,
            device_id,
        } => match core
            .local_session_start(&device_name, device_id.as_deref())
            .await
        {
            Ok(node_id) => {
                *local_node_id = Some(node_id);
                Ok(())
            }
            Err(e) => Err(e),
        },
        WsCommand::LocalSessionStop => {
            if let Some(ref nid) = *local_node_id {
                if let Err(e) = core.local_session_stop(nid).await {
                    warn!(error = %e, "local_session_stop error");
                }
                *local_node_id = None;
            }
            Ok(())
        }
        WsCommand::LocalSessionUpdate {
            tracks,
            index,
            position_secs,
            status,
            volume,
            repeat,
            shuffle,
        } => {
            if let Some(ref nid) = *local_node_id {
                if let Err(e) = core
                    .local_session_update(
                        nid,
                        tracks,
                        index,
                        position_secs,
                        status,
                        volume,
                        repeat,
                        shuffle,
                    )
                    .await
                {
                    warn!(error = %e, "local_session_update error");
                }
            }
            Ok(())
        }
        WsCommand::Handoff {
            from_node_id,
            to_node_id,
        } => {
            if let Err(e) = core.handoff(&from_node_id, &to_node_id).await {
                warn!(error = %e, "handoff error");
            }
            Ok(())
        }
    };

    if let Err(e) = result {
        warn!(error = %e, "WS dispatch error");
    }
}

async fn handle_request(req: WsRequest, core: &Core, db_path: &PathBuf) -> WsResponse {
    match req {
        WsRequest::GetAlbums => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_albums().ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::Albums { albums }
        }
        WsRequest::GetAlbumTracks { album_id } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_album_id(&album_id).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::AlbumTracks { tracks }
        }
        WsRequest::GetArtists => {
            let path = db_path.clone();
            let artists = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_artists().ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::Artists { artists }
        }
        WsRequest::GetArtistAlbums { artist } => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_albums_by_artist(&artist).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::ArtistAlbums { albums }
        }
        WsRequest::GetArtistTracks { artist } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_artist(&artist).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::ArtistTracks { tracks }
        }
        WsRequest::GetGenres => {
            let path = db_path.clone();
            let genres = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_all_genres().ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::Genres { genres }
        }
        WsRequest::GetGenreAlbums { genre } => {
            let path = db_path.clone();
            let albums = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_albums_by_genre(&genre).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::GenreAlbums { albums }
        }
        WsRequest::GetGenreTracks { genre } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.get_tracks_by_genre(&genre).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::GenreTracks { tracks }
        }
        WsRequest::Search { query } => {
            let path = db_path.clone();
            let tracks = tokio::task::spawn_blocking(move || {
                let db = Database::open(&path).ok()?;
                db.search_tracks(&query).ok()
            })
            .await
            .unwrap_or(None)
            .unwrap_or_default();
            WsResponse::SearchResults { tracks }
        }
        WsRequest::GetQueue => {
            let state = core.state_handle();
            let s = state.read().await;
            let (tracks, idx) = s
                .selected_node()
                .map(|n| (n.queue.clone(), n.current_index))
                .unwrap_or_else(|| (s.queue.clone(), s.current_index));
            WsResponse::Queue {
                tracks,
                current_index: idx,
            }
        }
    }
}

async fn serve_path_with_range(
    method: &Method,
    path: &str,
    content_type: &str,
    range_header: Option<&str>,
    cache_control: Option<&str>,
) -> Response<Body> {
    let mut file = match File::open(path).await {
        Ok(file) => file,
        Err(_) => return simple_response(StatusCode::NOT_FOUND, "Not Found"),
    };

    let metadata = match file.metadata().await {
        Ok(metadata) => metadata,
        Err(e) => {
            warn!(path = %path, error = %e, "failed to read file metadata");
            return simple_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let total_len = metadata.len();

    let (start, end, status) = match parse_range(range_header, total_len) {
        Ok(Some((start, end))) => (start, end, StatusCode::PARTIAL_CONTENT),
        Ok(None) => {
            let end = if total_len == 0 { 0 } else { total_len - 1 };
            (0, end, StatusCode::OK)
        }
        Err(()) => {
            let mut response = body_response(StatusCode::RANGE_NOT_SATISFIABLE, Body::empty());
            insert_header(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes */{total_len}"),
            );
            insert_header(&mut response, header::CONTENT_LENGTH, "0");
            return response;
        }
    };

    let content_length = if total_len == 0 { 0 } else { end - start + 1 };

    let body = if method == Method::HEAD || content_length == 0 {
        Body::empty()
    } else {
        if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
            warn!(path = %path, error = %e, "failed to seek file");
            return simple_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }

        let stream = futures_util::stream::try_unfold(
            (file, content_length),
            |(mut file, remaining)| async move {
                if remaining == 0 {
                    return Ok::<_, std::io::Error>(None);
                }

                let to_read = remaining.min(READ_CHUNK_SIZE as u64) as usize;
                let mut buf = vec![0u8; to_read];
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    return Ok(None);
                }
                buf.truncate(n);
                Ok(Some((Bytes::from(buf), (file, remaining - n as u64))))
            },
        );

        Body::from_stream(stream)
    };

    let mut response = body_response(status, body);
    insert_header(&mut response, header::CONTENT_TYPE, content_type);
    insert_header(&mut response, header::ACCEPT_RANGES, "bytes");
    insert_header(
        &mut response,
        header::CONTENT_LENGTH,
        &content_length.to_string(),
    );
    if status == StatusCode::PARTIAL_CONTENT {
        insert_header(
            &mut response,
            header::CONTENT_RANGE,
            &format!("bytes {}-{}/{}", start, end, total_len),
        );
    }
    if let Some(value) = cache_control {
        insert_header(&mut response, header::CACHE_CONTROL, value);
    }
    response
}

fn bytes_response(
    status: StatusCode,
    method: &Method,
    content_type: &str,
    cache_control: Option<&str>,
    data: Vec<u8>,
) -> Response<Body> {
    let len = data.len();
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(data)
    };
    let mut response = body_response(status, body);
    insert_header(&mut response, header::CONTENT_TYPE, content_type);
    insert_header(&mut response, header::CONTENT_LENGTH, &len.to_string());
    if let Some(value) = cache_control {
        insert_header(&mut response, header::CACHE_CONTROL, value);
    }
    response
}

fn simple_response(status: StatusCode, body: &str) -> Response<Body> {
    let mut response = body_response(status, Body::from(body.to_string()));
    insert_header(
        &mut response,
        header::CONTENT_TYPE,
        "text/plain; charset=utf-8",
    );
    insert_header(
        &mut response,
        header::CONTENT_LENGTH,
        &body.len().to_string(),
    );
    response
}

fn body_response(status: StatusCode, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    insert_header(&mut response, header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    insert_header(&mut response, header::ACCESS_CONTROL_ALLOW_HEADERS, "Range");
    response
}

fn insert_header(response: &mut Response<Body>, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

fn extract_embedded_picture(file_path: &str) -> Option<lofty::picture::Picture> {
    let path_lower = file_path.to_lowercase();
    if path_lower.ends_with(".dsf") {
        return extract_dsf_picture(file_path);
    }

    let tagged_file = Probe::open(file_path).ok()?.read().ok()?;
    let tag = match tagged_file.primary_tag() {
        Some(tag) => tag,
        None => tagged_file.first_tag()?,
    };
    tag.pictures()
        .iter()
        .find(|p| matches!(p.pic_type(), lofty::picture::PictureType::CoverFront))
        .cloned()
        .or_else(|| tag.pictures().first().cloned())
}

fn extract_dsf_picture(file_path: &str) -> Option<lofty::picture::Picture> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(file_path).ok()?;

    file.seek(SeekFrom::Start(20)).ok()?;
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf).ok()?;
    let id3_offset = u64::from_le_bytes(buf);

    file.seek(SeekFrom::Start(id3_offset)).ok()?;
    let mut header = [0u8; 3];
    file.read_exact(&mut header).ok()?;
    if &header != b"ID3" {
        return None;
    }
    file.seek(SeekFrom::Start(id3_offset)).ok()?;

    let id3_data = {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).ok()?;
        buf
    };

    parse_id3v2_apic(&id3_data)
}

fn parse_id3v2_apic(data: &[u8]) -> Option<lofty::picture::Picture> {
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return None;
    }

    let version = data[3];
    let flags = data[5];
    let size = if version == 4 {
        ((data[6] as u64) << 21)
            | ((data[7] as u64) << 14)
            | ((data[8] as u64) << 7)
            | (data[9] as u64)
    } else {
        ((data[6] as u64) << 24)
            | ((data[7] as u64) << 16)
            | ((data[8] as u64) << 8)
            | (data[9] as u64)
    } as usize;

    let has_footer = version == 4 && (flags & 0x10) != 0;
    let total_size = 10 + size + if has_footer { 10 } else { 0 };
    if total_size > data.len() {
        return None;
    }

    let frames_data = &data[10..10 + size];
    find_apic_frame(frames_data, version)
}

fn find_apic_frame(data: &[u8], version: u8) -> Option<lofty::picture::Picture> {
    let mut pos = 0;
    let synchsafe = version == 4;

    while pos + 10 <= data.len() {
        let frame_id = &data[pos..pos + 4];
        if frame_id == [0; 4] {
            break;
        }

        let frame_size = if synchsafe {
            ((data[pos + 4] as usize) << 21)
                | ((data[pos + 5] as usize) << 14)
                | ((data[pos + 6] as usize) << 7)
                | (data[pos + 7] as usize)
        } else {
            ((data[pos + 4] as usize) << 24)
                | ((data[pos + 5] as usize) << 16)
                | ((data[pos + 6] as usize) << 8)
                | (data[pos + 7] as usize)
        };

        let frame_flags: [u8; 2] = data[pos + 8..pos + 10].try_into().ok()?;
        let frame_data = data.get(pos + 10..pos + 10 + frame_size)?;
        let frame_id_str = std::str::from_utf8(frame_id).ok()?;

        if frame_id_str == "APIC" {
            return Some(parse_apic_data(frame_data, version)?);
        }

        let has_header = version == 4 && (frame_flags[1] & 0x01) != 0;
        let skip = if has_header { 4 } else { 0 };
        pos += 10 + frame_size + skip;
    }

    None
}

fn parse_apic_data(data: &[u8], version: u8) -> Option<lofty::picture::Picture> {
    let mut pos = 0;

    let _encoding = data.get(pos)?;
    pos += 1;

    let mime_end = memchr::memchr(0, &data[pos..])?;
    let mime = std::str::from_utf8(&data[pos..pos + mime_end])
        .ok()?
        .to_string();
    pos += mime_end + 1;

    let pic_type = lofty::picture::PictureType::from_u8(data[pos]);
    pos += 1;

    let desc_end = memchr::memchr(0, &data[pos..])?;
    pos += desc_end + 1;

    if version == 1 || version == 2 {
        if pos < data.len() {
            pos += 1;
        }
    }

    let pic_data = data.get(pos..)?;
    if pic_data.is_empty() {
        return None;
    }

    let mime_type = Some(lofty::picture::MimeType::from_str(&mime));

    Some(lofty::picture::Picture::new_unchecked(
        pic_type,
        mime_type,
        None,
        pic_data.to_vec(),
    ))
}

fn parse_range(header: Option<&str>, total_len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(header) = header else {
        return Ok(None);
    };
    if total_len == 0 {
        return Err(());
    }

    let value = header.trim();
    let Some(spec) = value.strip_prefix("bytes=") else {
        return Err(());
    };
    let Some((start_s, end_s)) = spec.split_once('-') else {
        return Err(());
    };

    if start_s.is_empty() {
        let suffix = end_s.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let start = total_len.saturating_sub(suffix.min(total_len));
        return Ok(Some((start, total_len - 1)));
    }

    let start = start_s.parse::<u64>().map_err(|_| ())?;
    if start >= total_len {
        return Err(());
    }

    let end = if end_s.is_empty() {
        total_len - 1
    } else {
        end_s.parse::<u64>().map_err(|_| ())?.min(total_len - 1)
    };
    if start > end {
        return Err(());
    }

    Ok(Some((start, end)))
}

fn content_type_for_path(path: &str) -> &'static str {
    match PathBuf::from(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("flac") => "audio/flac",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") | Some("mp4") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("ogg") => "audio/ogg",
        Some("opus") => "audio/ogg",
        Some("aiff") | Some("aif") => "audio/aiff",
        Some("dsf") => "audio/x-dsf",
        _ => "application/octet-stream",
    }
}

fn percent_decode(input: &str) -> String {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                result.push(byte);
                i += 3;
                continue;
            }
        }

        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(result).unwrap_or_default()
}
