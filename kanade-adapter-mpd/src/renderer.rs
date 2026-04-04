use async_trait::async_trait;
use tracing::{info, instrument};

use kanade_core::{error::CoreError, ports::AudioOutput};
use sha2::{Digest, Sha256};

use crate::client::MpdClient;

/// [`AudioOutput`] implementation that controls a local MPD daemon.
///
/// All operations translate directly to the corresponding MPD protocol
/// commands, which are sent over TCP.
pub struct MpdRenderer {
    client: MpdClient,
    media_public_base_url: String,
}

impl MpdRenderer {
    pub fn new(host: impl Into<String>, port: u16, media_public_base_url: impl Into<String>) -> Self {
        Self {
            client: MpdClient::new(host, port),
            media_public_base_url: media_public_base_url.into().trim_end_matches('/').to_string(),
        }
    }

    fn media_uri(&self, value: &str) -> String {
        if value.starts_with("http://") || value.starts_with("https://") {
            return value.to_string();
        }

        let mut hasher = Sha256::new();
        hasher.update(value.as_bytes());
        let track_id = hex::encode(hasher.finalize());
        format!("{}/media/tracks/{}", self.media_public_base_url, track_id)
    }

    fn quote_mpd_arg(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

#[async_trait]
impl AudioOutput for MpdRenderer {
    #[instrument(skip(self))]
    async fn play(&self) -> Result<(), CoreError> {
        info!("mpd-renderer: play");
        self.client.send("play\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn pause(&self) -> Result<(), CoreError> {
        self.client.send("pause 1\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn stop(&self) -> Result<(), CoreError> {
        info!("mpd-renderer: stop");
        self.client.send("stop\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn seek(&self, position_secs: f64) -> Result<(), CoreError> {
        self.client
            .send(&format!("seekcur {position_secs:.3}\n"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn set_volume(&self, volume: u8) -> Result<(), CoreError> {
        self.client
            .send(&format!("setvol {volume}\n"))
            .await?;
        Ok(())
    }

    /// Replace the MPD queue with the given list of file paths.
    ///
    /// Uses a `command_list` to atomically clear and re-populate the queue.
    #[instrument(skip(self, file_paths))]
    async fn set_queue(
        &self,
        file_paths: &[String],
        _projection_generation: u64,
    ) -> Result<(), CoreError> {
        info!(queue_len = file_paths.len(), "mpd-renderer: set_queue");
        let mut cmd = String::from("command_list_begin\nclear\n");
        for path in file_paths {
            let uri = self.media_uri(path);
            cmd.push_str(&format!("add {}\n", Self::quote_mpd_arg(&uri)));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }

    /// Append file paths to the MPD queue.
    #[instrument(skip(self, file_paths))]
    async fn add(&self, file_paths: &[String]) -> Result<(), CoreError> {
        let mut cmd = String::from("command_list_begin\n");
        for path in file_paths {
            let uri = self.media_uri(path);
            cmd.push_str(&format!("add {}\n", Self::quote_mpd_arg(&uri)));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }

    /// Remove the track at the given position from the MPD queue.
    #[instrument(skip(self))]
    async fn remove(&self, index: usize) -> Result<(), CoreError> {
        self.client.send(&format!("delete {index}\n")).await?;
        Ok(())
    }

    /// Move the track at `from` position to `to` position in the MPD queue.
    #[instrument(skip(self))]
    async fn move_track(&self, from: usize, to: usize) -> Result<(), CoreError> {
        self.client.send(&format!("move {from} {to}\n")).await?;
        Ok(())
    }
}

impl MpdRenderer {
    #[instrument(skip(self))]
    pub async fn clear(&self) -> Result<(), CoreError> {
        self.client.send("stop\nclear\n").await?;
        Ok(())
    }
}
