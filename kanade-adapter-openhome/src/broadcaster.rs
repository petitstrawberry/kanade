use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::debug;

use kanade_core::{ports::EventBroadcaster, state::PlaybackState};

/// Caches the latest [`PlaybackState`] so the OpenHome HTTP server can return
/// it on the next polled request (the control point polls us, we don't push).
///
/// Also implements [`EventBroadcaster`] so the Core can call
/// `on_state_changed` after every mutation.
pub struct OpenHomeBroadcaster {
    latest: RwLock<Option<PlaybackState>>,
}

impl OpenHomeBroadcaster {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { latest: RwLock::new(None) })
    }

    /// Returns the most recently cached state, or `None` if the Core has not
    /// yet produced any state.
    pub async fn current_state(&self) -> Option<PlaybackState> {
        self.latest.read().await.clone()
    }
}

impl Default for OpenHomeBroadcaster {
    fn default() -> Self {
        Self { latest: RwLock::new(None) }
    }
}

#[async_trait]
impl EventBroadcaster for OpenHomeBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        debug!("OpenHomeBroadcaster: caching new state (status={:?})", state.status);
        *self.latest.write().await = Some(state.clone());
    }
}
