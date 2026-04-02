use serde::{Deserialize, Serialize};

use crate::model::Track;

/// A single entry in the playback queue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueEntry {
    /// 0-based index in the current queue.
    pub index: usize,
    pub track: Track,
}

/// Whether the player is actively playing, paused, or stopped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

/// The single source of truth for the entire system.
///
/// This struct is held behind an `Arc<RwLock<PlaybackState>>` and shared
/// across every adapter and the core controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    /// Current playback status.
    pub status: PlaybackStatus,
    /// The ordered queue of tracks.
    pub queue: Vec<QueueEntry>,
    /// Index of the currently active track inside `queue`.
    pub current_index: Option<usize>,
    /// Playback position within the current track, in seconds.
    pub position_secs: f64,
    /// Volume level, 0–100.
    pub volume: u8,
    /// Whether repeat mode is active.
    pub repeat: bool,
    /// Whether shuffle mode is active.
    pub shuffle: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            queue: Vec::new(),
            current_index: None,
            position_secs: 0.0,
            volume: 50,
            repeat: false,
            shuffle: false,
        }
    }
}

impl PlaybackState {
    /// Returns the currently active QueueEntry, if any.
    pub fn current_entry(&self) -> Option<&QueueEntry> {
        self.current_index.and_then(|i| self.queue.get(i))
    }
}
