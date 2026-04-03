//! kanade-adapter-node-server — Server-side adapter for remote output nodes.
//!
//! This crate implements the server side of the kanade output-node protocol.
//!
//! [`NodeServer`] listens for WebSocket connections from [`kanade-node`]
//! processes. For each connection it:
//!
//! 1. Completes the kanade protocol handshake.
//! 2. Creates a [`RemoteNodeOutput`] backed by a channel.
//! 3. Registers the output and a corresponding [`Zone`] with the [`Core`].
//! 4. Forwards [`NodeCommand`] messages from the Core to the node.
//! 5. Applies incoming [`NodeStateUpdate`] messages to the Core's state.
//! 6. Cleans up when the node disconnects.

pub mod output;
pub mod server;

pub use output::RemoteNodeOutput;
pub use server::NodeServer;
