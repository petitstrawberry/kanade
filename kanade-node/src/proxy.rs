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
use reqwest::StatusCode;
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration, Instant};
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
    generation: u64,
}

/// Long-lived local HTTP proxy that re-signs Kanade media URLs on demand.
///
/// Clone-cheap: the inner state is reference-counted.
#[derive(Clone)]
pub struct MediaProxy {
    state: Arc<RwLock<Option<ProxyState>>>,
    bind_addr: String,
    base_url: String,
    refresh_tx: mpsc::Sender<()>,
    probe_client: reqwest::Client,
}

impl MediaProxy {
    /// Create a new proxy.
    ///
    /// * `bind_addr` — the address the proxy listens on (e.g. `"127.0.0.1:18080"`).
    /// * `base_url`  — the URL that MPD should use to reach the proxy
    ///   (e.g. `"http://127.0.0.1:18080"`).  When empty the URL is derived
    ///   from `bind_addr` by prepending `"http://"`.
    pub fn new(bind_addr: String, base_url: String, refresh_tx: mpsc::Sender<()>) -> Self {
        let base_url = if base_url.is_empty() {
            format!("http://{bind_addr}")
        } else {
            base_url
        };
        Self {
            state: Arc::new(RwLock::new(None)),
            bind_addr,
            base_url,
            refresh_tx,
            probe_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .expect("failed to build media proxy HTTP client"),
        }
    }

    /// Update the proxy with the credentials from the latest server session.
    ///
    /// Pass `auth = None` when the server did not issue an auth key.
    pub async fn update(&self, kanade_base_url: String, auth: Option<(String, [u8; 32])>) {
        let mut guard = self.state.write().await;
        let generation = guard
            .as_ref()
            .map(|state| state.generation.saturating_add(1))
            .unwrap_or(1);
        *guard = Some(ProxyState {
            kanade_base_url,
            auth,
            generation,
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
                    let refresh_tx = self.refresh_tx.clone();
                    let probe_client = self.probe_client.clone();
                    tokio::spawn(handle_request(stream, state, refresh_tx, probe_client));
                }
                Err(e) => warn!("MediaProxy: accept error: {e}"),
            }
        }
    }
}

async fn handle_request(
    stream: tokio::net::TcpStream,
    state: Arc<RwLock<Option<ProxyState>>>,
    refresh_tx: mpsc::Sender<()>,
    probe_client: reqwest::Client,
) {
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

    let track_path = format!("/media/tracks/{track_id}");
    let Some((mut redirect_url, mut generation, has_auth)) =
        build_redirect_target(&state, &track_path).await
    else {
        let _ = write_response(&mut writer, 503, None).await;
        return;
    };

    if has_auth && is_forbidden(&probe_client, &redirect_url).await {
        warn!("MediaProxy: detected 403 for signed URL; requesting key refresh and retrying once");
        let _ = refresh_tx.try_send(());
        if wait_for_new_generation(&state, generation, Duration::from_secs(5)).await {
            if let Some((new_redirect_url, new_generation, _)) =
                build_redirect_target(&state, &track_path).await
            {
                redirect_url = new_redirect_url;
                generation = new_generation;
            }
        }

        if is_forbidden(&probe_client, &redirect_url).await {
            warn!(
                generation,
                "MediaProxy: refreshed key still rejected with 403; returning 503"
            );
            let _ = write_response(&mut writer, 503, None).await;
            return;
        }
    }

    debug!(%redirect_url, "MediaProxy: redirecting {path_only}");
    let _ = write_response(&mut writer, 302, Some(&redirect_url)).await;
}

