use std::collections::HashSet;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::warn;

use crate::model::{PlaybackStatus, RepeatMode, Track};
use crate::ports::EventBroadcaster;
use crate::state::PlaybackState;

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    /// Active track changed (covers both "new track started" and "previous track finished")
    TrackChanged {
        previous: Option<Track>,
        current: Track,
    },
    /// Playback was paused
    PlaybackPaused {
        track: Option<Track>,
        position_secs: f64,
    },
    /// Playback was resumed (status went from Paused/Stopped to Playing WITHOUT track change)
    PlaybackResumed {
        track: Option<Track>,
        position_secs: f64,
    },
    /// Playback was stopped
    PlaybackStopped { track: Option<Track> },
    /// Track reached scrobble threshold: played for min(50% duration, 4 minutes), duration > 30s
    ScrobblePoint { track: Track },
}

#[async_trait]
pub trait KanadePlugin: Send + Sync {
    /// Unique plugin identifier (e.g., "lastfm-scrobbler")
    fn name(&self) -> &str;
    /// Called when a playback event occurs. MUST NOT block — spawn async tasks for I/O.
    async fn on_event(&self, event: &PlaybackEvent);
}

pub struct PluginBridge {
    plugins: StdRwLock<Vec<Arc<dyn KanadePlugin>>>,
    prev: RwLock<PrevState>,
}

struct PrevState {
    state: PlaybackState,
    scrobbled_tracks: HashSet<String>,
}

impl PrevState {
    fn new() -> Self {
        Self {
            state: PlaybackState {
                nodes: Vec::new(),
                selected_node_id: None,
                queue: Vec::new(),
                current_index: None,
                shuffle: false,
                repeat: RepeatMode::Off,
            },
            scrobbled_tracks: HashSet::new(),
        }
    }
}

impl PluginBridge {
    pub fn new(plugins: Vec<Arc<dyn KanadePlugin>>) -> Self {
        Self {
            plugins: StdRwLock::new(plugins),
            prev: RwLock::new(PrevState::new()),
        }
    }

    /// Register a plugin at runtime
    pub fn register(&self, plugin: Arc<dyn KanadePlugin>) {
        self.plugins
            .write()
            .expect("plugin list lock poisoned")
            .push(plugin);
    }

    fn diff_events(prev: &mut PrevState, state: &PlaybackState) -> Vec<PlaybackEvent> {
        let mut events = Vec::new();

        let prev_track = prev.state.current_track().cloned();
        let current_track = state.current_track().cloned();
        let track_changed =
            state.current_index != prev.state.current_index && current_track.is_some();

        if track_changed {
            events.push(PlaybackEvent::TrackChanged {
                previous: prev_track,
                current: current_track.clone().expect("current track must exist"),
            });
        }

        let prev_node = prev
            .state
            .selected_node_id
            .as_deref()
            .and_then(|id| prev.state.node(id));
        let node = state
            .selected_node_id
            .as_deref()
            .and_then(|id| state.node(id));

        if let (Some(prev_node), Some(node)) = (prev_node, node) {
            match (prev_node.status, node.status) {
                (PlaybackStatus::Playing, PlaybackStatus::Paused) => {
                    events.push(PlaybackEvent::PlaybackPaused {
                        track: current_track.clone(),
                        position_secs: node.position_secs,
                    });
                }
                (PlaybackStatus::Paused | PlaybackStatus::Stopped, PlaybackStatus::Playing)
                    if state.current_index == prev.state.current_index =>
                {
                    events.push(PlaybackEvent::PlaybackResumed {
                        track: current_track.clone(),
                        position_secs: node.position_secs,
                    });
                }
                (previous, PlaybackStatus::Stopped)
                    if previous != PlaybackStatus::Stopped && !track_changed =>
                {
                    events.push(PlaybackEvent::PlaybackStopped {
                        track: current_track.clone(),
                    });
                }
                _ => {}
            }

            if node.status == PlaybackStatus::Playing {
                if let Some(track) = current_track.as_ref() {
                    if let Some(duration_secs) = track.duration_secs {
                        if duration_secs > 30.0 && !prev.scrobbled_tracks.contains(&track.id) {
                            let threshold_secs = (duration_secs * 0.5).min(240.0);
                            if node.position_secs >= threshold_secs {
                                events.push(PlaybackEvent::ScrobblePoint {
                                    track: track.clone(),
                                });
                                prev.scrobbled_tracks.insert(track.id.clone());
                            }
                        }
                    }
                }
            }
        }

        prev.state = state.clone();
        events
    }
}

