use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;
use tracing::{info, instrument, warn};

use crate::{
    error::CoreError,
    model::{Node, NodeType, PlaybackStatus, RepeatMode, Track},
    ports::{AudioOutput, EventBroadcaster, StatePersister},
    state::PlaybackState,
};

#[derive(Debug, Clone, Default)]
struct NodeTransportState {
    projection_generation: u64,
    projection_start_index: Option<usize>,
    loaded_len: usize,
}

#[derive(Clone)]
struct OutputSlot {
    connection_id: String,
    output: Arc<dyn AudioOutput>,
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Core {
    state: Arc<RwLock<PlaybackState>>,
    outputs: Arc<RwLock<HashMap<String, OutputSlot>>>,
    broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    persisters: Vec<Arc<dyn StatePersister>>,
    transport_state: Arc<RwLock<HashMap<String, NodeTransportState>>>,
}

impl Core {
    pub fn new(
        outputs: Vec<(String, Arc<dyn AudioOutput>)>,
        broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(PlaybackState {
                nodes: Vec::new(),
                selected_node_id: None,
                queue: Vec::new(),
                current_index: None,
                shuffle: false,
                repeat: RepeatMode::Off,
            })),
            outputs: Arc::new(RwLock::new(
                outputs
                    .into_iter()
                    .map(|(id, output)| {
                        (
                            id,
                            OutputSlot {
                                connection_id: "static".to_string(),
                                output,
                            },
                        )
                    })
                    .collect(),
            )),
            broadcasters,
            persisters: Vec::new(),
            transport_state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new output at runtime. Safe to call on an `Arc<Core>`.
    pub async fn register_output(
        &self,
        id: String,
        connection_id: String,
        output: Arc<dyn AudioOutput>,
    ) {
        self.outputs.write().await.insert(
            id,
            OutputSlot {
                connection_id,
                output,
            },
        );
    }

    /// Remove a previously registered output. Safe to call on an `Arc<Core>`.
    pub async fn unregister_output(&self, id: &str, connection_id: &str) {
        let mut outputs = self.outputs.write().await;
        if outputs
            .get(id)
            .is_some_and(|slot| slot.connection_id == connection_id)
        {
            outputs.remove(id);
        }
    }

    pub async fn has_output(&self, id: &str) -> bool {
        self.outputs.read().await.contains_key(id)
    }

    pub async fn is_same_output(&self, id: &str, connection_id: &str) -> bool {
        self.outputs
            .read()
            .await
            .get(id)
            .is_some_and(|slot| slot.connection_id == connection_id)
    }

    pub fn add_broadcaster(&mut self, b: Arc<dyn EventBroadcaster>) {
        self.broadcasters.push(b);
    }

    pub fn add_persister(&mut self, p: Arc<dyn StatePersister>) {
        self.persisters.push(p);
    }

    pub async fn restore_state(&self, state: PlaybackState) {
        *self.state.write().await = state;
    }

    pub async fn sync_node_state(
        &self,
        node_id: &str,
        status: PlaybackStatus,
        position_secs: f64,
        volume: u8,
        mpd_song_index: Option<usize>,
        projection_generation: u64,
    ) {
        let projected_current_index = {
            let transport = self.transport_state.read().await;
            transport.get(node_id).and_then(|projection| {
                if projection.projection_generation != projection_generation {
                    return None;
                }

                let mpd_song_index = mpd_song_index?;
                if mpd_song_index >= projection.loaded_len {
                    return None;
                }

                projection
                    .projection_start_index
                    .and_then(|start| start.checked_add(mpd_song_index))
            })
        };

        let mut s = self.state.write().await;
        let mut changed = false;
        if let Some(node) = s.node_mut(node_id) {
            if node.status != status {
                node.status = status;
                changed = true;
            }
            if (node.position_secs - position_secs).abs() > f64::EPSILON {
                node.position_secs = position_secs;
                changed = true;
            }
            if node.volume != volume {
                node.volume = volume;
                changed = true;
            }

            if let Some(next_index) = projected_current_index {
                if next_index < node.queue.len() && node.current_index != Some(next_index) {
                    node.current_index = Some(next_index);
                    changed = true;
                }
            }
        }
        drop(s);
        if changed {
            self.broadcast().await;
        }
    }

    pub fn state_handle(&self) -> Arc<RwLock<PlaybackState>> {
        Arc::clone(&self.state)
    }

    async fn rebuild_projection_state(
        &self,
        node_id: &str,
        projection_start_index: Option<usize>,
        loaded_len: usize,
    ) -> u64 {
        let mut transport = self.transport_state.write().await;
        let entry = transport.entry(node_id.to_string()).or_default();
        entry.projection_generation = entry.projection_generation.wrapping_add(1);
        entry.projection_start_index = projection_start_index;
        entry.loaded_len = loaded_len;
        entry.projection_generation
    }

    pub async fn add_node(&self, node: Node) {
        let mut s = self.state.write().await;
        let is_first = s.nodes.is_empty();
        if let Some(existing) = s.node_mut(&node.id) {
            existing.name = node.name;
            existing.connected = node.connected;
        } else {
            s.nodes.push(node.clone());
        }
        let should_select = (is_first || s.selected_node_id.is_none()) && node.connected;
        if should_select {
            s.selected_node_id = Some(node.id.clone());

            let should_migrate_queue =
                s.node(&node.id).is_some_and(|n| n.queue.is_empty()) && !s.queue.is_empty();
            if should_migrate_queue {
                let queue = std::mem::take(&mut s.queue);
                let current_index = s.current_index.take();
                let shuffle = s.shuffle;
                let repeat = s.repeat;
                if let Some(n) = s.node_mut(&node.id) {
                    n.queue = queue;
                    n.current_index = current_index;
                    n.shuffle = shuffle;
                    n.repeat = repeat;
                }
            }
        }
        s.sync_top_level_from_selected_node();
        drop(s);
        self.broadcast().await;
    }

