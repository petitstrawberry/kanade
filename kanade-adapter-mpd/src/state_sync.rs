use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    time::Duration,
};

use kanade_core::{model::PlaybackStatus, ports::EventBroadcaster, state::PlaybackState};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::RwLock,
    time::sleep,
};
use tracing::{debug, info, warn};

use crate::client::MpdClient;

pub struct MpdStateSync {
    state: std::sync::Arc<RwLock<PlaybackState>>,
    broadcasters: Vec<std::sync::Arc<dyn EventBroadcaster>>,
    host: String,
    port: u16,
    projection_generation: Arc<AtomicU64>,
}

impl MpdStateSync {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        _client: MpdClient,
        state: std::sync::Arc<RwLock<PlaybackState>>,
        broadcasters: Vec<std::sync::Arc<dyn EventBroadcaster>>,
        projection_generation: Arc<AtomicU64>,
    ) -> Self {
        Self {
            state,
            broadcasters,
            host: host.into(),
            port,
            projection_generation,
        }
    }

    pub async fn run(&mut self) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            match self.sync_loop().await {
                Ok(()) => break,
                Err(e) => {
                    warn!("MPD sync error: {e}, reconnecting in {backoff:?}");
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }

    async fn sync_loop(&mut self) -> Result<(), String> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(&addr))
            .await
            .map_err(|e| format!("connect timeout: {e}"))?
            .map_err(|e| format!("connect error: {e}"))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let mut banner = String::new();
        reader
            .read_line(&mut banner)
            .await
            .map_err(|e| format!("banner read: {e}"))?;
        info!("MPD sync connected: {banner:?}");

        loop {
            self.poll_status(&mut reader, &mut writer).await?;
            sleep(Duration::from_millis(500)).await;
        }
    }

    async fn poll_status(
        &mut self,
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
    ) -> Result<(), String> {
        writer
            .write_all(b"status\n")
            .await
            .map_err(|e| format!("status write: {e}"))?;

        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("status read: {e}"))?;
            let trimmed = line.trim_end().to_string();
            if trimmed == "OK" {
                break;
            }
            lines.push(trimmed);
        }

        let mut map = HashMap::new();
        for line in &lines {
            if let Some((k, v)) = line.split_once(':') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }

        let elapsed = map
            .get("elapsed")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let song = map
            .get("song")
            .and_then(|s| s.parse::<usize>().ok());
        let volume = map
            .get("volume")
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(0);
        let playback_status = match map.get("state").map(String::as_str) {
            Some("play") => PlaybackStatus::Playing,
            Some("pause") => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        };
        let projection_generation = self.projection_generation.load(Ordering::Relaxed);

        let mut state = self.state.write().await;
        if let Some(node) = state.nodes.get_mut(0) {
            node.position_secs = elapsed;
            node.status = playback_status;
            node.volume = volume;
        }
        state.current_index = song;
        let snapshot = state.clone();
        drop(state);

        for broadcaster in &self.broadcasters {
            broadcaster.on_state_changed(&snapshot).await;
        }

        debug!(
            "MPD sync: elapsed={elapsed:.1}, mpd_song={song:?}, projection_generation={projection_generation}"
        );
        Ok(())
    }
}
