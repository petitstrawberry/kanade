use std::{collections::HashMap, time::Duration};

use kanade_core::state::PlaybackState;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::RwLock,
    time::sleep,
};
use tracing::{debug, info, warn};

use crate::client::MpdClient;

pub struct MpdStateSync {
    state: std::sync::Arc<RwLock<PlaybackState>>,
    host: String,
    port: u16,
}

impl MpdStateSync {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        _client: MpdClient,
        state: std::sync::Arc<RwLock<PlaybackState>>,
    ) -> Self {
        Self {
            state,
            host: host.into(),
            port,
        }
    }

    pub async fn run(&self) {
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

    async fn sync_loop(&self) -> Result<(), String> {
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

        self.poll_status(&mut reader, &mut writer).await?;

        loop {
            writer
                .write_all(b"idle player mixer\n")
                .await
                .map_err(|e| format!("idle write: {e}"))?;

            let mut changed = false;
            loop {
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("idle read: {e}"))?;
                let trimmed = line.trim_end();
                if trimmed == "OK" {
                    break;
                }
                if trimmed.starts_with("ACK") {
                    return Err(format!("MPD error: {trimmed}"));
                }
                if trimmed.starts_with("changed: ") {
                    changed = true;
                }
            }

            if changed {
                self.poll_status(&mut reader, &mut writer).await?;
            }
        }
    }

    async fn poll_status(
        &self,
        reader: &mut BufReader<OwnedReadHalf>,
        writer: &mut OwnedWriteHalf,
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

        let mut state = self.state.write().await;
        if let Some(zone) = state.zones.get_mut(0) {
            zone.position_secs = elapsed;
        }

        debug!("MPD sync: elapsed={elapsed:.1}");
        Ok(())
    }
}
