use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use kanade_core::model::{Album, RepeatMode, Track};
use kanade_node_protocol::{NodeCommand, NodeRegistration, NodeRegistrationAck, NodeStateUpdate};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum WsCommand {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek {
        position_secs: f64,
    },
    SetVolume {
        volume: u8,
    },
    SetRepeat {
        repeat: RepeatMode,
    },
    SetShuffle {
        shuffle: bool,
    },
    SelectNode {
        node_id: String,
    },
    AddToQueue {
        track: Track,
    },
    AddTracksToQueue {
        tracks: Vec<Track>,
    },
    PlayIndex {
        index: usize,
    },
    RemoveFromQueue {
        index: usize,
    },
    MoveInQueue {
        from: usize,
        to: usize,
    },
    ClearQueue,
    ReplaceAndPlay {
        tracks: Vec<Track>,
        index: usize,
    },
    LocalSessionStart {
        device_name: String,
        #[serde(default)]
        device_id: Option<String>,
    },
    LocalSessionStop,
    LocalSessionUpdate {
        tracks: Vec<Track>,
        #[serde(default)]
        index: Option<usize>,
        position_secs: f64,
        status: kanade_core::model::PlaybackStatus,
        volume: u8,
        repeat: RepeatMode,
        shuffle: bool,
    },
    Handoff {
        from_node_id: String,
        to_node_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "req", rename_all = "snake_case")]
pub enum WsRequest {
    GetAlbums,
    GetAlbumTracks { album_id: String },
    GetArtists,
    GetArtistAlbums { artist: String },
    GetArtistTracks { artist: String },
    GetGenres,
    GetGenreAlbums { genre: String },
    GetGenreTracks { genre: String },
    Search { query: String },
    GetQueue,
    SignUrls { paths: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    State {
        state: kanade_core::state::PlaybackState,
    },
    MediaAuth {
        media_auth_key_id: String,
    },
    Response {
        req_id: u64,
        data: WsResponse,
    },
    NodeRegistrationAck {
        #[serde(flatten)]
        ack: NodeRegistrationAck,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsResponse {
    Albums {
        albums: Vec<Album>,
    },
    AlbumTracks {
        tracks: Vec<Track>,
    },
    Artists {
        artists: Vec<String>,
    },
    ArtistAlbums {
        albums: Vec<Album>,
    },
    ArtistTracks {
        tracks: Vec<Track>,
    },
    Genres {
        genres: Vec<String>,
    },
    GenreAlbums {
        albums: Vec<Album>,
    },
    GenreTracks {
        tracks: Vec<Track>,
    },
    SearchResults {
        tracks: Vec<Track>,
    },
    Queue {
        tracks: Vec<Track>,
        current_index: Option<usize>,
    },
    SignedUrls {
        urls: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClientMessage {
    Command(WsCommand),
    Request {
        req_id: u64,
        #[serde(flatten)]
        req: WsRequest,
    },
    NodeStateUpdate(NodeStateUpdate),
    NodeRegistration(NodeRegistration),
}

pub type WsNodeCommand = NodeCommand;
