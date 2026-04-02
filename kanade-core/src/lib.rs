//! kanade-core — The brain of Kanade.
//!
//! This crate owns:
//! - All shared data structures (PlaybackState, Track, …)
//! - The port traits that adapters must implement
//! - The CoreController that drives playback logic

pub mod controller;
pub mod error;
pub mod model;
pub mod ports;
pub mod state;

pub use controller::CoreController;
pub use error::CoreError;
pub use model::{Album, Artist, Track};
pub use state::{PlaybackState, PlaybackStatus, QueueEntry};
