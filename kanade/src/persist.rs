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
        for node in &state.nodes {
            let db = Arc::clone(&self.db);
            let node_id = node.id.clone();
            let persist_node_id = node_id.clone();
            let queue_file_paths: Vec<String> = node.queue.iter().map(|t| t.file_path.clone()).collect();
            let current_index = node.current_index;
            let volume = node.volume;
            let shuffle = node.shuffle;
            let repeat = match node.repeat {
                RepeatMode::Off => "off",
                RepeatMode::One => "one",
                RepeatMode::All => "all",
            }
            .to_string();

            match tokio::task::spawn_blocking(move || {
                let guard = db
                    .lock()
                    .map_err(|e| anyhow::anyhow!("database mutex poisoned: {e}"))?;
                guard.save_node_state(
                    &persist_node_id,
                    &queue_file_paths,
                    current_index,
                    volume,
                    shuffle,
                    &repeat,
                )
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!(node_id = %node_id, error = %e, "failed to persist node state"),
                Err(e) => warn!(node_id = %node_id, error = %e, "failed to join node state persist task"),
            }
        }
    }
}
