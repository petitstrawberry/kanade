use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: String,
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album_title: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_secs: Option<f64>,
    pub format: Option<String>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Album {
    pub id: String,
    pub dir_path: String,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    Loading,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    #[default]
    Remote,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    #[serde(default = "default_connected")]
    pub connected: bool,
    pub status: PlaybackStatus,
    pub position_secs: f64,
    pub volume: u8,
    #[serde(default)]
    pub node_type: NodeType,
    #[serde(default)]
    pub queue: Vec<Track>,
    #[serde(default)]
    pub current_index: Option<usize>,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnected_at: Option<i64>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            connected: true,
            status: PlaybackStatus::Stopped,
            position_secs: 0.0,
            volume: 50,
            node_type: NodeType::Remote,
            queue: Vec::new(),
            current_index: None,
            repeat: RepeatMode::Off,
            shuffle: false,
            device_id: None,
            disconnected_at: None,
        }
    }
}

fn default_connected() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Playlist model
// ---------------------------------------------------------------------------

/// The kind of a playlist.
///
/// - `Normal`: a static, user-curated ordered list of tracks.
/// - `Smart`: a dynamic playlist whose contents are computed by evaluating a
///   filter over the library on demand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaylistKind {
    Normal,
    Smart {
        filter: SmartFilter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sort_by: Option<SmartSort>,
    },
}

/// Filter combining one or more conditions for a smart playlist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartFilter {
    /// How to combine `conditions`.
    #[serde(default)]
    pub match_mode: MatchMode,
    /// At least one condition is required for the filter to be considered
    /// non-empty; an empty list matches nothing.
    pub conditions: Vec<SmartCondition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// All conditions must match (logical AND).
    #[default]
    All,
    /// Any condition must match (logical OR).
    Any,
}

/// Single rule applied against a track field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartCondition {
    pub field: SmartField,
    pub op: SmartOperator,
    pub value: String,
}

/// Track fields that can be filtered on by a smart playlist.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmartField {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Composer,
    Genre,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmartOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
}

/// Sort order applied after filtering, for smart playlists.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmartSort {
    Title,
    Artist,
    Album,
    Genre,
}

/// Persistent playlist record (without its track contents).
///
/// For `Normal` playlists, ordered tracks are stored separately in the
/// `playlist_tracks` table and fetched via `Database::get_playlist_tracks`.
/// For `Smart` playlists, the track list is dynamically evaluated by
/// `Database::evaluate_smart_playlist`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub kind: PlaylistKind,
    /// Unix epoch seconds of creation.
    pub created_at: i64,
    /// Unix epoch seconds of last modification.
    pub updated_at: i64,
}
