use serde::{Deserialize, Serialize};

use crate::model::{Node, RepeatMode, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub nodes: Vec<Node>,
    #[serde(default, alias = "active_output_id", alias = "active_output_node_id")]
    pub selected_node_id: Option<String>,
    pub queue: Vec<Track>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl PlaybackState {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|i| self.queue.get(i))
    }
}
