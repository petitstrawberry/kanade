use std::collections::BTreeMap;

use md5::{Digest, Md5};
use quick_xml::de::from_str;
use serde::Deserialize;
use tracing::warn;

const LASTFM_API_URL: &str = "https://ws.audioscrobbler.com/2.0/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrobbleTrack {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration_secs: Option<u32>,
    pub track_number: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum LastFmError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {code}: {message}")]
    Api { code: u32, message: String },
    #[error("Invalid session key — re-authentication required")]
    InvalidSession,
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Failed to parse Last.fm XML response: {0}")]
    Xml(String),
}

#[derive(Clone)]
pub struct LastFmClient {
    api_key: String,
    secret: String,
    session_key: String,
    http: reqwest::Client,
}

impl LastFmClient {
    pub fn new(api_key: String, secret: String, session_key: String) -> Self {
        Self {
            api_key,
            secret,
            session_key,
            http: reqwest::Client::new(),
        }
    }

    pub async fn update_now_playing(&self, track: &ScrobbleTrack) -> Result<(), LastFmError> {
        let mut params = BTreeMap::new();
        params.insert("method".to_string(), "track.updateNowPlaying".to_string());
        params.insert("artist".to_string(), track.artist.clone());
        params.insert("track".to_string(), track.title.clone());
        params.insert("api_key".to_string(), self.api_key.clone());
        params.insert("sk".to_string(), self.session_key.clone());

        if let Some(album) = &track.album {
            params.insert("album".to_string(), album.clone());
        }
        if let Some(album_artist) = &track.album_artist {
            params.insert("albumArtist".to_string(), album_artist.clone());
        }
        if let Some(duration) = track.duration_secs {
            params.insert("duration".to_string(), duration.to_string());
        }
        if let Some(track_number) = track.track_number {
            params.insert("trackNumber".to_string(), track_number.to_string());
        }

        let response = self.send_signed_request(params).await?;
        response.ensure_ok()?;

        if let Some(now_playing) = response.now_playing {
            if let Some(ignored) = now_playing.ignored_message {
                if ignored.code.unwrap_or(0) != 0 {
                    warn!(
                        code = ignored.code.unwrap_or(0),
                        message = ignored.message.as_deref().unwrap_or(""),
                        "last.fm updateNowPlaying ignored"
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn scrobble(&self, track: &ScrobbleTrack, timestamp: i64) -> Result<(), LastFmError> {
        let mut params = BTreeMap::new();
        params.insert("method".to_string(), "track.scrobble".to_string());
        params.insert("artist[0]".to_string(), track.artist.clone());
        params.insert("track[0]".to_string(), track.title.clone());
        params.insert("timestamp[0]".to_string(), timestamp.to_string());
        params.insert("api_key".to_string(), self.api_key.clone());
        params.insert("sk".to_string(), self.session_key.clone());

        if let Some(album) = &track.album {
            params.insert("album[0]".to_string(), album.clone());
        }
        if let Some(album_artist) = &track.album_artist {
            params.insert("albumArtist[0]".to_string(), album_artist.clone());
        }
        if let Some(duration) = track.duration_secs {
            params.insert("duration[0]".to_string(), duration.to_string());
        }
        if let Some(track_number) = track.track_number {
            params.insert("trackNumber[0]".to_string(), track_number.to_string());
        }

        let response = self.send_signed_request(params).await?;
        response.ensure_ok()?;

        if let Some(scrobbles) = response.scrobbles {
            if scrobbles.accepted.unwrap_or(0) == 0 {
                return Err(LastFmError::Api {
                    code: 0,
                    message: "Scrobble was not accepted".to_string(),
                });
            }

            if let Some(scrobble) = scrobbles.scrobble {
                if let Some(ignored) = scrobble.ignored_message {
                    if ignored.code.unwrap_or(0) != 0 {
                        warn!(
                            code = ignored.code.unwrap_or(0),
                            message = ignored.message.as_deref().unwrap_or(""),
                            "last.fm scrobble ignored"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn sign(&self, params: &BTreeMap<String, String>) -> String {
        let mut payload = String::new();

        for (key, value) in params {
            if key == "format" || key == "api_sig" {
                continue;
            }
            payload.push_str(key);
            payload.push_str(value);
        }

        payload.push_str(&self.secret);

        let mut hasher = Md5::new();
        hasher.update(payload.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    async fn send_signed_request(
        &self,
        mut params: BTreeMap<String, String>,
    ) -> Result<LfmResponse, LastFmError> {
        let api_sig = self.sign(&params);
        params.insert("api_sig".to_string(), api_sig);

        let body = self
            .http
            .post(LASTFM_API_URL)
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        from_str::<LfmResponse>(&body).map_err(|error| LastFmError::Xml(error.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct LfmResponse {
    #[serde(rename = "@status")]
    status: String,
    error: Option<LfmErrorNode>,
    #[serde(rename = "nowplaying")]
    now_playing: Option<NowPlayingNode>,
    scrobbles: Option<ScrobblesNode>,
}

impl LfmResponse {
    fn ensure_ok(&self) -> Result<(), LastFmError> {
        if self.status == "ok" {
            return Ok(());
        }

        if let Some(error) = &self.error {
            if error.code == 9 || error.code == 29 {
                return Err(LastFmError::InvalidSession);
            }

            return Err(LastFmError::Api {
                code: error.code,
                message: error.message.clone(),
            });
        }

        Err(LastFmError::Api {
            code: 0,
            message: "Last.fm request failed without error details".to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct LfmErrorNode {
    #[serde(rename = "@code")]
    code: u32,
    #[serde(rename = "$text")]
    message: String,
}

#[derive(Debug, Deserialize)]
struct NowPlayingNode {
    #[serde(rename = "ignoredMessage")]
    ignored_message: Option<IgnoredMessageNode>,
}

#[derive(Debug, Deserialize)]
struct ScrobblesNode {
    #[serde(rename = "@accepted")]
    accepted: Option<u32>,
    scrobble: Option<ScrobbleNode>,
}

#[derive(Debug, Deserialize)]
struct ScrobbleNode {
    #[serde(rename = "ignoredMessage")]
    ignored_message: Option<IgnoredMessageNode>,
}

#[derive(Debug, Deserialize)]
struct IgnoredMessageNode {
    #[serde(rename = "@code")]
    code: Option<u32>,
    #[serde(rename = "$text")]
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_uses_sorted_params_and_secret() {
        let client = LastFmClient::new(
            "test_api_key".to_string(),
            "test_secret".to_string(),
            "test_session".to_string(),
        );

        let mut params = BTreeMap::new();
        params.insert("api_key".to_string(), "test_api_key".to_string());
        params.insert("method".to_string(), "auth.getToken".to_string());

        let signature = client.sign(&params);
        assert_eq!(signature, "2f81794ceb728b02eede68d1baf13a28");
    }
}