    pub async fn mark_node_connected(&self, node_id: &str, connected: bool) {
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.connected = connected;
            if !connected {
                node.status = PlaybackStatus::Stopped;
                node.position_secs = 0.0;
            }
        }
        drop(s);
        self.broadcast().await;
    }

    pub async fn handle_node_disconnected(&self, node_id: &str) {
        let (fallback_node_id, resume_status, resume_position_secs) = {
            let mut s = self.state.write().await;
            let was_selected = s.selected_node_id.as_deref() == Some(node_id);
            let mut resume_status = PlaybackStatus::Stopped;
            let mut resume_position_secs = 0.0;

            if let Some(node) = s.node(node_id) {
                resume_status = node.status;
                resume_position_secs = node.position_secs;
            }

            let fallback_node_id = if was_selected {
                s.nodes
                    .iter()
                    .find(|n| n.id != node_id && n.connected)
                    .map(|n| n.id.clone())
            } else {
                None
            };

            if let Some(node) = s.node_mut(node_id) {
                node.connected = false;
                if !(was_selected && fallback_node_id.is_none()) {
                    node.status = PlaybackStatus::Stopped;
                    node.position_secs = 0.0;
                }
            }

            if let Some(fallback_node_id) = fallback_node_id.as_ref() {
                s.selected_node_id = Some(fallback_node_id.clone());
                for node in s.nodes.iter_mut() {
                    if node.id == *fallback_node_id {
                        node.status = resume_status;
                        node.position_secs = resume_position_secs;
                    } else if node.id != node_id {
                        node.status = PlaybackStatus::Stopped;
                        node.position_secs = 0.0;
                    }
                }
            } else if was_selected {
                s.selected_node_id = Some(node_id.to_string());
            }

            (fallback_node_id, resume_status, resume_position_secs)
        };

        if let Some(fallback_node_id) = fallback_node_id {
            if let Err(e) = self
                .sync_connected_node_to_logical_state(&fallback_node_id)
                .await
            {
                warn!(fallback_node_id = %fallback_node_id, resume_status = ?resume_status, resume_position_secs, "handle_node_disconnected: failed to restore fallback output: {e}");
            }
        }

        self.broadcast().await;
    }

    pub async fn sync_connected_node_to_logical_state(
        &self,
        node_id: &str,
    ) -> Result<(), CoreError> {
        let (is_selected, resume_status, resume_position_secs) = {
            let s = self.state.read().await;
            let node = s.node(node_id).ok_or(CoreError::NodeNotFound)?;
            (
                s.selected_node_id.as_deref() == Some(node_id),
                node.status,
                node.position_secs,
            )
        };

        self.sync_output_to_global(node_id).await?;

        if is_selected {
            self.apply_output_runtime_state(node_id, resume_status, resume_position_secs)
                .await?;
        } else {
            self.stop_node(node_id).await?;
        }

        Ok(())
    }

    pub async fn remove_node(&self, node_id: &str) {
        let mut s = self.state.write().await;
        s.nodes.retain(|n| n.id != node_id);
        if s.selected_node_id.as_deref() == Some(node_id) {
            s.selected_node_id = s.nodes.first().map(|n| n.id.clone());
        }
        drop(s);
        self.transport_state.write().await.remove(node_id);
        self.outputs.write().await.remove(node_id);
        self.broadcast().await;
    }

    pub async fn local_session_start(
        &self,
        device_name: &str,
        device_id: Option<&str>,
    ) -> Result<String, CoreError> {
        let mut s = self.state.write().await;

        if let Some(did) = device_id {
            let matching_ids: Vec<String> = s
                .nodes
                .iter()
                .filter(|n| n.node_type == NodeType::Local && n.device_id.as_deref() == Some(did))
                .map(|n| n.id.clone())
                .collect();

            if let Some(canonical_id) = matching_ids
                .iter()
                .find(|id| s.node(id).is_some_and(|n| n.connected))
                .cloned()
                .or_else(|| matching_ids.first().cloned())
            {
                for duplicate_id in matching_ids.iter().filter(|id| **id != canonical_id) {
                    s.nodes.retain(|n| n.id != *duplicate_id);
                    if s.selected_node_id.as_deref() == Some(duplicate_id.as_str()) {
                        s.selected_node_id = Some(canonical_id.clone());
                    }
                }

                let node_id = canonical_id.clone();
                if let Some(existing) = s.node_mut(&canonical_id) {
                    existing.connected = true;
                    existing.name = device_name.to_string();
                    existing.device_id = Some(did.to_string());
                    existing.disconnected_at = None;
                }
                drop(s);
                self.transport_state.write().await.retain(|id, _| {
                    id == &canonical_id || !matching_ids.iter().any(|dup| dup == id)
                });
                self.outputs.write().await.retain(|id, _| {
                    id == &canonical_id || !matching_ids.iter().any(|dup| dup == id)
                });
                self.broadcast().await;
                return Ok(node_id);
            }
        }

        let node_id = device_id
            .map(|id| format!("local-{}", id))
            .unwrap_or_else(|| format!("local-{}", uuid::Uuid::new_v4()));

        s.nodes.push(Node {
            id: node_id.clone(),
            name: device_name.to_string(),
            connected: true,
            node_type: NodeType::Local,
            device_id: device_id.map(String::from),
            disconnected_at: None,
            ..Default::default()
        });
        drop(s);
        self.broadcast().await;
        Ok(node_id)
    }

