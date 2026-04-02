use async_trait::async_trait;
use tracing::instrument;

use kanade_core::{error::CoreError, ports::AudioOutput};

use crate::client::MpdClient;

/// [`AudioOutput`] implementation that controls a local MPD daemon.
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
impl AudioOutput for MpdRenderer {
    #[instrument(skip(self))]
    async fn play(&self) -> Result<(), CoreError> {
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
    async fn set_queue(&self, file_paths: &[String]) -> Result<(), CoreError> {
        let mut cmd = String::from("command_list_begin\nclear\n");
        for path in file_paths {
            cmd.push_str(&format!("add {path}\n"));
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
            cmd.push_str(&format!("add {path}\n"));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }
}
