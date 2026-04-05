//! kanade-core — The brain of Kanade.
//!
//! This crate owns:
//! - All shared data structures (PlaybackState, Node, Track, …)
//! - The port traits that adapters must implement (AudioOutput, EventBroadcaster)
//! - The Core controller that drives node-scoped playback logic

pub mod controller;
pub mod error;
pub mod model;
pub mod plugin;
pub mod ports;
pub mod state;

pub use controller::Core;
pub use error::CoreError;
pub use model::{Album, Artist, Node, PlaybackStatus, RepeatMode, Track};
pub use state::PlaybackState;
