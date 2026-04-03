use serde::{Deserialize, Serialize};

use kanade_core::model::{Album, RepeatMode, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum WsCommand {
    Play {
        node_id: String,
    },
    Pause {
        node_id: String,
    },
    Stop {
        node_id: String,
    },
    Next {
        node_id: String,
    },
    Previous {
        node_id: String,
    },
    Seek {
        node_id: String,
        position_secs: f64,
    },
    SetVolume {
        node_id: String,
        volume: u8,
    },
    SetRepeat {
        node_id: String,
        repeat: RepeatMode,
    },
    SetShuffle {
        node_id: String,
        shuffle: bool,
    },
    AddToQueue {
        node_id: String,
        track: Track,
    },
    AddTracksToQueue {
        node_id: String,
        tracks: Vec<Track>,
    },
    PlayIndex {
        node_id: String,
        index: usize,
    },
    RemoveFromQueue {
        node_id: String,
        index: usize,
    },
    MoveInQueue {
        node_id: String,
        from: usize,
        to: usize,
    },
    ClearQueue {
        node_id: String,
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
    GetQueue { node_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    State {
        state: kanade_core::state::PlaybackState,
    },
    Response {
        req_id: u64,
        data: WsResponse,
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
}
