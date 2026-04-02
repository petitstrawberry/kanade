use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::debug;

use kanade_core::{ports::EventBroadcaster, state::PlaybackState};

pub struct WsBroadcaster {
    tx: broadcast::Sender<String>,
}

impl WsBroadcaster {
    pub fn new(capacity: usize) -> (Arc<Self>, broadcast::Receiver<String>) {
        let (tx, rx) = broadcast::channel(capacity);
        (Arc::new(Self { tx }), rx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

#[async_trait]
impl EventBroadcaster for WsBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        match serde_json::to_string(state) {
            Ok(json) => {
                let _ = self.tx.send(json);
            }
            Err(e) => {
                debug!("WsBroadcaster: failed to serialise state: {e}");
            }
        }
    }
}
