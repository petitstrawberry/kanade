use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::debug;

use kanade_core::{ports::EventBroadcaster, state::PlaybackState};

pub struct OpenHomeBroadcaster {
    latest: RwLock<Option<PlaybackState>>,
}

impl OpenHomeBroadcaster {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            latest: RwLock::new(None),
        })
    }

    pub async fn current_state(&self) -> Option<PlaybackState> {
        self.latest.read().await.clone()
    }
}

impl Default for OpenHomeBroadcaster {
    fn default() -> Self {
        Self {
            latest: RwLock::new(None),
        }
    }
}

#[async_trait]
impl EventBroadcaster for OpenHomeBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        debug!("OpenHomeBroadcaster: caching new state");
        *self.latest.write().await = Some(state.clone());
    }
}
