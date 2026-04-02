use serde::{Deserialize, Serialize};

use kanade_core::model::{Album, RepeatMode, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum WsCommand {
    Play {
        zone_id: String,
    },
    Pause {
        zone_id: String,
    },
    Stop {
        zone_id: String,
    },
    Next {
        zone_id: String,
    },
    Previous {
        zone_id: String,
    },
    Seek {
        zone_id: String,
        position_secs: f64,
    },
    SetVolume {
        zone_id: String,
        volume: u8,
    },
    SetRepeat {
        zone_id: String,
        repeat: RepeatMode,
    },
    SetShuffle {
        zone_id: String,
        shuffle: bool,
    },
    AddToQueue {
        zone_id: String,
        track: Track,
    },
    AddTracksToQueue {
        zone_id: String,
        tracks: Vec<Track>,
    },
    PlayIndex {
        zone_id: String,
        index: usize,
    },
    RemoveFromQueue {
        zone_id: String,
        index: usize,
    },
    MoveInQueue {
        zone_id: String,
        from: usize,
        to: usize,
    },
    ClearQueue {
        zone_id: String,
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
    GetGenreTracks { genre: String },
    Search { query: String },
    GetQueue { zone_id: String },
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
