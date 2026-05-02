use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use kanade_core::model::{Album, Playlist, PlaylistKind, RepeatMode, Track};
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
    // -------- Playlist commands --------
    CreatePlaylist {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(flatten)]
        kind: PlaylistKind,
    },
    UpdatePlaylist {
        playlist_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// `Some(None)` clears the description, `None` leaves it untouched.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<PlaylistKind>,
    },
    DeletePlaylist {
        playlist_id: String,
    },
    SetPlaylistTracks {
        playlist_id: String,
        track_ids: Vec<String>,
    },
    AppendPlaylistTracks {
        playlist_id: String,
        track_ids: Vec<String>,
    },
    RemovePlaylistTrack {
        playlist_id: String,
        position: usize,
    },
    MovePlaylistTrack {
        playlist_id: String,
        from: usize,
        to: usize,
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
    GetPlaylists,
    GetPlaylist { playlist_id: String },
    GetPlaylistTracks { playlist_id: String },
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
    Playlists {
        playlists: Vec<Playlist>,
    },
    PlaylistDetails {
        playlist: Option<Playlist>,
    },
    PlaylistTracks {
        playlist_id: String,
        tracks: Vec<Track>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use kanade_core::model::{MatchMode, SmartCondition, SmartField, SmartFilter, SmartOperator};

    #[test]
    fn create_normal_playlist_command_roundtrip() {
        let json = r#"{"cmd":"create_playlist","name":"Favs","kind":"normal"}"#;
        let cmd: WsCommand = serde_json::from_str(json).unwrap();
        match &cmd {
            WsCommand::CreatePlaylist { name, kind, .. } => {
                assert_eq!(name, "Favs");
                assert!(matches!(kind, PlaylistKind::Normal));
            }
            _ => panic!("wrong variant"),
        }
        let serialized = serde_json::to_string(&cmd).unwrap();
        assert!(serialized.contains("\"cmd\":\"create_playlist\""));
        assert!(serialized.contains("\"kind\":\"normal\""));
    }

    #[test]
    fn create_smart_playlist_command_roundtrip() {
        let cmd = WsCommand::CreatePlaylist {
            name: "Rock".to_string(),
            description: None,
            kind: PlaylistKind::Smart {
                filter: SmartFilter {
                    match_mode: MatchMode::All,
                    conditions: vec![SmartCondition {
                        field: SmartField::Genre,
                        op: SmartOperator::Equals,
                        value: "Rock".to_string(),
                    }],
                },
                limit: Some(50),
                sort_by: None,
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: WsCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            WsCommand::CreatePlaylist {
                kind: PlaylistKind::Smart { .. },
                ..
            }
        ));
    }

    #[test]
    fn playlist_request_roundtrip() {
        let json = r#"{"req_id":7,"req":"get_playlists"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Request { req_id, req } => {
                assert_eq!(req_id, 7);
                assert!(matches!(req, WsRequest::GetPlaylists));
            }
            _ => panic!("wrong variant"),
        }
    }
}
