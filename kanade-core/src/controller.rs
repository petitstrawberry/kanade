use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::instrument;

use crate::{
    error::CoreError,
    model::{Node, PlaybackStatus, RepeatMode, Track},
    ports::{AudioOutput, EventBroadcaster},
    state::PlaybackState,
};

pub struct Core {
    state: Arc<RwLock<PlaybackState>>,
    outputs: Arc<RwLock<HashMap<String, Arc<dyn AudioOutput>>>>,
    broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    queue_generation: Arc<AtomicU64>,
}

impl Core {
    pub fn new(
        outputs: Vec<(String, Arc<dyn AudioOutput>)>,
        broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(PlaybackState { nodes: Vec::new() })),
            outputs: Arc::new(RwLock::new(outputs.into_iter().collect())),
            broadcasters,
            queue_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Register a new output at runtime. Safe to call on an `Arc<Core>`.
    pub async fn register_output(&self, id: String, output: Arc<dyn AudioOutput>) {
        self.outputs.write().await.insert(id, output);
    }

    /// Remove a previously registered output. Safe to call on an `Arc<Core>`.
    pub async fn unregister_output(&self, id: &str) {
        self.outputs.write().await.remove(id);
    }

    pub fn add_broadcaster(&mut self, b: Arc<dyn EventBroadcaster>) {
        self.broadcasters.push(b);
    }

