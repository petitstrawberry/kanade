use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kanade_core::{
    model::RepeatMode,
    ports::StatePersister,
    state::PlaybackState,
};
use tracing::warn;

pub struct DatabaseStatePersister {
    pub db: Arc<Mutex<kanade_db::Database>>,
}

impl DatabaseStatePersister {
    pub fn new(db: Arc<Mutex<kanade_db::Database>>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl StatePersister for DatabaseStatePersister {
    async fn persist(&self, state: &PlaybackState) {
        let db = Arc::clone(&self.db);
        let queue_file_paths: Vec<String> = state.queue.iter().map(|t| t.file_path.clone()).collect();
        let current_index = state.current_index;
        let selected_node_id = state.selected_node_id.clone();
        let shuffle = state.shuffle;
        let repeat = match state.repeat {
            RepeatMode::Off => "off",
            RepeatMode::One => "one",
            RepeatMode::All => "all",
        }
        .to_string();

        match tokio::task::spawn_blocking(move || {
            let guard = db
                .lock()
                .map_err(|e| anyhow::anyhow!("database mutex poisoned: {e}"))?;
            guard.save_playback_state(
                &queue_file_paths,
                current_index,
                selected_node_id,
                shuffle,
                &repeat,
            )
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "failed to persist playback state"),
            Err(e) => warn!(error = %e, "failed to join playback state persist task"),
        }
    }
}
