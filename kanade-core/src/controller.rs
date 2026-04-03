use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::instrument;

use crate::{
    error::CoreError,
    model::{Node, PlaybackStatus, RepeatMode, Track},
    ports::{AudioOutput, EventBroadcaster, StatePersister},
    state::PlaybackState,
};

#[derive(Debug, Clone, Default)]
struct NodeTransportState {
    projection_generation: u64,
    projection_start_index: Option<usize>,
    loaded_len: usize,
}

pub struct Core {
    state: Arc<RwLock<PlaybackState>>,
    outputs: Arc<RwLock<HashMap<String, Arc<dyn AudioOutput>>>>,
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
            state: Arc::new(RwLock::new(PlaybackState { nodes: Vec::new() })),
            outputs: Arc::new(RwLock::new(outputs.into_iter().collect())),
            broadcasters,
            persisters: Vec::new(),
            transport_state: Arc::new(RwLock::new(HashMap::new())),
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

    pub fn add_persister(&mut self, p: Arc<dyn StatePersister>) {
        self.persisters.push(p);
    }

    pub async fn restore_state(&self, state: PlaybackState) {
        *self.state.write().await = state;
    }

    /// Apply an external state update (e.g. from a remote output node) to the
    /// named node, then broadcast the change to all event listeners.
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
            transport
                .get(node_id)
                .and_then(|projection| {
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
        if let Some(node) = s.node_mut(node_id) {
            node.status = status;
            node.position_secs = position_secs;
            node.volume = volume;
            if let Some(next_index) = projected_current_index {
                if next_index < node.queue.len() {
                    node.current_index = Some(next_index);
                }
            }
        }
        drop(s);
        self.broadcast().await;
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

    async fn extend_projection_loaded_len(
        &self,
        node_id: &str,
        projection_start_hint: Option<usize>,
        added_len: usize,
    ) {
        if added_len == 0 {
            return;
        }
        let mut transport = self.transport_state.write().await;
        let entry = transport.entry(node_id.to_string()).or_default();
        if entry.projection_start_index.is_none() {
            entry.projection_start_index = projection_start_hint;
        }
        entry.loaded_len = entry.loaded_len.saturating_add(added_len);
    }

    async fn decrement_projection_loaded_len(&self, node_id: &str, removed_len: usize) {
        if removed_len == 0 {
            return;
        }
        let mut transport = self.transport_state.write().await;
        if let Some(entry) = transport.get_mut(node_id) {
            entry.loaded_len = entry.loaded_len.saturating_sub(removed_len);
            if entry.loaded_len == 0 {
                entry.projection_start_index = None;
            }
        }
    }

    pub async fn add_node(&self, node: Node) {
        self.state.write().await.nodes.push(node);
        self.broadcast().await;
    }

    pub async fn remove_node(&self, node_id: &str) {
        self.state
            .write()
            .await
            .nodes
            .retain(|n| n.id != node_id);
        self.transport_state.write().await.remove(node_id);
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
        let projection_start = node.current_index;
        let loaded_len = queue.len();
        drop(s);
        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, loaded_len)
            .await;
        for o in self.each_output(node_id).await? {
            o.set_queue(&queue, projection_generation).await?;
        }
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
        let projection_start = node.current_index;
        let loaded_len = queue.len();
        drop(s);
        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, loaded_len)
            .await;
        for o in self.each_output(node_id).await? {
            o.set_queue(&queue, projection_generation).await?;
        }
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
        let projection_start_hint = node.current_index;
        node.queue.push(track);
        drop(s);
        self.extend_projection_loaded_len(node_id, projection_start_hint, file_paths.len())
            .await;
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
        let projection_start_hint = node.current_index;
        node.queue.extend(tracks);
        drop(s);
        self.extend_projection_loaded_len(node_id, projection_start_hint, file_paths.len())
            .await;
        self.broadcast().await;
        Ok(())
    }

