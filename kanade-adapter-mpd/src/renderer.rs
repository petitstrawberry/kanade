use async_trait::async_trait;
use tracing::instrument;

use kanade_core::{error::CoreError, ports::AudioRenderer};

use crate::client::MpdClient;

/// [`AudioRenderer`] implementation that controls a local MPD daemon.
///
/// All operations translate directly to the corresponding MPD protocol
/// commands, which are sent over TCP.
pub struct MpdRenderer {
    client: MpdClient,
}

impl MpdRenderer {
    /// Create a new renderer targeting `host:port`.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { client: MpdClient::new(host, port) }
    }
}

#[async_trait]
impl AudioRenderer for MpdRenderer {
    #[instrument(skip(self))]
    async fn execute_play(&self) -> Result<(), CoreError> {
        self.client.send("play\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_pause(&self) -> Result<(), CoreError> {
        self.client.send("pause 1\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_stop(&self) -> Result<(), CoreError> {
        self.client.send("stop\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_next(&self) -> Result<(), CoreError> {
        self.client.send("next\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_previous(&self) -> Result<(), CoreError> {
        self.client.send("previous\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_seek(&self, position_secs: f64) -> Result<(), CoreError> {
        self.client
            .send(&format!("seekcur {position_secs:.3}\n"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn execute_set_volume(&self, volume: u8) -> Result<(), CoreError> {
        self.client
            .send(&format!("setvol {volume}\n"))
            .await?;
        Ok(())
    }

    /// Replace the MPD queue with the given list of file paths.
    ///
    /// Uses a `command_list` to atomically clear and re-populate the queue.
    #[instrument(skip(self, file_paths))]
    async fn execute_set_queue(&self, file_paths: &[String]) -> Result<(), CoreError> {
        let mut cmd = String::from("command_list_begin\nclear\n");
        for path in file_paths {
            // MPD `add` expects paths relative to its music directory.
            // We pass them as-is; operators are responsible for configuring
            // MPD's music_directory to match.
            cmd.push_str(&format!("add {path}\n"));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }
}