async fn build_redirect_target(
    state: &Arc<RwLock<Option<ProxyState>>>,
    track_path: &str,
) -> Option<(String, u64, bool)> {
    let guard = state.read().await;
    let st = guard.as_ref()?;
    let redirect_url = match &st.auth {
        Some((key_id, key)) => {
            let now = current_timestamp()?;
            let exp = now.saturating_add(MEDIA_URL_TTL_SECS);
            let sig = sign(key, track_path, exp);
            format!(
                "{}{track_path}?kid={key_id}&exp={exp}&sig={}",
                st.kanade_base_url.trim_end_matches('/'),
                hex::encode(sig),
            )
        }
        None => format!("{}{track_path}", st.kanade_base_url.trim_end_matches('/')),
    };
    Some((redirect_url, st.generation, st.auth.is_some()))
}

async fn wait_for_new_generation(
    state: &Arc<RwLock<Option<ProxyState>>>,
    previous_generation: u64,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let changed = {
            let guard = state.read().await;
            guard
                .as_ref()
                .map(|st| st.generation > previous_generation)
                .unwrap_or(false)
        };
        if changed {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn is_forbidden(client: &reqwest::Client, url: &str) -> bool {
    match client.head(url).send().await {
        Ok(response) => response.status() == StatusCode::FORBIDDEN,
        Err(e) => {
            debug!(error = %e, %url, "MediaProxy: upstream probe failed");
            false
        }
    }
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::RwLock;

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
        let (refresh_tx, _refresh_rx) = mpsc::channel::<()>(1);
        let proxy = MediaProxy::new("127.0.0.1:18080".to_string(), String::new(), refresh_tx);
        assert_eq!(proxy.base_url(), "http://127.0.0.1:18080");
    }

    #[test]
    fn proxy_base_url_uses_explicit_url() {
        let (refresh_tx, _refresh_rx) = mpsc::channel::<()>(1);
        let proxy = MediaProxy::new(
            "0.0.0.0:18080".to_string(),
            "http://host.docker.internal:18080".to_string(),
            refresh_tx,
        );
        assert_eq!(proxy.base_url(), "http://host.docker.internal:18080");
    }

    #[tokio::test]
    async fn refreshes_key_on_403_and_retries_redirect() {
        let valid_kid = Arc::new(RwLock::new("kid-new".to_string()));
        let valid_kid_for_server = Arc::clone(&valid_kid);

        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = upstream_listener.accept().await {
                let valid_kid_for_conn = Arc::clone(&valid_kid_for_server);
                tokio::spawn(async move {
                    let mut req = vec![0u8; 4096];
                    let n = stream.read(&mut req).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    let req_text = String::from_utf8_lossy(&req[..n]);
                    let request_line = req_text.lines().next().unwrap_or_default();
                    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                    let kid = path
                        .split("kid=")
                        .nth(1)
                        .and_then(|s| s.split('&').next())
                        .unwrap_or_default();
                    let expected = valid_kid_for_conn.read().await.clone();
                    let status = if kid == expected { 200 } else { 403 };
                    let reason = if status == 200 { "OK" } else { "Forbidden" };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        let bind_probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = bind_probe.local_addr().unwrap();
        drop(bind_probe);

        let (refresh_tx, mut refresh_rx) = mpsc::channel::<()>(2);
        let proxy = MediaProxy::new(proxy_addr.to_string(), String::new(), refresh_tx);
        let upstream_base = format!("http://{upstream_addr}");
        proxy
            .update(
                upstream_base.clone(),
                Some(("kid-old".to_string(), [0x11u8; 32])),
            )
            .await;

        let proxy_for_refresh = proxy.clone();
        tokio::spawn(async move {
            if refresh_rx.recv().await.is_some() {
                proxy_for_refresh
                    .update(upstream_base, Some(("kid-new".to_string(), [0x22u8; 32])))
                    .await;
            }
        });

        let proxy_task = tokio::spawn(proxy.clone().run());
        sleep(Duration::from_millis(50)).await;

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET /media/tracks/test HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();

        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();

        assert!(response.starts_with("HTTP/1.1 302"));
        assert!(response.contains("Location: "));
        assert!(response.contains("kid=kid-new"));
        assert!(!response.contains("kid=kid-old"));

        proxy_task.abort();
    }
}