    pub async fn clear_node_queue(&self, node_id: &str) -> Result<(), CoreError> {
        let projection_generation = self.rebuild_projection_state(node_id, None, 0).await;
        for o in self.each_output(node_id).await? {
            o.set_queue(&[], projection_generation).await?;
        }
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
        self.decrement_projection_loaded_len(node_id, 1).await;
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
        let projection_start = node.current_index;
        let loaded_len = queue.len();
        drop(s);
        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, loaded_len)
            .await;
        for o in self.each_output(node_id).await? {
            o.set_queue(&queue, projection_generation).await?;
        }
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
        let start = start_index.unwrap_or(0);
        let head = tracks.get(start).map(|t| vec![t.file_path.clone()]).unwrap_or_default();
        let tail = tracks.iter().skip(start + 1).map(|t| t.file_path.clone()).collect::<Vec<_>>();
        let rotated: Vec<String> = {
            let mut p = head;
            p.extend(tail);
            p
        };
        let projection_start = if rotated.is_empty() { None } else { start_index };
        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, rotated.len())
            .await;
        for o in self.each_output(node_id).await? {
            o.set_queue(&rotated, projection_generation).await?;
        }
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

    pub async fn restore_node_output_state(&self, node_id: &str) -> Result<(), CoreError> {
        let (queue, projection_start, loaded_len, volume) = {
            let s = self.state.read().await;
            let node = s.node(node_id).ok_or(CoreError::NodeNotFound)?;
            (
                Self::build_queue_file_paths(node),
                node.current_index,
                node.queue.len(),
                node.volume,
            )
        };

        let projection_generation = self
            .rebuild_projection_state(node_id, projection_start, loaded_len)
            .await;

        for o in self.each_output(node_id).await? {
            o.set_volume(volume).await?;
            o.set_queue(&queue, projection_generation).await?;
        }

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
            vec![("default".to_string(), output.clone() as Arc<dyn AudioOutput>)],
            vec![],
        );
        core.add_node(Node {
            id: "default".to_string(),
            name: "node".to_string(),
            output_ids: vec!["default".to_string()],
            queue: vec![],
            current_index: None,
            status: PlaybackStatus::Stopped,
            position_secs: 0.0,
            volume: 50,
            shuffle: false,
            repeat: RepeatMode::Off,
        })
        .await;
        (core, output)
    }

    #[tokio::test]
    async fn sync_node_state_maps_projection_song_to_global_index() {
        let (core, output) = setup_core_with_node().await;
        core.set_node_queue(
            "default",
            vec![sample_track("a"), sample_track("b"), sample_track("c"), sample_track("d")],
            Some(1),
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.sync_node_state("default", PlaybackStatus::Playing, 12.0, 70, Some(0), generation)
            .await;

        let node = core.get_node("default").await.unwrap();
        assert_eq!(node.current_index, Some(1));
        assert_eq!(node.position_secs, 12.0);
        assert_eq!(node.volume, 70);
    }

    #[tokio::test]
    async fn sync_node_state_ignores_stale_projection_generation() {
        let (core, output) = setup_core_with_node().await;
        core.set_node_queue(
            "default",
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

        let node = core.get_node("default").await.unwrap();
        assert_eq!(node.current_index, Some(1));
        assert_eq!(node.position_secs, 5.0);
        assert_eq!(node.volume, 60);
    }

    #[tokio::test]
    async fn add_to_queue_keeps_projection_base_and_allows_auto_advance() {
        let (core, output) = setup_core_with_node().await;
        core.set_node_queue(
            "default",
            vec![sample_track("a"), sample_track("b"), sample_track("c"), sample_track("d")],
            None,
        )
        .await
        .unwrap();
        core.play_node_index(
            "default",
            1,
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.add_to_node_queue("default", sample_track("e"))
            .await
            .unwrap();

        core.sync_node_state("default", PlaybackStatus::Playing, 0.0, 50, Some(2), generation)
            .await;

        let node = core.get_node("default").await.unwrap();
        assert_eq!(node.current_index, Some(3));
    }

    #[tokio::test]
    async fn sync_node_state_ignores_out_of_bounds_mpd_song_index() {
        let (core, output) = setup_core_with_node().await;
        core.set_node_queue(
            "default",
            vec![sample_track("a"), sample_track("b"), sample_track("c")],
            Some(1),
        )
        .await
        .unwrap();

        let generation = output.last_projection_generation();

        core.sync_node_state("default", PlaybackStatus::Playing, 8.0, 40, Some(5), generation)
            .await;

        let node = core.get_node("default").await.unwrap();
        assert_eq!(node.current_index, Some(1));
        assert_eq!(node.position_secs, 8.0);
    }
}
