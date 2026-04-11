use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use kanade_adapter_node_server::RemoteNodeOutput;
use kanade_db::Database;
use kanade_node_protocol::NodeRegistration;
use kanade_core::model::Node;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use kanade_core::controller::Core;

use crate::{
    broadcaster::WsBroadcaster,
    command::{
        ClientMessage, ServerMessage, WsCommand, WsNodeCommand, WsRequest, WsResponse,
    },
};

const RECONNECT_GRACE_MS: u64 = 5000;

pub struct WsServer {
    core: Arc<Core>,
    db_path: PathBuf,
    broadcaster: Arc<WsBroadcaster>,
    addr: SocketAddr,
    media_base_url: String,
}

impl WsServer {
    pub fn new(
        core: Arc<Core>,
        db_path: PathBuf,
        broadcaster: Arc<WsBroadcaster>,
        addr: SocketAddr,
        media_base_url: impl Into<String>,
    ) -> Self {
        Self {
            core,
            db_path,
            broadcaster,
            addr,
            media_base_url: media_base_url.into(),
        }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("WsServer: failed to bind");
        info!("WebSocket server listening on {}", self.addr);

        let core = self.core;
        let db_path = self.db_path;
        let broadcaster = self.broadcaster;
        let media_base_url = self.media_base_url;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let ctrl = Arc::clone(&core);
                    let db_path = db_path.clone();
                    let rx = broadcaster.subscribe();
                    let media_base_url = media_base_url.clone();
                    tokio::spawn(handle_connection(
                        stream,
                        peer,
                        ctrl,
                        db_path,
                        rx,
                        media_base_url,
                    ));
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
    media_base_url: String,
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

    let first_message = match tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws_rx.next().await {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(msg) => break Some(msg),
                        Err(e) => {
                            warn!("WS bad first message from {peer}: {e}");
                            return None;
                        }
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if ws_tx.send(Message::Pong(payload)).await.is_err() {
                        return None;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => return None,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    warn!("WS error before mode selection from {peer}: {e}");
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
            warn!("WS mode selection timed out for {peer}");
            return;
        }
    };

    match first_message {
        ClientMessage::NodeRegistration(registration) => {
            run_node_mode(
                peer,
                &core,
                &mut ws_tx,
                &mut ws_rx,
                media_base_url,
                registration,
            )
            .await;
        }
        ClientMessage::Command(cmd) => {
            run_ui_mode(
                peer,
                &core,
                &db_path,
                &mut ws_tx,
                &mut ws_rx,
                &mut state_rx,
                Some(ClientMessage::Command(cmd)),
            )
            .await;
        }
        ClientMessage::Request { req_id, req } => {
            run_ui_mode(
                peer,
                &core,
                &db_path,
                &mut ws_tx,
                &mut ws_rx,
                &mut state_rx,
                Some(ClientMessage::Request { req_id, req }),
            )
            .await;
        }
        ClientMessage::NodeStateUpdate(_) => {
            warn!("WS node state update before registration from {peer}");
        }
    }
}

async fn run_ui_mode(
    peer: SocketAddr,
    core: &Arc<Core>,
    db_path: &PathBuf,
    ws_tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    ws_rx: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>,
    state_rx: &mut tokio::sync::broadcast::Receiver<String>,
    first_message: Option<ClientMessage>,
) {
    let mut local_node_id: Option<String> = None;
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

    if let Some(msg) = first_message {
        handle_ui_client_message(msg, peer, core, db_path, ws_tx, &mut local_node_id).await;
    }

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
                            Ok(msg) => {
                                handle_ui_client_message(msg, peer, core, db_path, ws_tx, &mut local_node_id).await;
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

    if let Some(nid) = local_node_id {
        let _ = core.local_session_stop(&nid).await;
    }
}

async fn handle_ui_client_message(
    msg: ClientMessage,
    peer: SocketAddr,
    core: &Core,
    db_path: &PathBuf,
    ws_tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    local_node_id: &mut Option<String>,
) {
    match msg {
        ClientMessage::Command(cmd) => {
            dispatch_command(cmd, core, local_node_id).await;
        }
        ClientMessage::Request { req_id, req } => {
            info!("WS request from {peer}: {:?}", req);
            let resp = handle_request(req, core, db_path).await;
            let msg = ServerMessage::Response { req_id, data: resp };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = ws_tx.send(Message::Text(json)).await;
            }
        }
        ClientMessage::NodeRegistration(_) | ClientMessage::NodeStateUpdate(_) => {
            warn!("WS node message on UI connection from {peer}");
        }
    }
}

async fn run_node_mode(
    peer: SocketAddr,
    core: &Arc<Core>,
    ws_tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    ws_rx: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>,
    media_base_url: String,
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
            media_base_url,
        },
    };
    let ack_json = match serde_json::to_string(&ack) {
        Ok(json) => json,
        Err(e) => {
            warn!("Node {peer}: failed to serialize ack: {e}");
            return;
        }
    };
    if ws_tx.send(Message::Text(ack_json)).await.is_err() {
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WsNodeCommand>(64);
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

    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await;
    let mut last_seen = Instant::now();

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
                        let json = match serde_json::to_string(&cmd) {
                            Ok(json) => json,
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
                        last_seen = Instant::now();
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::NodeStateUpdate(update)) => {
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
                            Ok(ClientMessage::NodeRegistration(_)) => {
                                warn!("Node {peer}: duplicate registration ignored");
                            }
                            Ok(ClientMessage::Command(_) | ClientMessage::Request { .. }) => {
                                warn!("Node {peer}: received UI message in node mode");
                            }
                            Err(e) => warn!("Node {peer}: bad state update: {e}"),
                        }
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
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        last_seen = Instant::now();
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

async fn dispatch_command(cmd: WsCommand, core: &Core, local_node_id: &mut Option<String>) {
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
        WsCommand::LocalSessionStart { device_name } => match core.local_session_start(&device_name).await {
            Ok(node_id) => {
                *local_node_id = Some(node_id);
                Ok(())
            }
            Err(e) => Err(e),
        },
        WsCommand::LocalSessionStop => {
            if let Some(ref nid) = *local_node_id {
                if let Err(e) = core.local_session_stop(nid).await {
                    warn!("local_session_stop error: {e}");
                }
                *local_node_id = None;
            }
            Ok(())
        }
        WsCommand::LocalSessionUpdate { tracks, index, position_secs, status, volume, repeat, shuffle } => {
            if let Some(ref nid) = *local_node_id {
                if let Err(e) = core.local_session_update(nid, tracks, index, position_secs, status, volume, repeat, shuffle).await {
                    warn!("local_session_update error: {e}");
                }
            }
            Ok(())
        }
        WsCommand::Handoff { from_node_id, to_node_id } => {
            if let Err(e) = core.handoff(&from_node_id, &to_node_id).await {
                warn!("handoff error: {e}");
            }
            Ok(())
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
            let (tracks, idx) = s.selected_node()
                .map(|n| (n.queue.clone(), n.current_index))
                .unwrap_or_else(|| (s.queue.clone(), s.current_index));
            WsResponse::Queue {
                tracks,
                current_index: idx,
            }
        }
    }
}
