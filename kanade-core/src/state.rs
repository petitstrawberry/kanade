use serde::{Deserialize, Serialize};

use crate::model::Zone;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub zones: Vec<Zone>,
}

impl PlaybackState {
    pub fn zone(&self, id: &str) -> Option<&Zone> {
        self.zones.iter().find(|z| z.id == id)
    }

    pub fn zone_mut(&mut self, id: &str) -> Option<&mut Zone> {
        self.zones.iter_mut().find(|z| z.id == id)
    }
}
