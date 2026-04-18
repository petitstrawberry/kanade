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

    pub fn selected_node(&self) -> Option<&Node> {
        self.selected_node_id
            .as_deref()
            .and_then(|id| self.node(id))
    }

    pub fn selected_node_mut(&mut self) -> Option<&mut Node> {
        let id = self.selected_node_id.clone()?;
        self.node_mut(&id)
    }

    pub fn sync_top_level_from_selected_node(&mut self) {
        if let Some((queue, current_index, shuffle, repeat)) = self.selected_node().map(|node| {
            (
                node.queue.clone(),
                node.current_index,
                node.shuffle,
                node.repeat,
            )
        }) {
            self.queue = queue;
            self.current_index = current_index;
            self.shuffle = shuffle;
            self.repeat = repeat;
        } else {
            self.queue.clear();
            self.current_index = None;
            self.shuffle = false;
            self.repeat = RepeatMode::Off;
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.selected_node()
            .and_then(|n| n.current_index.and_then(|i| n.queue.get(i)))
            .or_else(|| self.current_index.and_then(|i| self.queue.get(i)))
    }
}