    pub async fn local_session_stop(&self, node_id: &str) -> Result<(), CoreError> {
        {
            let mut s = self.state.write().await;
            let node = s.node(node_id).ok_or(CoreError::LocalSessionNotFound)?;
            if node.node_type != NodeType::Local {
                return Err(CoreError::LocalSessionNotFound);
            }
            s.nodes.retain(|n| n.id != node_id);
            if s.selected_node_id.as_deref() == Some(node_id) {
                s.selected_node_id = s.nodes.iter().find(|n| n.connected).map(|n| n.id.clone());
            }
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn local_session_disconnect(&self, node_id: &str) -> Result<(), CoreError> {
        {
            let mut s = self.state.write().await;
            let node = s.node_mut(node_id).ok_or(CoreError::LocalSessionNotFound)?;
            if node.node_type != NodeType::Local {
                return Err(CoreError::LocalSessionNotFound);
            }
            node.connected = false;
            node.disconnected_at = Some(current_unix_timestamp());
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn local_session_update(
        &self,
        node_id: &str,
        queue: Vec<Track>,
        current_index: Option<usize>,
        position_secs: f64,
        status: PlaybackStatus,
        volume: u8,
        repeat: RepeatMode,
        shuffle: bool,
    ) -> Result<(), CoreError> {
        {
            let mut s = self.state.write().await;
            let node = s.node_mut(node_id).ok_or(CoreError::LocalSessionNotFound)?;
            if node.node_type != NodeType::Local {
                return Err(CoreError::LocalSessionNotFound);
            }
            node.connected = true;
            node.queue = queue;
            node.current_index = current_index;
            node.position_secs = position_secs;
            node.status = status;
            node.volume = volume;
            node.repeat = repeat;
            node.shuffle = shuffle;
            node.disconnected_at = None;
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn handoff(&self, from_node_id: &str, to_node_id: &str) -> Result<(), CoreError> {
        {
            let s = self.state.read().await;
            let _from = s.node(from_node_id).ok_or(CoreError::NodeNotFound)?;
            let to = s.node(to_node_id).ok_or(CoreError::NodeNotFound)?;
            if !to.connected {
                return Err(CoreError::HandoffFailed("target node not connected".into()));
            }
        }

        let (queue, current_index, position_secs) = {
            let s = self.state.read().await;
            let from_node = s.node(from_node_id).ok_or(CoreError::NodeNotFound)?;
            (
                from_node.queue.clone(),
                from_node.current_index,
                from_node.position_secs,
            )
        };

        {
            let mut s = self.state.write().await;
            if let Some(node) = s.node_mut(to_node_id) {
                node.queue = queue.clone();
                node.current_index = current_index;
                node.position_secs = position_secs;
            }
        }

        let target_type = {
            let s = self.state.read().await;
            s.node(to_node_id).map(|n| n.node_type)
        };

        if target_type == Some(NodeType::Remote) {
            self.sync_output_to_global(to_node_id).await?;
            for o in self.each_output(to_node_id).await? {
                o.play().await?;
                if position_secs > 0.0 {
                    o.seek(position_secs).await?;
                }
            }
            let mut s = self.state.write().await;
            if let Some(node) = s.node_mut(to_node_id) {
                node.status = PlaybackStatus::Playing;
            }
        }

        self.broadcast().await;
        Ok(())
    }

    pub async fn cleanup_disconnected_nodes(&self, max_age: std::time::Duration) {
        let cutoff = current_unix_timestamp().saturating_sub(max_age.as_secs() as i64);

        let removed_ids = {
            let mut s = self.state.write().await;
            let removed_ids: Vec<String> = s
                .nodes
                .iter()
                .filter(|node| {
                    node.node_type == NodeType::Local
                        && !node.connected
                        && node
                            .disconnected_at
                            .is_some_and(|disconnected_at| disconnected_at <= cutoff)
                })
                .map(|node| node.id.clone())
                .collect();

            if removed_ids.is_empty() {
                return;
            }

            s.nodes
                .retain(|node| !removed_ids.iter().any(|removed_id| removed_id == &node.id));
            if s.selected_node_id
                .as_ref()
                .is_some_and(|selected| removed_ids.iter().any(|removed_id| removed_id == selected))
            {
                s.selected_node_id = s
                    .nodes
                    .iter()
                    .find(|node| node.connected)
                    .map(|node| node.id.clone());
            }

            removed_ids
        };

        self.transport_state
            .write()
            .await
            .retain(|id, _| !removed_ids.iter().any(|removed_id| removed_id == id));
        self.outputs
            .write()
            .await
            .retain(|id, _| !removed_ids.iter().any(|removed_id| removed_id == id));
        self.broadcast().await;
    }

    pub async fn select_node(&self, node_id: &str) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        if s.node(node_id).is_none() {
            return Err(CoreError::NodeNotFound);
        }

        let previous_node_id = s.selected_node_id.clone();
        let same_target = previous_node_id.as_deref() == Some(node_id);

        if same_target {
            return Ok(());
        }

        let (resume_status, resume_position_secs) = previous_node_id
            .as_deref()
            .and_then(|id| s.node(id))
            .map(|n| (n.status.clone(), n.position_secs))
            .unwrap_or((PlaybackStatus::Stopped, 0.0));
        info!(target_node_id = %node_id, previous_node_id = ?previous_node_id, resume_status = ?resume_status, "select_node: switching");

        s.selected_node_id = Some(node_id.to_string());

        for node in s.nodes.iter_mut() {
            node.status = PlaybackStatus::Stopped;
            if node.id != node_id {
                node.position_secs = 0.0;
            }
        }
        drop(s);

        if let Some(previous_id) = previous_node_id.as_deref() {
            info!(previous_node_id = %previous_id, "select_node: stopping previous node outputs");
            for o in self.each_output(previous_id).await? {
                o.stop().await?;
            }
        }

        info!(target_node_id = %node_id, "select_node: syncing target node to global queue");
        self.sync_output_to_global(node_id).await?;

        match resume_status {
            PlaybackStatus::Playing => {
                info!(target_node_id = %node_id, "select_node: resuming target node playback");
                for o in self.each_output(node_id).await? {
                    o.play().await?;
                    if resume_position_secs > 0.0 {
                        o.seek(resume_position_secs).await?;
                    }
                }
                let mut s = self.state.write().await;
                if let Some(active) = s.node_mut(node_id) {
                    active.status = PlaybackStatus::Playing;
                    active.position_secs = resume_position_secs;
                }
            }
            PlaybackStatus::Paused => {
                for o in self.each_output(node_id).await? {
                    o.play().await?;
                    if resume_position_secs > 0.0 {
                        o.seek(resume_position_secs).await?;
                    }
                    o.pause().await?;
                }
                let mut s = self.state.write().await;
                if let Some(active) = s.node_mut(node_id) {
                    active.status = PlaybackStatus::Paused;
                    active.position_secs = resume_position_secs;
                }
            }
            _ => {
                let mut s = self.state.write().await;
                if let Some(active) = s.node_mut(node_id) {
                    active.position_secs = resume_position_secs;
                }
            }
        }

        self.broadcast().await;
        Ok(())
    }

    pub async fn select_output_node(&self, node_id: &str) -> Result<(), CoreError> {
        self.select_node(node_id).await
    }

    pub async fn get_node(&self, id: &str) -> Option<Node> {
        self.state.read().await.node(id).cloned()
    }

    async fn each_output(&self, node_id: &str) -> Result<Vec<Arc<dyn AudioOutput>>, CoreError> {
        let outputs = self.outputs.read().await;
        outputs
            .get(node_id)
            .map(|slot| vec![Arc::clone(&slot.output)])
            .ok_or(CoreError::NoActiveOutput)
    }

    async fn active_node_id(&self) -> Result<String, CoreError> {
        let s = self.state.read().await;
        let node_id = s
            .selected_node_id
            .clone()
            .ok_or(CoreError::NoActiveOutput)?;
        let node = s.node(&node_id).ok_or(CoreError::NodeNotFound)?;
        if !node.connected {
            return Err(CoreError::NoActiveOutput);
        }
        Ok(node_id)
    }

    async fn active_outputs(&self) -> Result<Vec<Arc<dyn AudioOutput>>, CoreError> {
        let node_id = self.active_node_id().await?;
        self.each_output(&node_id).await
    }

    async fn maybe_active_outputs(&self) -> Option<Vec<Arc<dyn AudioOutput>>> {
        self.active_outputs().await.ok()
    }

    pub async fn play(&self) -> Result<(), CoreError> {
        let node_id = self.active_node_id().await?;
        self.play_node(&node_id).await
    }

    pub async fn pause(&self) -> Result<(), CoreError> {
        let node_id = self.active_node_id().await?;
        self.pause_node(&node_id).await
    }

    pub async fn stop(&self) -> Result<(), CoreError> {
        let node_id = self.active_node_id().await?;
        self.stop_node(&node_id).await
    }

    pub async fn seek(&self, position_secs: f64) -> Result<(), CoreError> {
        let node_id = self.active_node_id().await?;
        self.seek_node(&node_id, position_secs).await
    }

    pub async fn set_volume(&self, volume: u8) -> Result<(), CoreError> {
        let node_id = self.active_node_id().await?;
        self.set_node_volume(&node_id, volume).await
    }

    #[instrument(skip(self))]
    pub async fn play_node(&self, node_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? {
            o.play().await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            if !node.queue.is_empty() && node.current_index.is_none() {
                node.current_index = Some(0);
            }
            if node.current_index.is_some() {
                node.status = PlaybackStatus::Playing;
            }
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn pause_node(&self, node_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? {
            o.pause().await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Paused;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn stop_node(&self, node_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? {
            o.stop().await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Stopped;
            node.position_secs = 0.0;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn next(&self) -> Result<(), CoreError> {
        let active_node_id = self.active_node_id().await?;
        let (queue_paths, projection_start, loaded_len) = {
            let mut s = self.state.write().await;
            let (next_index, queue_len, repeat) = {
                let node = s.node(&active_node_id).ok_or(CoreError::NodeNotFound)?;
                (node.current_index, node.queue.len(), node.repeat)
            };
            if queue_len == 0 {
                return Err(CoreError::QueueEmpty);
            }
            let next = match next_index {
                Some(i) => match repeat {
                    RepeatMode::Off => {
                        if i + 1 < queue_len {
                            i + 1
                        } else {
                            return Err(CoreError::QueueEmpty);
                        }
                    }
                    RepeatMode::One => i,
                    RepeatMode::All => (i + 1) % queue_len,
                },
                None => 0,
            };
            if let Some(node) = s.node_mut(&active_node_id) {
                node.current_index = Some(next);
            }
            for node in s.nodes.iter_mut() {
                node.position_secs = 0.0;
            }
            let queue = {
                let node = s.node(&active_node_id).ok_or(CoreError::NodeNotFound)?;
                Self::build_queue_file_paths(&node.queue, node.current_index)
            };
            let loaded_len = queue.len();
            let projection_start = s.node(&active_node_id).and_then(|node| node.current_index);
            (queue, projection_start, loaded_len)
        };
        info!(projection_start = ?projection_start, loaded_len, "core.next: rebuilt queue projection");
        let projection_generation = self
            .rebuild_projection_state(&active_node_id, projection_start, loaded_len)
            .await;
        info!(projection_generation, "core.next: rebuilt transport state");
        for o in self.active_outputs().await? {
            info!("core.next: sending set_queue to active output");
            o.set_queue(&queue_paths, projection_generation).await?;
        }
        for o in self.active_outputs().await? {
            info!("core.next: sending play to active output");
            o.play().await?;
        }
        let mut s = self.state.write().await;
        info!("core.next: reacquired state write for status update");
        if let Some(node) = s.node_mut(&active_node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        info!("core.next: done");
        Ok(())
    }

    pub async fn previous(&self) -> Result<(), CoreError> {
        let active_node_id = self.active_node_id().await?;
        let (queue_paths, projection_start, loaded_len) = {
            let mut s = self.state.write().await;
            let (current_index, queue_len, repeat) = {
                let node = s.node(&active_node_id).ok_or(CoreError::NodeNotFound)?;
                (node.current_index, node.queue.len(), node.repeat)
            };
            if queue_len == 0 {
                return Err(CoreError::QueueEmpty);
            }
            let prev = match current_index {
                Some(0) | None => match repeat {
                    RepeatMode::Off => return Err(CoreError::QueueEmpty),
                    RepeatMode::One => 0,
                    RepeatMode::All => queue_len - 1,
                },
                Some(i) => i - 1,
            };
            if let Some(node) = s.node_mut(&active_node_id) {
                node.current_index = Some(prev);
            }
            for node in s.nodes.iter_mut() {
                node.position_secs = 0.0;
            }
            let queue = {
                let node = s.node(&active_node_id).ok_or(CoreError::NodeNotFound)?;
                Self::build_queue_file_paths(&node.queue, node.current_index)
            };
            let loaded_len = queue.len();
            let projection_start = s.node(&active_node_id).and_then(|node| node.current_index);
            (queue, projection_start, loaded_len)
        };
        let projection_generation = self
            .rebuild_projection_state(&active_node_id, projection_start, loaded_len)
            .await;
        for o in self.active_outputs().await? {
            o.set_queue(&queue_paths, projection_generation).await?;
        }
        for o in self.active_outputs().await? {
            o.play().await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(&active_node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn seek_node(&self, node_id: &str, position_secs: f64) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? {
            o.seek(position_secs).await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.position_secs = position_secs;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_node_volume(&self, node_id: &str, volume: u8) -> Result<(), CoreError> {
        if volume > 100 {
            return Err(CoreError::InvalidVolume);
        }
        for o in self.each_output(node_id).await? {
            o.set_volume(volume).await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.volume = volume;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_shuffle(&self, shuffle: bool) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.selected_node_mut().ok_or(CoreError::NoActiveOutput)?;
        node.shuffle = shuffle;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_repeat(&self, repeat: RepeatMode) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.selected_node_mut().ok_or(CoreError::NoActiveOutput)?;
        node.repeat = repeat;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_to_queue(&self, track: Track) -> Result<(), CoreError> {
        let file_paths = vec![track.file_path.clone()];
        let selected_node_id = {
            let s = self.state.read().await;
            s.selected_node_id
                .clone()
                .ok_or(CoreError::NoActiveOutput)?
        };
        {
            let mut s = self.state.write().await;
            let node = s
                .node_mut(&selected_node_id)
                .ok_or(CoreError::NodeNotFound)?;
            node.queue.push(track);
        }
        if let Some(outputs) = self.maybe_active_outputs().await {
            for o in outputs {
                o.add(&file_paths).await?;
            }
            self.extend_projection_loaded_len_active(file_paths.len())
                .await;
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_tracks_to_queue(&self, tracks: Vec<Track>) -> Result<(), CoreError> {
        if tracks.is_empty() {
            return Ok(());
        }
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        let selected_node_id = {
            let s = self.state.read().await;
            s.selected_node_id
                .clone()
                .ok_or(CoreError::NoActiveOutput)?
        };
        {
            let mut s = self.state.write().await;
            let node = s
                .node_mut(&selected_node_id)
                .ok_or(CoreError::NodeNotFound)?;
            node.queue.extend(tracks);
        }
        if let Some(outputs) = self.maybe_active_outputs().await {
            for o in outputs {
                o.add(&file_paths).await?;
            }
            self.extend_projection_loaded_len_active(file_paths.len())
                .await;
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn clear_queue(&self) -> Result<(), CoreError> {
        if let Ok(active_node_id) = self.active_node_id().await {
            let projection_generation = self
                .rebuild_projection_state(&active_node_id, None, 0)
                .await;
            for o in self.each_output(&active_node_id).await? {
                o.set_queue(&[], projection_generation).await?;
            }
        }
        let mut s = self.state.write().await;
        let node = s.selected_node_mut().ok_or(CoreError::NoActiveOutput)?;
        node.queue.clear();
        node.current_index = None;
        node.position_secs = 0.0;
        node.status = PlaybackStatus::Stopped;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn remove_from_queue(&self, index: usize) -> Result<(), CoreError> {
        let mpd_index = {
            let mut s = self.state.write().await;
            let node = s.selected_node_mut().ok_or(CoreError::NoActiveOutput)?;
            if index >= node.queue.len() {
                return Err(CoreError::QueueIndexOutOfBounds);
            }
            let mpd_index = node
                .current_index
                .map(|ci| index.saturating_sub(ci))
                .unwrap_or(index);
            node.queue.remove(index);
            match node.current_index {
                Some(ci) if ci == index && node.queue.is_empty() => {
                    node.current_index = None;
                    node.status = PlaybackStatus::Stopped;
                }
                Some(ci) if ci == index => {
                    node.current_index = Some(ci.min(node.queue.len() - 1));
                }
                Some(ci) if ci > index => {
                    node.current_index = Some(ci - 1);
                }
                _ => {}
            }
            mpd_index
        };
        if let Some(outputs) = self.maybe_active_outputs().await {
            for o in outputs {
                o.remove(mpd_index).await?;
            }
            self.decrement_projection_loaded_len_active(1).await;
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn move_in_queue(&self, from: usize, to: usize) -> Result<(), CoreError> {
        let (mpd_from, mpd_to) = {
            let mut s = self.state.write().await;
            let node = s.selected_node_mut().ok_or(CoreError::NoActiveOutput)?;
            if from >= node.queue.len() || to >= node.queue.len() {
                return Err(CoreError::QueueIndexOutOfBounds);
            }
            let mpd_from = node
                .current_index
                .map(|ci| from.saturating_sub(ci))
                .unwrap_or(from);
            let mpd_to = node
                .current_index
                .map(|ci| to.saturating_sub(ci))
                .unwrap_or(to);
            let track = node.queue.remove(from);
            node.queue.insert(to, track);
            match node.current_index {
                Some(ci) if ci == from => {
                    node.current_index = Some(to);
                }
                Some(ci) if from < ci && ci <= to => {
                    node.current_index = Some(ci - 1);
                }
                Some(ci) if to <= ci && ci < from => {
                    node.current_index = Some(ci + 1);
                }
                _ => {}
            }
            (mpd_from, mpd_to)
        };
        if let Some(outputs) = self.maybe_active_outputs().await {
            for o in outputs {
                o.move_track(mpd_from, mpd_to).await?;
            }
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn play_index(&self, index: usize) -> Result<(), CoreError> {
        let active_node_id = self.active_node_id().await?;
        let (queue_paths, projection_start, loaded_len) = {
            let mut s = self.state.write().await;
            let queue_len = s
                .node(&active_node_id)
                .ok_or(CoreError::NodeNotFound)?
                .queue
                .len();
            if index >= queue_len {
                return Err(CoreError::QueueIndexOutOfBounds);
            }
            if let Some(node) = s.node_mut(&active_node_id) {
                node.current_index = Some(index);
            }
            for node in s.nodes.iter_mut() {
                node.position_secs = 0.0;
            }
            let queue = {
                let node = s.node(&active_node_id).ok_or(CoreError::NodeNotFound)?;
                Self::build_queue_file_paths(&node.queue, node.current_index)
            };
            let loaded_len = queue.len();
            let projection_start = s.node(&active_node_id).and_then(|node| node.current_index);
            (queue, projection_start, loaded_len)
        };
        let projection_generation = self
            .rebuild_projection_state(&active_node_id, projection_start, loaded_len)
            .await;
        for o in self.active_outputs().await? {
            o.set_queue(&queue_paths, projection_generation).await?;
        }
        for o in self.active_outputs().await? {
            o.play().await?;
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(&active_node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_queue(
        &self,
        tracks: Vec<Track>,
        start_index: Option<usize>,
    ) -> Result<(), CoreError> {
        let selected_node_id = {
            let s = self.state.read().await;
            s.selected_node_id
                .clone()
                .ok_or(CoreError::NoActiveOutput)?
        };
        let active_node_id = self.active_node_id().await.ok();
        let start = start_index.unwrap_or(0);
        let head = tracks
            .get(start)
            .map(|t| vec![t.file_path.clone()])
            .unwrap_or_default();
        let tail = tracks
            .iter()
            .skip(start + 1)
            .map(|t| t.file_path.clone())
            .collect::<Vec<_>>();
        let rotated: Vec<String> = {
            let mut p = head;
            p.extend(tail);
            p
        };
        let projection_start = if rotated.is_empty() {
            None
        } else {
            start_index
        };
        if let Some(active_node_id) = active_node_id.as_ref() {
            let projection_generation = self
                .rebuild_projection_state(active_node_id, projection_start, rotated.len())
                .await;
            for o in self.each_output(active_node_id).await? {
                o.set_queue(&rotated, projection_generation).await?;
            }
        }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(&selected_node_id) {
            node.queue = tracks;
            node.current_index = start_index;
            node.position_secs = 0.0;
            node.status = PlaybackStatus::Stopped;
        } else {
            return Err(CoreError::NodeNotFound);
        }
        if start_index.is_some() {
            if let Some(active_node_id) = active_node_id.as_ref() {
                if let Some(node) = s.node_mut(active_node_id) {
                    node.status = PlaybackStatus::Playing;
                }
            }
        }
        drop(s);
        if start_index.is_some() {
            if let Some(outputs) = self.maybe_active_outputs().await {
                for o in outputs {
                    o.play().await?;
                }
            }
        }
        self.broadcast().await;
        Ok(())
    }

    pub async fn sync_output_to_global(&self, node_id: &str) -> Result<(), CoreError> {
        let (queue, projection_start, loaded_len, volume) = {
            let s = self.state.read().await;
            let node = s.node(node_id).ok_or(CoreError::NodeNotFound)?;
            (
                Self::build_queue_file_paths(&node.queue, node.current_index),
                node.current_index,
                node.queue.len(),
                node.volume,
            )
        };

        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, loaded_len)
            .await;

        info!(node_id = %node_id, queue_len = queue.len(), projection_start = ?projection_start, loaded_len, volume, projection_generation, "sync_output_to_global: applying state to node");

        for o in self.each_output(node_id).await? {
            info!(node_id = %node_id, volume, "sync_output_to_global: set_volume");
            o.set_volume(volume).await?;
            info!(node_id = %node_id, queue_len = queue.len(), projection_generation, "sync_output_to_global: set_queue");
            o.set_queue(&queue, projection_generation).await?;
        }

        Ok(())
    }

    async fn apply_output_runtime_state(
        &self,
        node_id: &str,
        status: PlaybackStatus,
        position_secs: f64,
    ) -> Result<(), CoreError> {
        match status {
            PlaybackStatus::Playing | PlaybackStatus::Loading => {
                for o in self.each_output(node_id).await? {
                    o.play().await?;
                    if position_secs > 0.0 {
                        o.seek(position_secs).await?;
                    }
                }
            }
            PlaybackStatus::Paused => {
                for o in self.each_output(node_id).await? {
                    o.play().await?;
                    if position_secs > 0.0 {
                        o.seek(position_secs).await?;
                    }
                    o.pause().await?;
                }
            }
            PlaybackStatus::Stopped => {
                for o in self.each_output(node_id).await? {
                    o.stop().await?;
                }
            }
        }

        Ok(())
    }

    fn build_queue_file_paths(queue: &[Track], current_index: Option<usize>) -> Vec<String> {
        let start = current_index.unwrap_or(0);
        let tail = if current_index.is_some() {
            queue
                .iter()
                .skip(start + 1)
                .map(|t| t.file_path.clone())
                .collect::<Vec<_>>()
        } else {
            queue
                .iter()
                .skip(1)
                .map(|t| t.file_path.clone())
                .collect::<Vec<_>>()
        };
        let head = queue
            .get(start)
            .map(|t| vec![t.file_path.clone()])
            .unwrap_or_default();
        let mut paths = head;
        paths.extend(tail);
        paths
    }

    async fn extend_projection_loaded_len_active(&self, added_len: usize) {
        if added_len == 0 {
            return;
        }
        let node_id = match self.active_node_id().await {
            Ok(node_id) => node_id,
            Err(_) => return,
        };

        let mut transport = self.transport_state.write().await;
        if let Some(entry) = transport.get_mut(&node_id) {
            entry.loaded_len = entry.loaded_len.saturating_add(added_len);
        }
    }

    async fn decrement_projection_loaded_len_active(&self, removed_len: usize) {
        if removed_len == 0 {
            return;
        }
        let node_id = match self.active_node_id().await {
            Ok(node_id) => node_id,
            Err(_) => return,
        };

        let mut transport = self.transport_state.write().await;
        if let Some(entry) = transport.get_mut(&node_id) {
            entry.loaded_len = entry.loaded_len.saturating_sub(removed_len);
            if entry.loaded_len == 0 {
                entry.projection_start_index = None;
            }
        }
    }

    async fn broadcast(&self) {
        let snapshot = {
            let mut s = self.state.write().await;
            s.sync_top_level_from_selected_node();
            drop(s);
            self.state.read().await.clone()
        };
        let node_summary = snapshot
            .nodes
            .iter()
            .map(|n| format!("{}:{}:{}", n.id, n.name, n.connected))
            .collect::<Vec<_>>()
            .join(", ");
        info!(selected_node_id = ?snapshot.selected_node_id, nodes = %node_summary, "core.broadcast state");
        for b in &self.broadcasters {
            b.on_state_changed(&snapshot).await;
        }
        for p in &self.persisters {
            p.persist(&snapshot).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockOutput {
        last_set_queue: Mutex<Option<(Vec<String>, u64)>>,
    }

    impl MockOutput {
        fn new() -> Self {
            Self {
                last_set_queue: Mutex::new(None),
            }
        }

        fn last_projection_generation(&self) -> u64 {
            self.last_set_queue
                .lock()
                .unwrap()
                .as_ref()
                .map(|(_, generation)| *generation)
                .unwrap_or(0)
        }
    }

    #[async_trait]
    impl AudioOutput for MockOutput {
        async fn play(&self) -> Result<(), CoreError> {
            Ok(())
        }

        async fn pause(&self) -> Result<(), CoreError> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), CoreError> {
            Ok(())
        }

        async fn seek(&self, _position_secs: f64) -> Result<(), CoreError> {
            Ok(())
        }

        async fn set_volume(&self, _volume: u8) -> Result<(), CoreError> {
            Ok(())
        }

        async fn set_queue(
            &self,
            file_paths: &[String],
            projection_generation: u64,
        ) -> Result<(), CoreError> {
            *self.last_set_queue.lock().unwrap() =
                Some((file_paths.to_vec(), projection_generation));
            Ok(())
        }

        async fn add(&self, _file_paths: &[String]) -> Result<(), CoreError> {
            Ok(())
        }

        async fn remove(&self, _index: usize) -> Result<(), CoreError> {
            Ok(())
        }

        async fn move_track(&self, _from: usize, _to: usize) -> Result<(), CoreError> {
            Ok(())
        }
    }

    fn sample_track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            file_path: format!("/music/{id}.flac"),
            album_id: None,
            title: Some(id.to_string()),
            artist: None,
            album_artist: None,
            album_title: None,
            composer: None,
            genre: None,
            track_number: None,
            disc_number: None,
            duration_secs: None,
            format: None,
            sample_rate: None,
        }
    }

    async fn setup_core_with_node() -> (Core, Arc<MockOutput>) {
        let output = Arc::new(MockOutput::new());
        let core = Core::new(
            vec![(
                "default".to_string(),
                output.clone() as Arc<dyn AudioOutput>,
            )],
            vec![],
        );
        core.add_node(Node {
            id: "default".to_string(),
            name: "node".to_string(),
            ..Default::default()
        })
        .await;
        (core, output)
    }

    #[tokio::test]
    async fn sync_node_state_maps_projection_song_to_global_index() {
        let (core, output) = setup_core_with_node().await;
        core.set_queue(
            vec![
                sample_track("a"),
                sample_track("b"),
                sample_track("c"),
                sample_track("d"),
            ],
            Some(1),
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.sync_node_state(
            "default",
            PlaybackStatus::Playing,
            12.0,
            70,
            Some(0),
            generation,
        )
        .await;

        let s = core.state.read().await;
        assert_eq!(s.current_index, Some(1));
        let node = s.node("default").unwrap();
        assert_eq!(node.position_secs, 12.0);
        assert_eq!(node.volume, 70);
    }

    #[tokio::test]
    async fn sync_node_state_ignores_stale_projection_generation() {
        let (core, output) = setup_core_with_node().await;
        core.set_queue(
            vec![sample_track("a"), sample_track("b"), sample_track("c")],
            Some(1),
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.sync_node_state(
            "default",
            PlaybackStatus::Playing,
            5.0,
            60,
            Some(1),
            generation.wrapping_sub(1),
        )
        .await;

        let s = core.state.read().await;
        assert_eq!(s.current_index, Some(1));
        let node = s.node("default").unwrap();
        assert_eq!(node.position_secs, 5.0);
        assert_eq!(node.volume, 60);
    }

    #[tokio::test]
    async fn add_to_queue_keeps_projection_base_and_allows_auto_advance() {
        let (core, output) = setup_core_with_node().await;
        core.set_queue(
            vec![
                sample_track("a"),
                sample_track("b"),
                sample_track("c"),
                sample_track("d"),
            ],
            None,
        )
        .await
        .unwrap();
        core.play_index(1).await.unwrap();

        let generation = output.last_projection_generation();

        core.add_to_queue(sample_track("e")).await.unwrap();

        core.sync_node_state(
            "default",
            PlaybackStatus::Playing,
            0.0,
            50,
            Some(2),
            generation,
        )
        .await;

        let s = core.state.read().await;
        assert_eq!(s.current_index, Some(3));
    }

    #[tokio::test]
    async fn sync_node_state_ignores_out_of_bounds_mpd_song_index() {
        let (core, output) = setup_core_with_node().await;
        core.set_queue(
            vec![sample_track("a"), sample_track("b"), sample_track("c")],
            Some(1),
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.sync_node_state(
            "default",
            PlaybackStatus::Playing,
            8.0,
            40,
            Some(5),
            generation,
        )
        .await;

        let s = core.state.read().await;
        assert_eq!(s.current_index, Some(1));
        let node = s.node("default").unwrap();
        assert_eq!(node.position_secs, 8.0);
    }

    #[tokio::test]
    async fn local_session_start_deduplicates_same_device_id() {
        let core = Core::new(vec![], vec![]);
        core.restore_state(PlaybackState {
            nodes: vec![
                Node {
                    id: "local-device-1".to_string(),
                    name: "Old iPhone".to_string(),
                    connected: false,
                    node_type: NodeType::Local,
                    device_id: Some("device-1".to_string()),
                    disconnected_at: Some(current_unix_timestamp().saturating_sub(60)),
                    ..Default::default()
                },
                Node {
                    id: "local-device-1-dup".to_string(),
                    name: "Old iPhone Duplicate".to_string(),
                    connected: false,
                    node_type: NodeType::Local,
                    device_id: Some("device-1".to_string()),
                    disconnected_at: Some(current_unix_timestamp().saturating_sub(120)),
                    ..Default::default()
                },
            ],
            selected_node_id: None,
            queue: vec![],
            current_index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
        })
        .await;

        let node_id = core
            .local_session_start("Kana's iPhone", Some("device-1"))
            .await
            .unwrap();

        let s = core.state.read().await;
        let matching_nodes: Vec<_> = s
            .nodes
            .iter()
            .filter(|node| node.device_id.as_deref() == Some("device-1"))
            .collect();

        assert_eq!(node_id, "local-device-1");
        assert_eq!(matching_nodes.len(), 1);
        assert_eq!(matching_nodes[0].name, "Kana's iPhone");
        assert!(matching_nodes[0].connected);
        assert_eq!(matching_nodes[0].disconnected_at, None);
    }

    #[tokio::test]
    async fn cleanup_disconnected_nodes_removes_expired_local_nodes() {
        let core = Core::new(vec![], vec![]);
        core.restore_state(PlaybackState {
            nodes: vec![
                Node {
                    id: "local-stale".to_string(),
                    name: "Stale Local".to_string(),
                    connected: false,
                    node_type: NodeType::Local,
                    device_id: Some("device-stale".to_string()),
                    disconnected_at: Some(current_unix_timestamp().saturating_sub(120)),
                    ..Default::default()
                },
                Node {
                    id: "local-fresh".to_string(),
                    name: "Fresh Local".to_string(),
                    connected: false,
                    node_type: NodeType::Local,
                    device_id: Some("device-fresh".to_string()),
                    disconnected_at: Some(current_unix_timestamp().saturating_sub(5)),
                    ..Default::default()
                },
            ],
            selected_node_id: None,
            queue: vec![],
            current_index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
        })
        .await;

        core.cleanup_disconnected_nodes(std::time::Duration::from_secs(30))
            .await;

        let s = core.state.read().await;
        assert!(s.node("local-stale").is_none());
        assert!(s.node("local-fresh").is_some());
    }
}
