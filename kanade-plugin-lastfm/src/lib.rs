pub mod client;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::{error, warn};

use kanade_core::model::Track;
use kanade_core::plugin::{KanadePlugin, PlaybackEvent};

pub use client::{LastFmClient, LastFmError, ScrobbleTrack};

#[async_trait]
trait LastFmApi: Send + Sync {
    async fn update_now_playing(&self, track: &ScrobbleTrack) -> Result<(), LastFmError>;
    async fn scrobble(&self, track: &ScrobbleTrack, timestamp: i64) -> Result<(), LastFmError>;
}

#[async_trait]
impl LastFmApi for LastFmClient {
    async fn update_now_playing(&self, track: &ScrobbleTrack) -> Result<(), LastFmError> {
        LastFmClient::update_now_playing(self, track).await
    }

    async fn scrobble(&self, track: &ScrobbleTrack, timestamp: i64) -> Result<(), LastFmError> {
        LastFmClient::scrobble(self, track, timestamp).await
    }
}

pub struct LastFmScrobbler {
    client: Arc<dyn LastFmApi>,
    current_track_started: Arc<RwLock<Option<(String, DateTime<Utc>)>>>,
    pending_scrobbles: Arc<RwLock<Vec<(ScrobbleTrack, i64)>>>,
}

impl LastFmScrobbler {
    pub fn from_env() -> Result<Self, LastFmError> {
        let api_key = std::env::var("LASTFM_API_KEY").map_err(|_| {
            LastFmError::Config("Missing environment variable: LASTFM_API_KEY".to_string())
        })?;
        let secret = std::env::var("LASTFM_SECRET").map_err(|_| {
            LastFmError::Config("Missing environment variable: LASTFM_SECRET".to_string())
        })?;
        let session_key = std::env::var("LASTFM_SESSION_KEY").map_err(|_| {
            LastFmError::Config("Missing environment variable: LASTFM_SESSION_KEY".to_string())
        })?;

        // Session key is expected from a one-time external Last.fm auth flow (getToken -> authorize -> getSession).

        let client = LastFmClient::new(api_key, secret, session_key);
        Ok(Self::with_client(client))
    }

    pub fn new(client: LastFmClient) -> Self {
        Self::with_client(client)
    }

