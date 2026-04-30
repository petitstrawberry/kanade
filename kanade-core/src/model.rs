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
        }
    }
}

fn default_connected() -> bool {
    true
}
