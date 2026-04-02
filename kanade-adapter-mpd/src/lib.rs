//! kanade-adapter-mpd — Output adapter that drives a local MPD daemon.
//!
//! Implements [`AudioRenderer`] by sending MPD protocol commands over TCP.
//! The Core never knows it is talking to MPD; it only calls the trait methods.

pub mod client;
pub mod renderer;

pub use renderer::MpdRenderer;
