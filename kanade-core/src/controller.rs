use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::instrument;

use crate::{
    error::CoreError,
    model::Track,
    ports::{AudioRenderer, EventBroadcaster},
    state::{PlaybackState, PlaybackStatus, QueueEntry},
};

/// The central controller — the "brain" of Kanade.
///
/// All input adapters (WebSocket, OpenHome, …) call methods on this struct.
/// It mutates the shared [`PlaybackState`], delegates rendering to the
/// registered [`AudioRenderer`], and broadcasts the new state to every
/// registered [`EventBroadcaster`].
pub struct CoreController {
    state: Arc<RwLock<PlaybackState>>,
    renderer: Arc<dyn AudioRenderer>,
    broadcasters: Vec<Arc<dyn EventBroadcaster>>,
}

impl CoreController {
    /// Create a new controller.
    ///
    /// `renderer` — the single audio output adapter.
    /// `broadcasters` — all input adapters that need state-change notifications.
    pub fn new(
        renderer: Arc<dyn AudioRenderer>,
        broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(PlaybackState::default())),
            renderer,
            broadcasters,
        }
    }

    /// Returns a clone of the `Arc<RwLock<PlaybackState>>` so adapters can
    /// read the current state without going through the controller.
    pub fn state_handle(&self) -> Arc<RwLock<PlaybackState>> {
        Arc::clone(&self.state)
    }

    // ------------------------------------------------------------------
    // Playback commands
    // ------------------------------------------------------------------

    /// Start or resume playback.
    #[instrument(skip(self))]
    pub async fn play(&self) -> Result<(), CoreError> {
        self.renderer.execute_play().await?;
        {
            let mut s = self.state.write().await;
            s.status = PlaybackStatus::Playing;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Pause playback.
    #[instrument(skip(self))]
    pub async fn pause(&self) -> Result<(), CoreError> {
        self.renderer.execute_pause().await?;
        {
            let mut s = self.state.write().await;
            s.status = PlaybackStatus::Paused;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Stop playback.
    #[instrument(skip(self))]
    pub async fn stop(&self) -> Result<(), CoreError> {
        self.renderer.execute_stop().await?;
        {
            let mut s = self.state.write().await;
            s.status = PlaybackStatus::Stopped;
            s.position_secs = 0.0;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Skip to the next track.
    #[instrument(skip(self))]
    pub async fn next(&self) -> Result<(), CoreError> {
        {
            let mut s = self.state.write().await;
            let queue_len = s.queue.len();
            if queue_len == 0 {
                return Err(CoreError::QueueEmpty);
            }
            let next_index = match s.current_index {
                Some(i) => {
                    if s.repeat {
                        (i + 1) % queue_len
                    } else if i + 1 < queue_len {
                        i + 1
                    } else {
                        s.status = PlaybackStatus::Stopped;
                        s.current_index = None;
                        self.renderer.execute_stop().await?;
                        drop(s);
                        self.broadcast().await;
                        return Ok(());
                    }
                }
                None => 0,
            };
            s.current_index = Some(next_index);
            s.position_secs = 0.0;
        }
        self.renderer.execute_next().await?;
        self.broadcast().await;
        Ok(())
    }

    /// Go back to the previous track.
    #[instrument(skip(self))]
    pub async fn previous(&self) -> Result<(), CoreError> {
        {
            let mut s = self.state.write().await;
            let queue_len = s.queue.len();
            if queue_len == 0 {
                return Err(CoreError::QueueEmpty);
            }
            let prev_index = match s.current_index {
                Some(0) | None => {
                    if s.repeat {
                        queue_len - 1
                    } else {
                        0
                    }
                }
                Some(i) => i - 1,
            };
            s.current_index = Some(prev_index);
            s.position_secs = 0.0;
        }
        self.renderer.execute_previous().await?;
        self.broadcast().await;
        Ok(())
    }

    /// Seek to a position (in seconds) within the current track.
    #[instrument(skip(self))]
    pub async fn seek(&self, position_secs: f64) -> Result<(), CoreError> {
        self.renderer.execute_seek(position_secs).await?;
        {
            let mut s = self.state.write().await;
            s.position_secs = position_secs;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Set the playback volume (0–100).
    #[instrument(skip(self))]
    pub async fn set_volume(&self, volume: u8) -> Result<(), CoreError> {
        if volume > 100 {
            return Err(CoreError::InvalidVolume);
        }
        self.renderer.execute_set_volume(volume).await?;
        {
            let mut s = self.state.write().await;
            s.volume = volume;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Replace the current queue with a new list of tracks and optionally
    /// start playing from the given index.
    #[instrument(skip(self, tracks))]
    pub async fn set_queue(
        &self,
        tracks: Vec<Track>,
        start_index: Option<usize>,
    ) -> Result<(), CoreError> {
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        self.renderer.execute_set_queue(&file_paths).await?;
        {
            let mut s = self.state.write().await;
            s.queue = tracks
                .into_iter()
                .enumerate()
                .map(|(index, track)| QueueEntry { index, track })
                .collect();
            s.current_index = start_index;
            s.position_secs = 0.0;
            s.status = if start_index.is_some() {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Stopped
            };
        }
        if start_index.is_some() {
            self.renderer.execute_play().await?;
        }
        self.broadcast().await;
        Ok(())
    }

    /// Enable or disable repeat mode.
    pub async fn set_repeat(&self, repeat: bool) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        s.repeat = repeat;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    /// Enable or disable shuffle mode.
    pub async fn set_shuffle(&self, shuffle: bool) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        s.shuffle = shuffle;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Snapshot the current state and push it to every broadcaster.
    async fn broadcast(&self) {
        let snapshot = self.state.read().await.clone();
        for b in &self.broadcasters {
            b.on_state_changed(&snapshot).await;
        }
    }
}
