use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::instrument;

use crate::{
    error::CoreError,
    model::{PlaybackStatus, RepeatMode, Track, Zone},
    ports::{AudioOutput, EventBroadcaster},
    state::PlaybackState,
};

pub struct Core {
    state: Arc<RwLock<PlaybackState>>,
    outputs: HashMap<String, Arc<dyn AudioOutput>>,
    broadcasters: Vec<Arc<dyn EventBroadcaster>>,
}

impl Core {
    pub fn new(
        outputs: Vec<(String, Arc<dyn AudioOutput>)>,
        broadcasters: Vec<Arc<dyn EventBroadcaster>>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(PlaybackState { zones: Vec::new() })),
            outputs: outputs.into_iter().collect(),
            broadcasters,
        }
    }

    pub fn add_output(&mut self, id: String, output: Arc<dyn AudioOutput>) {
        self.outputs.insert(id, output);
    }

    pub fn add_broadcaster(&mut self, b: Arc<dyn EventBroadcaster>) {
        self.broadcasters.push(b);
    }

    pub fn state_handle(&self) -> Arc<RwLock<PlaybackState>> {
        Arc::clone(&self.state)
    }

    pub async fn add_zone(&self, zone: Zone) {
        self.state.write().await.zones.push(zone);
    }

    pub async fn get_zone(&self, id: &str) -> Option<Zone> {
        self.state.read().await.zone(id).cloned()
    }

    async fn each_output(&self, zone_id: &str) -> Result<Vec<Arc<dyn AudioOutput>>, CoreError> {
        let s = self.state.read().await;
        let zone = s.zone(zone_id).ok_or(CoreError::ZoneNotFound)?;
        let ids = zone.output_ids.clone();
        drop(s);
        let mut outs = Vec::new();
        for id in &ids {
            if let Some(o) = self.outputs.get(id) {
                outs.push(Arc::clone(o));
            }
        }
        Ok(outs)
    }

    #[instrument(skip(self))]
    pub async fn play_zone(&self, zone_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(zone_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            if !zone.queue.is_empty() && zone.current_index.is_none() {
                zone.current_index = Some(0);
            }
            if zone.current_index.is_some() {
                zone.status = PlaybackStatus::Playing;
            }
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn pause_zone(&self, zone_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(zone_id).await? { o.pause().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.status = PlaybackStatus::Paused;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn stop_zone(&self, zone_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(zone_id).await? { o.stop().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.status = PlaybackStatus::Stopped;
            zone.position_secs = 0.0;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn next_zone(&self, zone_id: &str) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        if zone.queue.is_empty() {
            return Err(CoreError::QueueEmpty);
        }
        let next = match zone.current_index {
            Some(i) => match zone.repeat {
                RepeatMode::Off => {
                    if i + 1 < zone.queue.len() { i + 1 } else { return Err(CoreError::QueueEmpty) }
                }
                RepeatMode::One => i,
                RepeatMode::All => (i + 1) % zone.queue.len(),
            },
            None => 0,
        };
        zone.current_index = Some(next);
        zone.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(zone);
        drop(s);
        for o in self.each_output(zone_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(zone_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn previous_zone(&self, zone_id: &str) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        if zone.queue.is_empty() {
            return Err(CoreError::QueueEmpty);
        }
        let prev = match zone.current_index {
            Some(0) | None => match zone.repeat {
                RepeatMode::Off => return Err(CoreError::QueueEmpty),
                RepeatMode::One => 0,
                RepeatMode::All => zone.queue.len() - 1,
            },
            Some(i) => i - 1,
        };
        zone.current_index = Some(prev);
        zone.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(zone);
        drop(s);
        for o in self.each_output(zone_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(zone_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn seek_zone(&self, zone_id: &str, position_secs: f64) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        zone.position_secs = position_secs;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_zone_volume(&self, zone_id: &str, volume: u8) -> Result<(), CoreError> {
        if volume > 100 {
            return Err(CoreError::InvalidVolume);
        }
        for o in self.each_output(zone_id).await? { o.set_volume(volume).await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.volume = volume;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_zone_shuffle(&self, zone_id: &str, shuffle: bool) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.shuffle = shuffle;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_zone_repeat(&self, zone_id: &str, repeat: RepeatMode) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.repeat = repeat;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_to_zone_queue(&self, zone_id: &str, track: Track) -> Result<(), CoreError> {
        let file_paths = vec![track.file_path.clone()];
        for o in self.each_output(zone_id).await? { o.add(&file_paths).await?; }
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        zone.queue.push(track);
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn add_tracks_to_zone_queue(&self, zone_id: &str, tracks: Vec<Track>) -> Result<(), CoreError> {
        if tracks.is_empty() {
            return Ok(());
        }
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        for o in self.each_output(zone_id).await? { o.add(&file_paths).await?; }
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        zone.queue.extend(tracks);
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn clear_zone_queue(&self, zone_id: &str) -> Result<(), CoreError> {
        for o in self.each_output(zone_id).await? { o.set_queue(&[]).await?; }
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        zone.queue.clear();
        zone.current_index = None;
        zone.position_secs = 0.0;
        zone.status = PlaybackStatus::Stopped;
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn remove_from_zone_queue(&self, zone_id: &str, index: usize) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        if index >= zone.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        zone.queue.remove(index);
        match zone.current_index {
            Some(ci) if ci == index && zone.queue.is_empty() => {
                zone.current_index = None;
                zone.status = PlaybackStatus::Stopped;
            }
            Some(ci) if ci == index => {
                zone.current_index = Some(ci.min(zone.queue.len() - 1));
            }
            Some(ci) if ci > index => {
                zone.current_index = Some(ci - 1);
            }
            _ => {}
        }
        let queue = Self::build_queue_file_paths(zone);
        drop(s);
        for o in self.each_output(zone_id).await? { o.set_queue(&queue).await?; }
        self.broadcast().await;
        Ok(())
    }

    pub async fn move_in_zone_queue(
        &self,
        zone_id: &str,
        from: usize,
        to: usize,
    ) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        if from >= zone.queue.len() || to >= zone.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        let track = zone.queue.remove(from);
        zone.queue.insert(to, track);
        match zone.current_index {
            Some(ci) if ci == from => {
                zone.current_index = Some(to);
            }
            Some(ci) if from < ci && ci <= to => {
                zone.current_index = Some(ci - 1);
            }
            Some(ci) if to <= ci && ci < from => {
                zone.current_index = Some(ci + 1);
            }
            _ => {}
        }
        let queue = Self::build_queue_file_paths(zone);
        drop(s);
        for o in self.each_output(zone_id).await? { o.set_queue(&queue).await?; }
        self.broadcast().await;
        Ok(())
    }

    pub async fn play_zone_index(&self, zone_id: &str, index: usize) -> Result<(), CoreError> {
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        if index >= zone.queue.len() {
            return Err(CoreError::QueueIndexOutOfBounds);
        }
        zone.current_index = Some(index);
        zone.position_secs = 0.0;
        let queue = Self::build_queue_file_paths(zone);
        drop(s);
        for o in self.each_output(zone_id).await? { o.set_queue(&queue).await?; }
        for o in self.each_output(zone_id).await? { o.play().await?; }
        let mut s = self.state.write().await;
        if let Some(zone) = s.zone_mut(zone_id) {
            zone.status = PlaybackStatus::Playing;
        }
        drop(s);
        self.broadcast().await;
        Ok(())
    }

    pub async fn set_zone_queue(
        &self,
        zone_id: &str,
        tracks: Vec<Track>,
        start_index: Option<usize>,
    ) -> Result<(), CoreError> {
        let file_paths: Vec<String> = tracks.iter().map(|t| t.file_path.clone()).collect();
        for o in self.each_output(zone_id).await? { o.set_queue(&file_paths).await?; }
        let mut s = self.state.write().await;
        let zone = s.zone_mut(zone_id).ok_or(CoreError::ZoneNotFound)?;
        zone.queue = tracks;
        zone.current_index = start_index;
        zone.position_secs = 0.0;
        zone.status = if start_index.is_some() {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Stopped
        };
        drop(s);
        if start_index.is_some() {
            for o in self.each_output(zone_id).await? { o.play().await?; }
        }
        self.broadcast().await;
        Ok(())
    }

    fn build_queue_file_paths(zone: &Zone) -> Vec<String> {
        let start = zone.current_index.unwrap_or(0);
        let tail = if zone.current_index.is_some() {
            zone.queue.iter().skip(start + 1).map(|t| t.file_path.clone()).collect::<Vec<_>>()
        } else {
            zone.queue.iter().skip(1).map(|t| t.file_path.clone()).collect::<Vec<_>>()
        };
        let head = zone.queue.get(start).map(|t| vec![t.file_path.clone()]).unwrap_or_default();
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
