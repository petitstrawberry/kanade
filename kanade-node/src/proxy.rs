//! Local HTTP media proxy for `kanade-node`.
//!
//! MPD resolves queued URLs lazily — only when playback reaches each track.
//! Signed URLs added to the queue can therefore expire before MPD ever fetches
//! them.  To fix this without extending the TTL we run a loopback HTTP proxy:
//!
//! 1. `MpdRenderer` receives the proxy base URL (e.g.
//!    `http://127.0.0.1:{proxy_port}`) as its media base URL (no auth key
//!    embedded in the URL).
//! 2. MPD stores and eventually requests
//!    `{proxy_base_url}/media/tracks/{track_id}`.
//! 3. The proxy looks up the current session's signing key, computes a **fresh**
//!    signed URL against the real Kanade server, and returns `HTTP 302 Found`.
//! 4. MPD follows the redirect and fetches the freshly signed URL, which is
//!    always within its 15-minute window.
//!
//! The proxy's auth state (`ProxyState`) is a `tokio::sync::RwLock` updated at
//! the start of every server session.  The proxy task itself is long-lived and
//! survives reconnections.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

type HmacSha256 = Hmac<Sha256>;

/// Fresh-sign TTL for proxy-generated redirect URLs.
/// This matches the server-side policy; the URL is signed at the moment MPD
/// requests the track so it is always freshly within this window.
const MEDIA_URL_TTL_SECS: u64 = 15 * 60;

struct ProxyState {
    kanade_base_url: String,
    /// `(key_id, key)` when the server issued auth credentials.
    auth: Option<(String, [u8; 32])>,
}

/// Long-lived local HTTP proxy that re-signs Kanade media URLs on demand.
///
/// Clone-cheap: the inner state is reference-counted.
#[derive(Clone)]
pub struct MediaProxy {
    state: Arc<RwLock<Option<ProxyState>>>,
    bind_addr: String,
    base_url: String,
}

impl MediaProxy {
    /// Create a new proxy.
    ///
    /// * `bind_addr` — the address the proxy listens on (e.g. `"127.0.0.1:18080"`).
    /// * `base_url`  — the URL that MPD should use to reach the proxy
    ///   (e.g. `"http://127.0.0.1:18080"`).  When empty the URL is derived
    ///   from `bind_addr` by prepending `"http://"`.
    pub fn new(bind_addr: String, base_url: String) -> Self {
        let base_url = if base_url.is_empty() {
            format!("http://{bind_addr}")
        } else {
            base_url
        };
        Self {
            state: Arc::new(RwLock::new(None)),
            bind_addr,
            base_url,
        }
    }

    /// Update the proxy with the credentials from the latest server session.
    ///
    /// Pass `auth = None` when the server did not issue an auth key.
    pub async fn update(&self, kanade_base_url: String, auth: Option<(String, [u8; 32])>) {
        let mut guard = self.state.write().await;
        *guard = Some(ProxyState {
            kanade_base_url,
            auth,
        });
    }

    /// The base URL that `MpdRenderer` should use for its media URIs.
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    /// Run the proxy accept loop.  Never returns under normal operation.
    pub async fn run(self) {
        let addr = &self.bind_addr;
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                warn!("MediaProxy: failed to bind {addr}: {e}");
                return;
            }
        };
        info!("Media proxy listening on {addr}");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let state = Arc::clone(&self.state);
                    tokio::spawn(handle_request(stream, state));
                }
                Err(e) => warn!("MediaProxy: accept error: {e}"),
            }
        }
    }
}

async fn handle_request(stream: tokio::net::TcpStream, state: Arc<RwLock<Option<ProxyState>>>) {
    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);

    // Read request line.
    let mut request_line = String::new();
    if let Err(e) = reader.read_line(&mut request_line).await {
        debug!("MediaProxy: failed to read request line: {e}");
        return;
    }

    // Drain headers — we do not use them, but must consume them before
    // writing the response.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(_) if line.trim().is_empty() => break,
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = write_response(&mut writer, 400, None).await;
        return;
    }

    // Strip query string — MPD may append its own parameters.
    let path_only = parts[1].split('?').next().unwrap();

    let Some(track_id) = path_only.strip_prefix("/media/tracks/") else {
        let _ = write_response(&mut writer, 404, None).await;
        return;
    };

    let guard = state.read().await;
    let redirect_url = match &*guard {
        None => {
            // No session connected yet.
            drop(guard);
            let _ = write_response(&mut writer, 503, None).await;
            return;
        }
        Some(st) => {
            let track_path = format!("/media/tracks/{track_id}");
            match &st.auth {
                Some((key_id, key)) => {
                    let now = match current_timestamp() {
                        Some(t) => t,
                        None => {
                            warn!("MediaProxy: system clock unavailable, cannot sign URL");
                            drop(guard);
                            let _ = write_response(&mut writer, 503, None).await;
                            return;
                        }
                    };
                    let exp = now.saturating_add(MEDIA_URL_TTL_SECS);
                    let sig = sign(key, &track_path, exp);
                    format!(
                        "{}{track_path}?kid={key_id}&exp={exp}&sig={}",
                        st.kanade_base_url.trim_end_matches('/'),
                        hex::encode(sig),
                    )
                }
                None => {
                    // Server did not issue auth; forward without signature.
                    format!("{}{track_path}", st.kanade_base_url.trim_end_matches('/'))
                }
            }
        }
    };
    drop(guard);

    debug!(%redirect_url, "MediaProxy: redirecting {path_only}");
    let _ = write_response(&mut writer, 302, Some(&redirect_url)).await;
}

async fn write_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: u16,
    location: Option<&str>,
) -> Result<(), ()> {
    let reason = match status {
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut response = format!("HTTP/1.1 {status} {reason}\r\n");
    if let Some(loc) = location {
        response.push_str(&format!("Location: {loc}\r\n"));
    }
    response.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
    writer.write_all(response.as_bytes()).await.map_err(|_| ())
}

fn sign(key: &[u8; 32], path: &str, exp: u64) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("32-byte key is always valid for HMAC");
    mac.update(format!("GET:{path}:{exp}").as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn current_timestamp() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The proxy must produce the same signature as the server-side verifier.
    /// Reference values computed with the same HMAC-SHA256 implementation.
    #[test]
    fn sign_matches_expected_hmac() {
        let key = [0x11u8; 32];
        let path = "/media/tracks/abc123";
        let exp = 1_700_000_900u64;
        let sig = hex::encode(sign(&key, path, exp));
        // HMAC-SHA256 over "GET:/media/tracks/abc123:1700000900" with key [0x11; 32]
        assert_eq!(
            sig,
            "3d1bfc599fd4dc0cb332066e5879c00f3d2024e4de7049cbe1b823fa8c72cd17"
        );
    }

    #[test]
    fn proxy_base_url_uses_configured_bind_addr() {
        let proxy = MediaProxy::new("127.0.0.1:18080".to_string(), String::new());
        assert_eq!(proxy.base_url(), "http://127.0.0.1:18080");
    }

    #[test]
    fn proxy_base_url_uses_explicit_url() {
        let proxy = MediaProxy::new(
            "0.0.0.0:18080".to_string(),
            "http://host.docker.internal:18080".to_string(),
        );
        assert_eq!(proxy.base_url(), "http://host.docker.internal:18080");
    }
}
