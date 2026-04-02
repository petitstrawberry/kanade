use async_trait::async_trait;

use crate::{error::CoreError, state::PlaybackState};

// ---------------------------------------------------------------------------
// Output port — implemented by audio renderer adapters (e.g. MPD)
// ---------------------------------------------------------------------------

/// Every audio-rendering backend must implement this trait.
///
/// The Core only calls these methods and never knows whether the backend is
/// MPD, GStreamer, or something else.
#[async_trait]
pub trait AudioRenderer: Send + Sync {
    /// Start or resume playback.
    async fn execute_play(&self) -> Result<(), CoreError>;

    /// Pause playback.
    async fn execute_pause(&self) -> Result<(), CoreError>;

    /// Stop playback (position reset to 0).
    async fn execute_stop(&self) -> Result<(), CoreError>;

    /// Skip to the next track.
    async fn execute_next(&self) -> Result<(), CoreError>;

    /// Go back to the previous track.
    async fn execute_previous(&self) -> Result<(), CoreError>;

    /// Seek to an absolute position within the current track.
    async fn execute_seek(&self, position_secs: f64) -> Result<(), CoreError>;

    /// Set the playback volume (0–100).
    async fn execute_set_volume(&self, volume: u8) -> Result<(), CoreError>;

    /// Replace the renderer's internal queue with the given list of file paths.
    async fn execute_set_queue(&self, file_paths: &[String]) -> Result<(), CoreError>;
}

// ---------------------------------------------------------------------------
// Broadcast port — implemented by anything that must react to state changes
// ---------------------------------------------------------------------------

/// Anything that wants to be notified when the shared PlaybackState changes
/// must implement this trait.
///
/// Both input adapters (WebSocket, OpenHome) register themselves so the Core
/// can push updates after every mutation.
#[async_trait]
pub trait EventBroadcaster: Send + Sync {
    /// Called by the Core after every successful state mutation.
    async fn on_state_changed(&self, state: &PlaybackState);
}
