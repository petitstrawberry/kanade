use serde::{Deserialize, Serialize};

use crate::model::Node;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub nodes: Vec<Node>,
}

impl PlaybackState {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
}
