use std::{net::SocketAddr, path::PathBuf};

use kanade_db::Database;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, warn};

pub struct MediaServer {
    db_path: PathBuf,
    addr: SocketAddr,
}

impl MediaServer {
    pub fn new(db_path: PathBuf, addr: SocketAddr) -> Self {
        Self { db_path, addr }
    }

    pub async fn run(self) {
        let listener = TcpListener::bind(self.addr)
            .await
            .expect("MediaServer: failed to bind");
        info!("Media HTTP server listening on {}", self.addr);

        loop {
            match listener.accept().await {
                Ok((stream, _peer)) => {
                    let db_path = self.db_path.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, db_path).await {
                            if is_expected_disconnect(&e) {
                                debug!("MediaServer client disconnected: {e}");
                            } else {
                                warn!("MediaServer connection error: {e}");
                            }
                        }
                    });
                }
                Err(e) => error!("MediaServer: accept error: {e}"),
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, db_path: PathBuf) -> Result<(), String> {
    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| format!("read request line: {e}"))?;
    if request_line.trim().is_empty() {
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        write_simple_response(&mut writer, 400, "Bad Request", &[]).await?;
        return Ok(());
    }

    let method = parts[0];
    let path = parts[1];
    let mut range_header: Option<String> = None;

    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .await
            .map_err(|e| format!("read header: {e}"))?;
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("range:") {
            range_header = Some(trimmed[6..].trim().to_string());
        }
    }

    if method != "GET" && method != "HEAD" {
        write_simple_response(&mut writer, 405, "Method Not Allowed", &[]).await?;
        return Ok(());
    }

    let Some(track_id) = path.strip_prefix("/media/tracks/") else {
        write_simple_response(&mut writer, 404, "Not Found", &[]).await?;
        return Ok(());
    };

    let track_id = track_id.split('?').next().unwrap_or(track_id).to_string();

    let track = tokio::task::spawn_blocking(move || {
        let db = Database::open(&db_path).ok()?;
        db.get_track_by_id(&track_id).ok()?
    })
    .await
    .map_err(|e| format!("db join: {e}"))?
    .ok_or_else(|| "track not found".to_string())?;

    let mut file = File::open(&track.file_path)
        .await
        .map_err(|e| format!("open file: {e}"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| format!("metadata: {e}"))?;
    let total_len = metadata.len();

    let (start, end, status_code, status_text) = match parse_range(range_header.as_deref(), total_len)
    {
        Ok(Some((start, end))) => (start, end, 206, "Partial Content"),
        Ok(None) => (0, total_len.saturating_sub(1), 200, "OK"),
        Err(()) => {
            let headers = vec![
                ("Content-Range".to_string(), format!("bytes */{total_len}")),
                ("Content-Length".to_string(), "0".to_string()),
            ];
            write_response_headers(&mut writer, 416, "Range Not Satisfiable", &headers).await?;
            return Ok(());
        }
    };

    let content_length = if total_len == 0 { 0 } else { end - start + 1 };
    let mut headers = vec![
        ("Content-Type".to_string(), content_type_for_path(&track.file_path).to_string()),
        ("Accept-Ranges".to_string(), "bytes".to_string()),
        ("Content-Length".to_string(), content_length.to_string()),
    ];
    if status_code == 206 {
        headers.push((
            "Content-Range".to_string(),
            format!("bytes {}-{}/{}", start, end, total_len),
        ));
    }

    write_response_headers(&mut writer, status_code, status_text, &headers).await?;

    if method == "HEAD" || content_length == 0 {
        return Ok(());
    }

    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|e| format!("seek: {e}"))?;

    let mut remaining = content_length;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let n = file
            .read(&mut buf[..to_read])
            .await
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| format!("write body: {e}"))?;
        remaining -= n as u64;
    }

    Ok(())
}

fn parse_range(header: Option<&str>, total_len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(header) = header else {
        return Ok(None);
    };
    if total_len == 0 {
        return Err(());
    }
    let value = header.trim();
    let Some(spec) = value.strip_prefix("bytes=") else {
        return Err(());
    };
    let Some((start_s, end_s)) = spec.split_once('-') else {
        return Err(());
    };

    if start_s.is_empty() {
        let suffix = end_s.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let start = total_len.saturating_sub(suffix.min(total_len));
        return Ok(Some((start, total_len - 1)));
    }

    let start = start_s.parse::<u64>().map_err(|_| ())?;
    if start >= total_len {
        return Err(());
    }
    let end = if end_s.is_empty() {
        total_len - 1
    } else {
        end_s.parse::<u64>().map_err(|_| ())?.min(total_len - 1)
    };
    if start > end {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn content_type_for_path(path: &str) -> &'static str {
    match PathBuf::from(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("flac") => "audio/flac",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") | Some("mp4") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("ogg") => "audio/ogg",
        Some("opus") => "audio/ogg",
        Some("aiff") | Some("aif") => "audio/aiff",
        Some("dsf") => "audio/x-dsf",
        _ => "application/octet-stream",
    }
}

async fn write_simple_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: u16,
    text: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = vec![
        ("Content-Length".to_string(), body.len().to_string()),
        ("Content-Type".to_string(), "text/plain; charset=utf-8".to_string()),
    ];
    write_response_headers(writer, status, text, &headers).await?;
    if !body.is_empty() {
        writer
            .write_all(body)
            .await
            .map_err(|e| format!("write body: {e}"))?;
    }
    Ok(())
}

async fn write_response_headers(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: u16,
    text: &str,
    headers: &[(String, String)],
) -> Result<(), String> {
    let mut response = format!("HTTP/1.1 {status} {text}\r\n");
    for (k, v) in headers {
        response.push_str(k);
        response.push_str(": ");
        response.push_str(v);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    writer
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("write headers: {e}"))?;
    Ok(())
}

fn is_expected_disconnect(error: &str) -> bool {
    error.contains("Broken pipe") || error.contains("Connection reset")
}