    fn with_client<T>(client: T) -> Self
    where
        T: LastFmApi + 'static,
    {
        Self {
            client: Arc::new(client),
            current_track_started: Arc::new(RwLock::new(None)),
            pending_scrobbles: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl ScrobbleTrack {
    fn from_track(track: &Track) -> Self {
        Self {
            artist: track
                .artist
                .clone()
                .unwrap_or_else(|| "Unknown Artist".to_string()),
            title: track
                .title
                .clone()
                .unwrap_or_else(|| "Unknown Track".to_string()),
            album: track.album_title.clone(),
            album_artist: track.album_artist.clone(),
            duration_secs: track.duration_secs.map(|duration| duration as u32),
            track_number: track.track_number,
        }
    }
}

#[async_trait]
impl KanadePlugin for LastFmScrobbler {
    fn name(&self) -> &str {
        "lastfm-scrobbler"
    }

    async fn on_event(&self, event: &PlaybackEvent) {
        match event {
            PlaybackEvent::TrackChanged { current, .. } => {
                let client = Arc::clone(&self.client);
                let current_track_started = Arc::clone(&self.current_track_started);
                let pending_scrobbles = Arc::clone(&self.pending_scrobbles);
                let current = current.clone();

                tokio::spawn(async move {
                    let mut started = current_track_started.write().await;
                    *started = Some((current.id.clone(), Utc::now()));
                    drop(started);

                    flush_pending_scrobbles(Arc::clone(&client), Arc::clone(&pending_scrobbles))
                        .await;

                    let now_playing = ScrobbleTrack::from_track(&current);
                    if let Err(err) = client.update_now_playing(&now_playing).await {
                        if matches!(err, LastFmError::InvalidSession) {
                            error!("last.fm update_now_playing failed: invalid session");
                        } else {
                            warn!(error = %err, "last.fm update_now_playing failed");
                        }
                    }
                });
            }
            PlaybackEvent::ScrobblePoint { track } => {
                let client = Arc::clone(&self.client);
                let current_track_started = Arc::clone(&self.current_track_started);
                let pending_scrobbles = Arc::clone(&self.pending_scrobbles);
                let track = track.clone();

                tokio::spawn(async move {
                    let started = current_track_started.read().await;
                    let timestamp = started
                        .as_ref()
                        .and_then(|(id, when)| (id == &track.id).then_some(when.timestamp()))
                        .unwrap_or_else(|| Utc::now().timestamp());
                    drop(started);

                    let scrobble_track = ScrobbleTrack::from_track(&track);
                    if let Err(err) = client.scrobble(&scrobble_track, timestamp).await {
                        if matches!(err, LastFmError::InvalidSession) {
                            error!("last.fm scrobble failed: invalid session");
                        } else {
                            warn!(error = %err, "last.fm scrobble failed, queued for retry");
                        }
                        let mut pending = pending_scrobbles.write().await;
                        pending.push((scrobble_track, timestamp));
                    }
                });
            }
            PlaybackEvent::PlaybackResumed { .. }
            | PlaybackEvent::PlaybackPaused { .. }
            | PlaybackEvent::PlaybackStopped { .. } => {}
        }
    }
}

async fn flush_pending_scrobbles(
    client: Arc<dyn LastFmApi>,
    pending_scrobbles: Arc<RwLock<Vec<(ScrobbleTrack, i64)>>>,
) {
    let mut pending_batch = {
        let mut pending = pending_scrobbles.write().await;
        let count = pending.len().min(50);
        pending.drain(0..count).collect::<Vec<_>>()
    };

    if pending_batch.is_empty() {
        return;
    }

    let mut failed = Vec::new();
    for (track, timestamp) in pending_batch.drain(..) {
        if let Err(err) = client.scrobble(&track, timestamp).await {
            if matches!(err, LastFmError::InvalidSession) {
                error!("last.fm pending scrobble failed: invalid session");
            } else {
                warn!(error = %err, "last.fm pending scrobble retry failed");
            }
            failed.push((track, timestamp));
        }
    }

    if !failed.is_empty() {
        let mut pending = pending_scrobbles.write().await;
        failed.extend(pending.drain(..));
        *pending = failed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MockApiState {
        now_playing_calls: Vec<ScrobbleTrack>,
        scrobble_calls: Vec<(ScrobbleTrack, i64)>,
        fail_scrobble_count: usize,
    }

    struct MockApi {
        state: Arc<Mutex<MockApiState>>,
    }

    #[async_trait]
    impl LastFmApi for MockApi {
        async fn update_now_playing(&self, track: &ScrobbleTrack) -> Result<(), LastFmError> {
            self.state
                .lock()
                .expect("mock lock poisoned")
                .now_playing_calls
                .push(track.clone());
            Ok(())
        }

        async fn scrobble(&self, track: &ScrobbleTrack, timestamp: i64) -> Result<(), LastFmError> {
            let mut guard = self.state.lock().expect("mock lock poisoned");
            guard.scrobble_calls.push((track.clone(), timestamp));

            if guard.fail_scrobble_count > 0 {
                guard.fail_scrobble_count -= 1;
                return Err(LastFmError::Api {
                    code: 11,
                    message: "temporary failure".to_string(),
                });
            }

            Ok(())
        }
    }

    fn make_track(
        id: &str,
        title: Option<&str>,
        artist: Option<&str>,
        album_artist: Option<&str>,
        album_title: Option<&str>,
        duration_secs: Option<f64>,
        track_number: Option<u32>,
    ) -> Track {
        Track {
            id: id.to_string(),
            file_path: format!("/music/{id}.flac"),
            album_id: None,
            title: title.map(ToString::to_string),
            artist: artist.map(ToString::to_string),
            album_artist: album_artist.map(ToString::to_string),
            album_title: album_title.map(ToString::to_string),
            composer: None,
            genre: None,
            track_number,
            disc_number: None,
            duration_secs,
            format: None,
            sample_rate: None,
        }
    }

    #[test]
    fn from_track_converts_all_present_fields() {
        let track = make_track(
            "track-1",
            Some("Song"),
            Some("Artist"),
            Some("Album Artist"),
            Some("Album"),
            Some(245.9),
            Some(7),
        );

        let scrobble = ScrobbleTrack::from_track(&track);
        assert_eq!(scrobble.artist, "Artist");
        assert_eq!(scrobble.title, "Song");
        assert_eq!(scrobble.album.as_deref(), Some("Album"));
        assert_eq!(scrobble.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(scrobble.duration_secs, Some(245));
        assert_eq!(scrobble.track_number, Some(7));
    }

    #[test]
    fn from_track_uses_defaults_when_optional_fields_missing() {
        let track = make_track("track-2", None, None, None, None, None, None);
        let scrobble = ScrobbleTrack::from_track(&track);

        assert_eq!(scrobble.artist, "Unknown Artist");
        assert_eq!(scrobble.title, "Unknown Track");
        assert_eq!(scrobble.album, None);
        assert_eq!(scrobble.album_artist, None);
        assert_eq!(scrobble.duration_secs, None);
        assert_eq!(scrobble.track_number, None);
    }

    #[tokio::test]
    async fn on_event_updates_now_playing_and_scrobbles() {
        let state = Arc::new(Mutex::new(MockApiState::default()));
        let plugin = LastFmScrobbler::with_client(MockApi {
            state: Arc::clone(&state),
        });
        let track = make_track(
            "track-3",
            Some("Song 3"),
            Some("Artist 3"),
            None,
            None,
            Some(180.0),
            Some(3),
        );

        plugin
            .on_event(&PlaybackEvent::TrackChanged {
                previous: None,
                current: track.clone(),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        plugin
            .on_event(&PlaybackEvent::ScrobblePoint { track })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        let guard = state.lock().expect("mock lock poisoned");
        assert_eq!(guard.now_playing_calls.len(), 1);
        assert_eq!(guard.scrobble_calls.len(), 1);
    }

    #[tokio::test]
    async fn failed_scrobbles_are_retried_on_next_track_change() {
        let state = Arc::new(Mutex::new(MockApiState {
            fail_scrobble_count: 1,
            ..Default::default()
        }));
        let plugin = LastFmScrobbler::with_client(MockApi {
            state: Arc::clone(&state),
        });

        let first = make_track(
            "track-a",
            Some("A"),
            Some("Artist"),
            None,
            None,
            Some(200.0),
            Some(1),
        );
        let second = make_track(
            "track-b",
            Some("B"),
            Some("Artist"),
            None,
            None,
            Some(210.0),
            Some(2),
        );

        plugin
            .on_event(&PlaybackEvent::TrackChanged {
                previous: None,
                current: first.clone(),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        plugin
            .on_event(&PlaybackEvent::ScrobblePoint {
                track: first.clone(),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        {
            let pending = plugin.pending_scrobbles.read().await;
            assert_eq!(pending.len(), 1);
        }

        plugin
            .on_event(&PlaybackEvent::TrackChanged {
                previous: Some(first),
                current: second,
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;

        let pending = plugin.pending_scrobbles.read().await;
        assert!(pending.is_empty());
    }
}
