use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, instrument, warn};

use kanade_core::{error::CoreError, ports::AudioOutput};
use sha2::{Digest, Sha256};

use crate::client::MpdClient;

type HmacSha256 = Hmac<Sha256>;
/// MPD fetches queued URLs lazily, so node-signed media URLs need a longer TTL
/// than the interactive clients that request fresh URLs on demand.
const MEDIA_URL_TTL_SECS: u64 = 24 * 60 * 60;

struct MediaAuth {
    key_id: String,
    key: [u8; 32],
}

/// [`AudioOutput`] implementation that controls a local MPD daemon.
///
/// All operations translate directly to the corresponding MPD protocol
/// commands, which are sent over TCP.
pub struct MpdRenderer {
    client: MpdClient,
    media_public_base_url: String,
    media_auth: Option<MediaAuth>,
}

impl MpdRenderer {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        media_public_base_url: impl Into<String>,
    ) -> Self {
        Self {
            client: MpdClient::new(host, port),
            media_public_base_url: media_public_base_url
                .into()
                .trim_end_matches('/')
                .to_string(),
            media_auth: None,
        }
    }

    pub fn new_with_media_auth(
        host: impl Into<String>,
        port: u16,
        media_public_base_url: impl Into<String>,
        media_auth_key_id: impl Into<String>,
        media_auth_key: [u8; 32],
    ) -> Self {
        Self {
            client: MpdClient::new(host, port),
            media_public_base_url: media_public_base_url
                .into()
                .trim_end_matches('/')
                .to_string(),
            media_auth: Some(MediaAuth {
                key_id: media_auth_key_id.into(),
                key: media_auth_key,
            }),
        }
    }

    fn media_uri(&self, value: &str) -> String {
        self.media_uri_at(value, current_unix_timestamp())
    }

    fn media_uri_at(&self, value: &str, now: u64) -> String {
        if value.starts_with("http://") || value.starts_with("https://") {
            return value.to_string();
        }

        let mut hasher = Sha256::new();
        hasher.update(value.as_bytes());
        let track_id = hex::encode(hasher.finalize());
        let path = format!("/media/tracks/{track_id}");

        if let Some(auth) = &self.media_auth {
            let exp = now.saturating_add(MEDIA_URL_TTL_SECS);
            let sig = hex::encode(compute_media_signature(&auth.key, &path, exp));
            return format!(
                "{}{}?kid={}&exp={}&sig={}",
                self.media_public_base_url, path, auth.key_id, exp, sig
            );
        }

        format!("{}{}", self.media_public_base_url, path)
    }

    fn quote_mpd_arg(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

fn compute_media_signature(key: &[u8; 32], path: &str, exp: u64) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("32-byte HMAC key should be valid");
    mac.update(format!("GET:{path}:{exp}").as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn current_unix_timestamp() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(e) => {
            let fallback = u64::MAX.saturating_sub(MEDIA_URL_TTL_SECS);
            warn!(error = %e, fallback, "system clock is before UNIX_EPOCH, using fallback timestamp for media URL signing");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_uri_without_auth_uses_plain_track_url() {
        let renderer = MpdRenderer::new("127.0.0.1", 6600, "https://example.com/");
        let uri = renderer.media_uri_at("/music/track.flac", 1_700_000_000);
        assert_eq!(
            uri,
            "https://example.com/media/tracks/4d7bca5e140a88117639c432b89240f072969fea064dece62c8ba745c0daf141"
        );
    }

    #[test]
    fn media_uri_with_auth_adds_signed_query() {
        let renderer = MpdRenderer::new_with_media_auth(
            "127.0.0.1",
            6600,
            "https://example.com/",
            "kid-123",
            [0x11; 32],
        );
        let uri = renderer.media_uri_at("/music/track.flac", 1_700_000_000);
        assert_eq!(
            uri,
            "https://example.com/media/tracks/4d7bca5e140a88117639c432b89240f072969fea064dece62c8ba745c0daf141?kid=kid-123&exp=1700086400&sig=95f083899bfb5eeb67ab0662a2ab2c8da2edb9387bc3342c9bcd48182176d2d2"
        );
    }

    #[test]
    fn media_uri_keeps_absolute_http_urls_unchanged() {
        let renderer = MpdRenderer::new_with_media_auth(
            "127.0.0.1",
            6600,
            "https://example.com/",
            "kid-123",
            [0x11; 32],
        );
        let uri = renderer.media_uri_at("https://cdn.example.com/a.flac", 1_700_000_000);
        assert_eq!(uri, "https://cdn.example.com/a.flac");
    }
}

#[async_trait]
impl AudioOutput for MpdRenderer {
    #[instrument(skip(self))]
    async fn play(&self) -> Result<(), CoreError> {
        info!("mpd-renderer: play");
        self.client.send("play\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn pause(&self) -> Result<(), CoreError> {
        self.client.send("pause 1\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn stop(&self) -> Result<(), CoreError> {
        info!("mpd-renderer: stop");
        self.client.send("stop\n").await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn seek(&self, position_secs: f64) -> Result<(), CoreError> {
        self.client
            .send(&format!("seekcur {position_secs:.3}\n"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn set_volume(&self, volume: u8) -> Result<(), CoreError> {
        self.client.send(&format!("setvol {volume}\n")).await?;
        Ok(())
    }

    /// Replace the MPD queue with the given list of file paths.
    ///
    /// Uses a `command_list` to atomically clear and re-populate the queue.
    #[instrument(skip(self, file_paths))]
    async fn set_queue(
        &self,
        file_paths: &[String],
        _projection_generation: u64,
    ) -> Result<(), CoreError> {
        info!(queue_len = file_paths.len(), "mpd-renderer: set_queue");
        let mut cmd = String::from("command_list_begin\nclear\n");
        for path in file_paths {
            let uri = self.media_uri(path);
            cmd.push_str(&format!("add {}\n", Self::quote_mpd_arg(&uri)));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }

    /// Append file paths to the MPD queue.
    #[instrument(skip(self, file_paths))]
    async fn add(&self, file_paths: &[String]) -> Result<(), CoreError> {
        let mut cmd = String::from("command_list_begin\n");
        for path in file_paths {
            let uri = self.media_uri(path);
            cmd.push_str(&format!("add {}\n", Self::quote_mpd_arg(&uri)));
        }
        cmd.push_str("command_list_end\n");
        self.client.send(&cmd).await?;
        Ok(())
    }

    /// Remove the track at the given position from the MPD queue.
    #[instrument(skip(self))]
    async fn remove(&self, index: usize) -> Result<(), CoreError> {
        self.client.send(&format!("delete {index}\n")).await?;
        Ok(())
    }

    /// Move the track at `from` position to `to` position in the MPD queue.
    #[instrument(skip(self))]
    async fn move_track(&self, from: usize, to: usize) -> Result<(), CoreError> {
        self.client.send(&format!("move {from} {to}\n")).await?;
        Ok(())
    }
}

impl MpdRenderer {
    #[instrument(skip(self))]
    pub async fn clear(&self) -> Result<(), CoreError> {
        self.client.send("stop\nclear\n").await?;
        Ok(())
    }
}
