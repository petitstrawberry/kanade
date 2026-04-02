use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("output error: {0}")]
    Output(String),

    #[error("zone not found")]
    ZoneNotFound,

    #[error("queue is empty")]
    QueueEmpty,

    #[error("queue index out of bounds")]
    QueueIndexOutOfBounds,

    #[error("track not found: {0}")]
    TrackNotFound(String),

    #[error("invalid volume: value must be 0–100")]
    InvalidVolume,

    #[error("internal error: {0}")]
    Internal(String),
}