    /// Apply an external state update (e.g. from a remote output node) to the
    /// named node, then broadcast the change to all event listeners.
    pub async fn sync_node_state(
        &self,
        node_id: &str,
        status: PlaybackStatus,
        position_secs: f64,
        volume: u8,
        current_index: Option<usize>,
    ) {
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = status;
            node.position_secs = position_secs;
            node.volume = volume;
            node.current_index = current_index;
        }
        drop(s);
        self.broadcast().await;
    }

    pub fn state_handle(&self) -> Arc<RwLock<PlaybackState>> {
        Arc::clone(&self.state)
    }

    pub fn queue_generation(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.queue_generation)
    }

    fn bump_queue_generation(&self) {
        self.queue_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn add_node(&self, node: Node) {
        self.state.write().await.nodes.push(node);
    }

    pub async fn remove_node(&self, node_id: &str) {
        self.state
            .write()
            .await
            .nodes
            .retain(|n| n.id != node_id);
    }

    pub async fn get_node(&self, id: &str) -> Option<Node> {
        self.state.read().await.node(id).cloned()
    }

    async fn each_output(&self, node_id: &str) -> Result<Vec<Arc<dyn AudioOutput>>, CoreError> {
        let s = self.state.read().await;
        let node = s.node(node_id).ok_or(CoreError::NodeNotFound)?;
        let ids = node.output_ids.clone();
        drop(s);
        let outputs = self.outputs.read().await;
        let mut outs = Vec::new();
        for id in &ids {
            if let Some(o) = outputs.get(id) {
                outs.push(Arc::clone(o));
            }
        }
        Ok(outs)
    }

    #[instrument(skip(self))]
    pub async fn play_node(&self, node_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? { o.play().await?; }
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
        for o in self.each_output(node_id).await? { o.pause().await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Paused;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn stop_node(&self, node_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? { o.stop().await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Stopped;
            node.position_secs = 0.0;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn next_node(&self, node_id: &str) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        if node.queue.is_empty() {
            return Err(CoreError::QueueEmpty);
        }
        let next = match node.current_index {
            Some(i) => match node.repeat {
                RepeatMode::Off => {
                    if i + 1 < node.queue.len() { i + 1 } else { return Err(CoreError::QueueEmpty) }
                }
                RepeatMode::One => i,
                RepeatMode::All => (i + 1) % node.queue.len(),
            },
            None => 0,
        };
        node.current_index = Some(next);
        node.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(node);
        drop(s);
        self.bump_queue_generation();
        for o in self.each_output(node_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(node_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn previous_node(&self, node_id: &str) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        if node.queue.is_empty() {
            return Err(CoreError::QueueEmpty);
        }
        let prev = match node.current_index {
            Some(0) | None => match node.repeat {
                RepeatMode::Off => return Err(CoreError::QueueEmpty),
                RepeatMode::One => 0,
                RepeatMode::All => node.queue.len() - 1,
            },
            Some(i) => i - 1,
        };
        node.current_index = Some(prev);
        node.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(node);
        drop(s);
        self.bump_queue_generation();
        for o in self.each_output(node_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(node_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn seek_node(&self, node_id: &str, position_secs: f64) -> Result<(), CoreError> {
        for o in self.each_output(node_id).await? { o.seek(position_secs).await?; }
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        node.position_secs = position_secs;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_node_volume(&self, node_id: &str, volume: u8) -> Result<(), CoreError> {
        if volume > 100 {
            return Err(CoreError::InvalidVolume);
        }
        for o in self.each_output(node_id).await? { o.set_volume(volume).await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.volume = volume;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_node_shuffle(&self, node_id: &str, shuffle: bool) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.shuffle = shuffle;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_node_repeat(&self, node_id: &str, repeat: RepeatMode) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.repeat = repeat;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_to_node_queue(&self, node_id: &str, track: Track) -> Result<(), CoreError> {
        let file_paths = vec![track.file_path.clone()];
        for o in self.each_output(node_id).await? { o.add(&file_paths).await?; }
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        node.queue.push(track);
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_tracks_to_node_queue(&self, node_id: &str, tracks: Vec<Track>) -> Result<(), CoreError> {
        if tracks.is_empty() {
            return Ok(());
        }
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        for o in self.each_output(node_id).await? { o.add(&file_paths).await?; }
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        node.queue.extend(tracks);
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn clear_node_queue(&self, node_id: &str) -> Result<(), CoreError> {
        self.bump_queue_generation();
        for o in self.each_output(node_id).await? { o.set_queue(&[]).await?; }
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        node.queue.clear();
        node.current_index = None;
        node.position_secs = 0.0;
        node.status = PlaybackStatus::Stopped;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn remove_from_node_queue(&self, node_id: &str, index: usize) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        if index >= node.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        // MPD playlist offset: position 0 = core current_index
        let mpd_index = node.current_index.map(|ci| index.saturating_sub(ci)).unwrap_or(index);
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
        drop(s);
        for o in self.each_output(node_id).await? { o.remove(mpd_index).await?; }
        self.broadcast().await;
        Ok(())
    }

    pub async fn move_in_node_queue(
        &self,
        node_id: &str,
        from: usize,
        to: usize,
    ) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        if from >= node.queue.len() || to >= node.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        let mpd_from = node.current_index.map(|ci| from.saturating_sub(ci)).unwrap_or(from);
        let mpd_to = node.current_index.map(|ci| to.saturating_sub(ci)).unwrap_or(to);
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
        drop(s);
        for o in self.each_output(node_id).await? { o.move_track(mpd_from, mpd_to).await?; }
        self.broadcast().await;
        Ok(())
    }

    pub async fn play_node_index(&self, node_id: &str, index: usize) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        if index >= node.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        node.current_index = Some(index);
        node.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(node);
        drop(s);
        self.bump_queue_generation();
        for o in self.each_output(node_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(node_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(node) = s.node_mut(node_id) {
            node.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_node_queue(
        &self,
        node_id: &str,
        tracks: Vec<Track>,
        start_index: Option<usize>,
    ) -> Result<(), CoreError> {
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        self.bump_queue_generation();
        for o in self.each_output(node_id).await? { o.set_queue(&file_paths).await?; }
        let mut s = self.state.write().await;
        let node = s.node_mut(node_id).ok_or(CoreError::NodeNotFound)?;
        node.queue = tracks;
        node.current_index = start_index;
        node.position_secs = 0.0;
        node.status = if start_index.is_some() {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Stopped
        };
        drop(s);
        if start_index.is_some() {
            for o in self.each_output(node_id).await? { o.play().await?; }
        }
        self.broadcast().await;
        Ok(())
    }

    fn build_queue_file_paths(node: &Node) -> Vec<String> {
        let start = node.current_index.unwrap_or(0);
        let tail = if node.current_index.is_some() {
            node.queue.iter().skip(start + 1).map(|t| t.file_path.clone()).collect::<Vec<_>>()
        } else {
            node.queue.iter().skip(1).map(|t| t.file_path.clone()).collect::<Vec<_>>()
        };
        let head = node.queue.get(start).map(|t| vec![t.file_path.clone()]).unwrap_or_default();
        let mut paths = head;
        paths.extend(tail);
        paths
    }

    async fn broadcast(&self) {
        let snapshot = self.state.read().await.clone();
        for b in &self.broadcasters {
            b.on_state_changed(&snapshot).await;
        }
    }
}
