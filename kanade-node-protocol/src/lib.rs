//! kanade-node-protocol — Shared message types for the kanade output-node protocol.
//!
//! This crate defines the messages exchanged between the Kanade server and
//! remote output nodes over a WebSocket connection (the *kanade protocol*).
//!
//! # Handshake
//!
//! 1. The output node connects to the server's node endpoint.
//! 2. The node sends a [`NodeRegistration`] message with a human-readable name.
//! 3. The server assigns a UUID as the node (= zone) identifier and replies
//!    with a [`NodeRegistrationAck`] that contains both the assigned `node_id`
//!    and the media base URL the node must use when constructing track URIs.
//!
//! # Ongoing communication
//!
//! * **Server → Node**: [`NodeCommand`] messages drive playback.
//! * **Node → Server**: [`NodeStateUpdate`] messages report current state.

use kanade_core::model::PlaybackStatus;
use serde::{Deserialize, Serialize};

/// Sent by the output node immediately after the WebSocket connection is
/// established.  The node only supplies a human-readable `name`; the server
/// assigns the node's identifier automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistration {
    pub name: String,
}

/// Sent by the server in response to a successful [`NodeRegistration`].
///
/// `node_id` is the UUID the server assigned to this node (= zone ID).
/// `media_base_url` is the HTTP base URL of the server's media endpoint
/// (e.g. `http://192.168.1.10:8081`). The node must use this URL when
/// constructing track URIs for its local audio backend (e.g. MPD).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistrationAck {
    pub node_id: String,
    pub media_base_url: String,
}

/// Commands sent from the Kanade server to an output node.
///
/// These mirror the [`kanade_core::ports::AudioOutput`] trait methods so that
/// every trait call on the server side is forwarded to the remote node over
/// the WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeCommand {
    Play,
    Pause,
    Stop,
    Seek { position_secs: f64 },
    SetVolume { volume: u8 },
    /// Replace the node's entire playback queue with the given file paths.
    SetQueue { file_paths: Vec<String> },
    /// Append tracks to the node's playback queue.
    Add { file_paths: Vec<String> },
    /// Remove the track at the given queue position.
    Remove { index: usize },
    /// Move a track within the queue.
    MoveTrack { from: usize, to: usize },
}

/// Periodic state update sent from the output node to the server so that the
/// server's [`kanade_core::state::PlaybackState`] stays in sync with what the
/// node's audio backend (e.g. MPD) is actually doing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStateUpdate {
    pub status: PlaybackStatus,
    pub position_secs: f64,
    pub volume: u8,
    pub current_index: Option<usize>,
}
