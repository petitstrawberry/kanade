use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kanade_core::{model::RepeatMode, ports::StatePersister, state::PlaybackState};
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

        let queue_file_paths: Vec<String> =
            state.queue.iter().map(|t| t.file_path.clone()).collect();
        let current_index = state.current_index;
        let selected_node_id = state.selected_node_id.clone();
        let shuffle = state.shuffle;
        let repeat = match state.repeat {
            RepeatMode::Off => "off",
            RepeatMode::One => "one",
            RepeatMode::All => "all",
        }
        .to_string();

        let node_states: Vec<(
            String,
            Vec<String>,
            Option<usize>,
            u8,
            bool,
            String,
            kanade_core::model::NodeType,
            Option<String>,
            Option<i64>,
        )> = state
            .nodes
            .iter()
            .map(|node| {
                let paths: Vec<String> = node.queue.iter().map(|t| t.file_path.clone()).collect();
                let rep = match node.repeat {
                    RepeatMode::Off => "off",
                    RepeatMode::One => "one",
                    RepeatMode::All => "all",
                };
                (
                    node.id.clone(),
                    paths,
                    node.current_index,
                    node.volume,
                    node.shuffle,
                    rep.to_string(),
                    node.node_type,
                    node.device_id.clone(),
                    node.disconnected_at,
                )
            })
            .collect();

        let keep_node_ids: Vec<String> = node_states
            .iter()
            .map(|(node_id, ..)| node_id.clone())
            .collect();

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
            )?;

            for (node_id, paths, idx, vol, shuf, rep, node_type, device_id, disconnected_at) in
                &node_states
            {
                guard.save_node_state(
                    node_id,
                    paths,
                    *idx,
                    *vol,
                    *shuf,
                    rep,
                    *node_type,
                    device_id.as_deref(),
                    *disconnected_at,
                )?;
            }

            guard.prune_node_states_except(&keep_node_ids)?;

            Ok::<(), anyhow::Error>(())
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "failed to persist playback state"),
            Err(e) => warn!(error = %e, "failed to join playback state persist task"),
        }
    }
}
