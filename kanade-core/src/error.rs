use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("renderer error: {0}")]
    Renderer(String),

    #[error("queue is empty")]
    QueueEmpty,

    #[error("track not found: {0}")]
    TrackNotFound(String),

    #[error("invalid volume: value must be 0–100")]
    InvalidVolume,

    #[error("internal error: {0}")]
    Internal(String),
}
