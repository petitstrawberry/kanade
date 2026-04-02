//! kanade-adapter-ws — WebSocket input/broadcast adapter.
//!
//! Accepts JSON commands from native UIs over WebSocket and pushes the
//! updated [`PlaybackState`] as JSON to all connected clients after every
//! state change.
//!
//! Architecture role:
//! - **Input adapter**: parses inbound JSON → calls `CoreController` methods.
//! - **Broadcaster**: implements [`EventBroadcaster`] to push state JSON.

pub mod broadcaster;
pub mod command;
pub mod server;

pub use broadcaster::WsBroadcaster;
pub use server::WsServer;
