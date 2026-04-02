//! kanade-core — The brain of Kanade.
//!
//! This crate owns:
//! - All shared data structures (PlaybackState, Zone, Track, …)
//! - The port traits that adapters must implement (AudioOutput, EventBroadcaster)
//! - The Core controller that drives zone-scoped playback logic

pub mod controller;
pub mod error;
pub mod model;
pub mod ports;
pub mod state;

pub use controller::Core;
pub use error::CoreError;
pub use model::{Album, Artist, PlaybackStatus, RepeatMode, Track, Zone};
pub use state::PlaybackState;
