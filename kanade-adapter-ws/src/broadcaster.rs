use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::debug;

use kanade_core::{ports::EventBroadcaster, state::PlaybackState};

/// Implements [`EventBroadcaster`] for the WebSocket adapter.
///
/// When the Core calls `on_state_changed`, this broadcaster serialises the new
/// state to JSON and sends it through a Tokio broadcast channel.  The
/// [`WsServer`] task subscribes to this channel and forwards messages to every
/// connected WebSocket client.
pub struct WsBroadcaster {
    tx: broadcast::Sender<String>,
}

impl WsBroadcaster {
    /// Create a new broadcaster.  Returns the broadcaster and a receiver that
    /// the WebSocket server should subscribe to.
    pub fn new(capacity: usize) -> (Arc<Self>, broadcast::Receiver<String>) {
        let (tx, rx) = broadcast::channel(capacity);
        (Arc::new(Self { tx }), rx)
    }

    /// Subscribe an additional receiver to the broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

#[async_trait]
impl EventBroadcaster for WsBroadcaster {
    async fn on_state_changed(&self, state: &PlaybackState) {
        match serde_json::to_string(state) {
            Ok(json) => {
                // Ignore send errors — there may be no subscribers yet.
                let _ = self.tx.send(json);
            }
            Err(e) => {
                debug!("WsBroadcaster: failed to serialise state: {e}");
            }
        }
    }
}
