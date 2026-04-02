//! kanade-adapter-mpd — Output adapter that drives a local MPD daemon.
//!
//! Implements [`AudioOutput`] by sending MPD protocol commands over TCP.
//! The Core never knows it is talking to MPD; it only calls the trait methods.
//!
//! Also provides [`MpdStateSync`] which runs an idle loop against MPD to
//! keep the shared [`PlaybackState`] in sync with MPD's actual state.

pub mod client;
pub mod renderer;
pub mod state_sync;

pub use client::MpdClient;
pub use renderer::MpdRenderer;
pub use state_sync::MpdStateSync;
