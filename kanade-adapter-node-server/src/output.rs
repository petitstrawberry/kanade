use async_trait::async_trait;
use kanade_core::{error::CoreError, ports::AudioOutput};
use kanade_node_protocol::NodeCommand;
use tokio::sync::mpsc;

/// [`AudioOutput`] implementation that forwards every call to a connected
/// output node over the kanade protocol.
///
/// Each instance holds the sending half of an `mpsc` channel.  The receiving
/// half is owned by the connection task running inside [`super::NodeServer`],
/// which serialises the commands and writes them to the WebSocket.
pub struct RemoteNodeOutput {
    tx: mpsc::Sender<NodeCommand>,
}

impl RemoteNodeOutput {
    pub fn new(tx: mpsc::Sender<NodeCommand>) -> Self {
        Self { tx }
    }

    async fn send(&self, cmd: NodeCommand) -> Result<(), CoreError> {
        self.tx
            .send(cmd)
            .await
            .map_err(|e| CoreError::Output(e.to_string()))
    }
}

#[async_trait]
impl AudioOutput for RemoteNodeOutput {
    async fn play(&self) -> Result<(), CoreError> {
        self.send(NodeCommand::Play).await
    }

    async fn pause(&self) -> Result<(), CoreError> {
        self.send(NodeCommand::Pause).await
    }

    async fn stop(&self) -> Result<(), CoreError> {
        self.send(NodeCommand::Stop).await
    }

    async fn seek(&self, position_secs: f64) -> Result<(), CoreError> {
        self.send(NodeCommand::Seek { position_secs }).await
    }

    async fn set_volume(&self, volume: u8) -> Result<(), CoreError> {
        self.send(NodeCommand::SetVolume { volume }).await
    }

    async fn set_queue(&self, file_paths: &[String]) -> Result<(), CoreError> {
        self.send(NodeCommand::SetQueue {
            file_paths: file_paths.to_vec(),
        })
        .await
    }

    async fn add(&self, file_paths: &[String]) -> Result<(), CoreError> {
        self.send(NodeCommand::Add {
            file_paths: file_paths.to_vec(),
        })
        .await
    }

    async fn remove(&self, index: usize) -> Result<(), CoreError> {
        self.send(NodeCommand::Remove { index }).await
    }

    async fn move_track(&self, from: usize, to: usize) -> Result<(), CoreError> {
        self.send(NodeCommand::MoveTrack { from, to }).await
    }
}
