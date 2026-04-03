use async_trait::async_trait;
use crate::error::CoreError;
use crate::state::PlaybackState;

#[async_trait]
pub trait AudioOutput: Send + Sync {
    async fn play(&self) -> Result<(), CoreError>;
    async fn pause(&self) -> Result<(), CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn seek(&self, position_secs: f64) -> Result<(), CoreError>;
    async fn set_volume(&self, volume: u8) -> Result<(), CoreError>;
    async fn set_queue(
        &self,
        file_paths: &[String],
        projection_generation: u64,
    ) -> Result<(), CoreError>;
    async fn add(&self, file_paths: &[String]) -> Result<(), CoreError>;
    async fn remove(&self, index: usize) -> Result<(), CoreError>;
    async fn move_track(&self, from: usize, to: usize) -> Result<(), CoreError>;
}

#[async_trait]
pub trait EventBroadcaster: Send + Sync {
    async fn on_state_changed(&self, state: &crate::state::PlaybackState);
}

#[async_trait]
pub trait StatePersister: Send + Sync {
    async fn persist(&self, state: &PlaybackState);
}
