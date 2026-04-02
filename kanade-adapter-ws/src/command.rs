use serde::{Deserialize, Serialize};

use kanade_core::model::Track;

/// Commands that a WebSocket client may send to Kanade.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum WsCommand {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek { position_secs: f64 },
    SetVolume { volume: u8 },
    SetQueue { tracks: Vec<Track>, start_index: Option<usize> },
    SetRepeat { repeat: bool },
    SetShuffle { shuffle: bool },
}
