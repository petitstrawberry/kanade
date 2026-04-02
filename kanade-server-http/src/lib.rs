//! kanade-server-http — HTTP media file server for Kanade.
//!
//! Serves audio files from the library over HTTP with Range request support.
//! Used by audio backends (e.g. MPD) to stream tracks via URL.

mod media_server;

pub use media_server::MediaServer;