#[async_trait]
impl EventBroadcaster for PluginBridge {
    async fn on_state_changed(&self, state: &PlaybackState) {
        let events = {
            let mut prev = self.prev.write().await;
            Self::diff_events(&mut prev, state)
        };

        if events.is_empty() {
            return;
        }

        let plugins = self
            .plugins
            .read()
            .expect("plugin list lock poisoned")
            .clone();

        for event in events {
            for plugin in &plugins {
                let plugin = Arc::clone(plugin);
                let plugin_name = plugin.name().to_string();
                let event = event.clone();

                if let Err(error) = tokio::spawn(async move {
                    plugin.on_event(&event).await;
                })
                .await
                {
                    warn!(plugin = %plugin_name, %error, "plugin handler failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::model::Node;

    struct MockPlugin {
        name: String,
        events: Mutex<Vec<PlaybackEvent>>,
    }

    impl MockPlugin {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<PlaybackEvent> {
            self.events.lock().expect("events lock poisoned").clone()
        }

        fn clear(&self) {
            self.events.lock().expect("events lock poisoned").clear();
        }
    }

    #[async_trait]
    impl KanadePlugin for MockPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        async fn on_event(&self, event: &PlaybackEvent) {
            self.events
                .lock()
                .expect("events lock poisoned")
                .push(event.clone());
        }
    }

    fn track(id: &str, duration_secs: Option<f64>) -> Track {
        Track {
            id: id.to_string(),
            file_path: format!("/music/{id}.flac"),
            album_id: None,
            title: Some(id.to_string()),
            artist: None,
            album_artist: None,
            album_title: None,
            composer: None,
            genre: None,
            track_number: None,
            disc_number: None,
            duration_secs,
            format: None,
            sample_rate: None,
        }
    }

    fn state(
        queue: Vec<Track>,
        current_index: Option<usize>,
        status: PlaybackStatus,
        position_secs: f64,
    ) -> PlaybackState {
        PlaybackState {
            nodes: vec![Node {
                id: "default".to_string(),
                name: "default".to_string(),
                status,
                position_secs,
                ..Default::default()
            }],
            selected_node_id: Some("default".to_string()),
            queue,
            current_index,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }

    #[tokio::test]
    async fn track_changed_fires_when_current_index_changes() {
        let plugin = Arc::new(MockPlugin::new("p1"));
        let bridge = PluginBridge::new(vec![plugin.clone() as Arc<dyn KanadePlugin>]);

        bridge
            .on_state_changed(&state(
                vec![track("a", Some(300.0)), track("b", Some(300.0))],
                Some(0),
                PlaybackStatus::Playing,
                0.0,
            ))
            .await;
        plugin.clear();

        bridge
            .on_state_changed(&state(
                vec![track("a", Some(300.0)), track("b", Some(300.0))],
                Some(1),
                PlaybackStatus::Playing,
                0.0,
            ))
            .await;

        let events = plugin.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            PlaybackEvent::TrackChanged {
                previous: Some(track("a", Some(300.0))),
                current: track("b", Some(300.0)),
            }
        );
    }

    #[tokio::test]
    async fn playback_paused_resumed_and_stopped_fire_on_status_transitions() {
        let plugin = Arc::new(MockPlugin::new("p1"));
        let bridge = PluginBridge::new(vec![plugin.clone() as Arc<dyn KanadePlugin>]);
        let queue = vec![track("a", Some(300.0))];

        bridge
            .on_state_changed(&state(
                queue.clone(),
                Some(0),
                PlaybackStatus::Playing,
                10.0,
            ))
            .await;
        plugin.clear();

        bridge
            .on_state_changed(&state(queue.clone(), Some(0), PlaybackStatus::Paused, 25.0))
            .await;
        assert_eq!(
            plugin.events(),
            vec![PlaybackEvent::PlaybackPaused {
                track: Some(track("a", Some(300.0))),
                position_secs: 25.0,
            }]
        );

        plugin.clear();
        bridge
            .on_state_changed(&state(
                queue.clone(),
                Some(0),
                PlaybackStatus::Playing,
                26.0,
            ))
            .await;
        assert_eq!(
            plugin.events(),
            vec![PlaybackEvent::PlaybackResumed {
                track: Some(track("a", Some(300.0))),
                position_secs: 26.0,
            }]
        );

        plugin.clear();
        bridge
            .on_state_changed(&state(queue, Some(0), PlaybackStatus::Stopped, 0.0))
            .await;
        assert_eq!(
            plugin.events(),
            vec![PlaybackEvent::PlaybackStopped {
                track: Some(track("a", Some(300.0))),
            }]
        );
    }

    #[tokio::test]
    async fn scrobble_point_fires_when_position_crosses_threshold() {
        let plugin = Arc::new(MockPlugin::new("p1"));
        let bridge = PluginBridge::new(vec![plugin.clone() as Arc<dyn KanadePlugin>]);
        let queue = vec![track("a", Some(300.0))];

        bridge
            .on_state_changed(&state(
                queue.clone(),
                Some(0),
                PlaybackStatus::Playing,
                100.0,
            ))
            .await;
        plugin.clear();

        bridge
            .on_state_changed(&state(queue, Some(0), PlaybackStatus::Playing, 150.0))
            .await;

        assert_eq!(
            plugin.events(),
            vec![PlaybackEvent::ScrobblePoint {
                track: track("a", Some(300.0)),
            }]
        );
    }

    #[tokio::test]
    async fn scrobble_point_does_not_fire_for_short_tracks() {
        let plugin = Arc::new(MockPlugin::new("p1"));
        let bridge = PluginBridge::new(vec![plugin.clone() as Arc<dyn KanadePlugin>]);

        bridge
            .on_state_changed(&state(
                vec![track("short", Some(20.0))],
                Some(0),
                PlaybackStatus::Playing,
                19.0,
            ))
            .await;

        let events = plugin.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PlaybackEvent::TrackChanged { .. }));
    }

    #[tokio::test]
    async fn scrobble_point_does_not_fire_twice_for_same_track() {
        let plugin = Arc::new(MockPlugin::new("p1"));
        let bridge = PluginBridge::new(vec![plugin.clone() as Arc<dyn KanadePlugin>]);
        let queue = vec![track("a", Some(200.0))];

        bridge
            .on_state_changed(&state(
                queue.clone(),
                Some(0),
                PlaybackStatus::Playing,
                110.0,
            ))
            .await;
        plugin.clear();

        bridge
            .on_state_changed(&state(
                queue.clone(),
                Some(0),
                PlaybackStatus::Playing,
                120.0,
            ))
            .await;
        bridge
            .on_state_changed(&state(queue, Some(0), PlaybackStatus::Playing, 180.0))
            .await;

        assert_eq!(
            plugin.events(),
            vec![PlaybackEvent::ScrobblePoint {
                track: track("a", Some(200.0)),
            }]
        );
    }

    #[tokio::test]
    async fn multiple_plugins_receive_all_events() {
        let plugin_a = Arc::new(MockPlugin::new("a"));
        let plugin_b = Arc::new(MockPlugin::new("b"));

        let bridge = PluginBridge::new(vec![
            plugin_a.clone() as Arc<dyn KanadePlugin>,
            plugin_b.clone() as Arc<dyn KanadePlugin>,
        ]);

        bridge
            .on_state_changed(&state(
                vec![track("a", Some(300.0))],
                Some(0),
                PlaybackStatus::Playing,
                0.0,
            ))
            .await;

        assert_eq!(plugin_a.events().len(), 1);
        assert_eq!(plugin_b.events().len(), 1);
        assert!(matches!(
            plugin_a.events()[0],
            PlaybackEvent::TrackChanged { .. }
        ));
        assert!(matches!(
            plugin_b.events()[0],
            PlaybackEvent::TrackChanged { .. }
        ));
    }
}
