use std::{net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, instrument, warn};

use kanade_core::controller::Core;

use crate::{
    broadcaster::OpenHomeBroadcaster,
    soap::{fault_response, ok_response, parse_action, SoapAction},
};

const TRANSPORT_SERVICE: &str = "urn:av-openhome-org:service:Transport:1";

pub struct OpenHomeServer {
    core: Arc<Core>,
    broadcaster: Arc<OpenHomeBroadcaster>,
    addr: SocketAddr,
}

impl OpenHomeServer {
    pub fn new(
        core: Arc<Core>,
        broadcaster: Arc<OpenHomeBroadcaster>,
        addr: SocketAddr,
    ) -> Self {
        Self { core, broadcaster, addr }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("OpenHomeServer: failed to bind");
        info!("OpenHome HTTP server listening on {}", self.addr);

        let core = self.core;
        let broadcaster = self.broadcaster;

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let ctrl = Arc::clone(&core);
                    let bc = Arc::clone(&broadcaster);
                    tokio::spawn(handle_request(stream, peer, ctrl, bc));
                }
                Err(e) => {
                    error!("OpenHomeServer: accept error: {e}");
                }
            }
        }
    }
}

#[instrument(skip(stream, core, _broadcaster))]
async fn handle_request(
    stream: TcpStream,
    peer: SocketAddr,
    core: Arc<Core>,
    _broadcaster: Arc<OpenHomeBroadcaster>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0];

    let mut soap_action = String::new();
    let mut content_length: usize = 0;
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line).await.is_err() {
            return;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_lowercase();
        if lower.starts_with("soapaction:") {
            soap_action = trimmed[11..].trim().to_string();
        } else if lower.starts_with("content-length:") {
            content_length = trimmed[15..].trim().parse().unwrap_or(0);
        }
    }

    if method != "POST" || soap_action.is_empty() {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = writer.write_all(resp.as_bytes()).await;
        return;
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        use tokio::io::AsyncReadExt;
        if reader.read_exact(&mut body).await.is_err() {
            return;
        }
    }
    let body_str = String::from_utf8_lossy(&body);

    debug!("OpenHome SOAP action from {peer}: {soap_action}");

    let response_body = match parse_action(&body_str, &soap_action) {
        Ok(action) => {
            let action_name = action_name_str(&action);
            let result = dispatch(action, &core).await;
            match result {
                Ok(()) => ok_response(action_name, TRANSPORT_SERVICE),
                Err(e) => fault_response(501, &e.to_string()),
            }
        }
        Err(e) => fault_response(402, &e.to_string()),
    };

    let http_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nContent-Length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = writer.write_all(http_response.as_bytes()).await;
}

fn action_name_str(action: &SoapAction) -> &'static str {
    match action {
        SoapAction::Play => "Play",
        SoapAction::Pause => "Pause",
        SoapAction::Stop => "Stop",
        SoapAction::Next => "Next",
        SoapAction::Previous => "Previous",
        SoapAction::SeekSecondAbsolute { .. } => "SeekSecondAbsolute",
        SoapAction::SetVolume { .. } => "SetVolume",
        SoapAction::Unknown(_) => "Unknown",
    }
}

async fn dispatch(
    action: SoapAction,
    core: &Core,
) -> Result<(), kanade_core::error::CoreError> {
    match action {
        SoapAction::Play => core.play_zone("default").await,
        SoapAction::Pause => core.pause_zone("default").await,
        SoapAction::Stop => core.stop_zone("default").await,
        SoapAction::Next => core.next_zone("default").await,
        SoapAction::Previous => core.previous_zone("default").await,
        SoapAction::SeekSecondAbsolute { seconds } => {
            core.seek_zone("default", seconds as f64).await
        }
        SoapAction::SetVolume { volume } => {
            core.set_zone_volume("default", volume).await
        }
        SoapAction::Unknown(name) => {
            warn!("OpenHome: unhandled action: {name}");
            Ok(())
        }
    }
}
