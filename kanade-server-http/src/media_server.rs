use std::{net::SocketAddr, path::PathBuf};

use kanade_db::Database;
use lofty::{probe::Probe, prelude::*};
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

enum ArtResult {
    FilePath(String),
    Embedded(String, Vec<u8>),
}

fn extract_embedded_picture(file_path: &str) -> Option<lofty::picture::Picture> {
    let path_lower = file_path.to_lowercase();
    if path_lower.ends_with(".dsf") {
        return extract_dsf_picture(file_path);
    }

    let tagged_file = Probe::open(file_path).ok()?.read().ok()?;
    let tag = match tagged_file.primary_tag() {
        Some(t) => t,
        None => tagged_file.first_tag()?,
    };
    tag.pictures()
        .iter()
        .find(|p| matches!(p.pic_type(), lofty::picture::PictureType::CoverFront))
        .cloned()
        .or_else(|| tag.pictures().first().cloned())
}

fn extract_dsf_picture(file_path: &str) -> Option<lofty::picture::Picture> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = std::fs::File::open(file_path).ok()?;

    // DSF format: DSD chunk (12 header + 28 body) → fmt → data → ... → ID3v2
    // DSD body bytes 8-15 (file offset 20-27): metadata offset pointer to ID3v2 tag
    f.seek(SeekFrom::Start(20)).ok()?;
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).ok()?;
    let id3_offset = u64::from_le_bytes(buf);

    f.seek(SeekFrom::Start(id3_offset)).ok()?;
    let mut header = [0u8; 3];
    f.read_exact(&mut header).ok()?;
    if &header != b"ID3" {
        return None;
    }
    f.seek(SeekFrom::Start(id3_offset)).ok()?;

    let id3_data = {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        buf
    };

    parse_id3v2_apic(&id3_data)
}

fn parse_id3v2_apic(data: &[u8]) -> Option<lofty::picture::Picture> {
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return None;
    }

    let version = data[3];
    let flags = data[5];
    let size = if version == 4 {
        ((data[6] as u64) << 21) | ((data[7] as u64) << 14) | ((data[8] as u64) << 7) | (data[9] as u64)
    } else {
        ((data[6] as u64) << 24) | ((data[7] as u64) << 16) | ((data[8] as u64) << 8) | (data[9] as u64)
    } as usize;

    let has_footer = version == 4 && (flags & 0x10) != 0;
    let total_size = 10 + size + if has_footer { 10 } else { 0 };
    if total_size > data.len() {
        return None;
    }

    let frames_data = &data[10..10 + size];
    find_apic_frame(frames_data, version)
}

fn find_apic_frame(data: &[u8], version: u8) -> Option<lofty::picture::Picture> {
    let mut pos = 0;
    let synchsafe = version == 4;

    while pos + 10 <= data.len() {
        let frame_id = &data[pos..pos + 4];
        if frame_id == [0; 4] {
            break;
        }

        let frame_size = if synchsafe {
            ((data[pos + 4] as usize) << 21)
                | ((data[pos + 5] as usize) << 14)
                | ((data[pos + 6] as usize) << 7)
                | (data[pos + 7] as usize)
        } else {
            ((data[pos + 4] as usize) << 24)
                | ((data[pos + 5] as usize) << 16)
                | ((data[pos + 6] as usize) << 8)
                | (data[pos + 7] as usize)
        };

        let frame_flags: [u8; 2] = data[pos + 8..pos + 10].try_into().ok()?;
        let frame_data = data.get(pos + 10..pos + 10 + frame_size)?;
        let frame_id_str = std::str::from_utf8(frame_id).ok()?;

        if frame_id_str == "APIC" {
            return Some(parse_apic_data(frame_data, version)?);
        }

        let has_header = version == 4 && (frame_flags[1] & 0x01) != 0;
        let skip = if has_header { 4 } else { 0 };
        pos += 10 + frame_size + skip;
    }

    None
}

fn parse_apic_data(data: &[u8], version: u8) -> Option<lofty::picture::Picture> {
    let mut pos = 0;

    let _encoding = data.get(pos)?;
    pos += 1;

    let mime_end = memchr::memchr(0, &data[pos..])?;
    let mime = std::str::from_utf8(&data[pos..pos + mime_end]).ok()?.to_string();
    pos += mime_end + 1;

    let pic_type = lofty::picture::PictureType::from_u8(data[pos]);
    pos += 1;

    let desc_end = memchr::memchr(0, &data[pos..])?;
    pos += desc_end + 1;

    if version == 1 || version == 2 {
        if pos < data.len() {
            pos += 1;
        }
    }

    let pic_data = data.get(pos..)?;
    if pic_data.is_empty() {
        return None;
    }

    let mime_type = Some(lofty::picture::MimeType::from_str(&mime));

    Some(lofty::picture::Picture::new_unchecked(
        pic_type,
        mime_type,
        None,
        pic_data.to_vec(),
    ))
}

async fn serve_bytes(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    method: &str,
    content_type: &str,
    data: &[u8],
) -> Result<(), String> {
    let len = data.len() as u64;
    let headers = vec![
        ("Content-Type".to_string(), content_type.to_string()),
        ("Content-Length".to_string(), len.to_string()),
        ("Cache-Control".to_string(), "public, max-age=86400".to_string()),
    ];
    write_response_headers(writer, 200, "OK", &headers).await?;
    if method != "HEAD" {
        writer
            .write_all(data)
            .await
            .map_err(|e| format!("write body: {e}"))?;
    }
    Ok(())
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

    if method == "OPTIONS" {
        write_simple_response(&mut writer, 204, "No Content", &[]).await?;
        return Ok(());
    }

    if method != "GET" && method != "HEAD" {
        write_simple_response(&mut writer, 405, "Method Not Allowed", &[]).await?;
        return Ok(());
    }

    let Some(track_id) = path.strip_prefix("/media/tracks/") else {
        let Some(album_id) = path.strip_prefix("/media/art/") else {
            let Some(raw_path) = path.strip_prefix("/media/file/") else {
                write_simple_response(&mut writer, 404, "Not Found", &[]).await?;
                return Ok(());
            };

            let decoded = percent_decode(raw_path);
            if decoded.is_empty() || decoded.contains("..") {
                write_simple_response(&mut writer, 400, "Bad Request", &[]).await?;
                return Ok(());
            }

            let content_type = content_type_for_path(&decoded);
            match File::open(&decoded).await {
                Ok(mut file) => {
                    if let Err(e) = serve_file_with_range(&mut writer, method, &mut file, content_type, range_header.as_deref()).await {
                        let status = if e.contains("open file:") { 404 } else { 500 };
                        let text = if status == 404 { "Not Found" } else { "Internal Server Error" };
                        write_simple_response(&mut writer, status, text, &[]).await?;
                        return Err(e);
                    }
                    return Ok(());
                }
                Err(_) => {
                    write_simple_response(&mut writer, 404, "Not Found", &[]).await?;
                    return Ok(());
                }
            }
        };

        let album_id = album_id.split('?').next().unwrap_or(album_id).to_string();

        let result = tokio::task::spawn_blocking({
            let db_path = db_path.clone();
            let album_id = album_id.clone();
            move || -> Result<ArtResult, String> {
                let db = Database::open(&db_path).map_err(|e| e.to_string())?;
                if let Some(art_path) = db.get_album_artwork_path(&album_id).map_err(|e| e.to_string())? {
                    return Ok(ArtResult::FilePath(art_path));
                }
                let tracks = db.get_tracks_by_album_id(&album_id).map_err(|e| e.to_string())?;
                if let Some(track) = tracks.into_iter().next() {
                    if let Some(picture) = extract_embedded_picture(&track.file_path) {
                        let mime = picture
                            .mime_type()
                            .map(|m| m.as_str())
                            .unwrap_or("image/jpeg")
                            .to_string();
                        let data = picture.data().to_vec();
                        return Ok(ArtResult::Embedded(mime, data));
                    }
                }
                Err("no artwork found".to_string())
            }
        })
        .await;

        match result {
            Ok(Ok(ArtResult::FilePath(art_path))) => {
                let art_type = content_type_for_path(&art_path);
                if let Err(e) = serve_static_file(&mut writer, method, &art_path, art_type).await {
                    let status = if e.contains("open file:") { 404 } else { 500 };
                    let text = if status == 404 { "Not Found" } else { "Internal Server Error" };
                    write_simple_response(&mut writer, status, text, &[]).await?;
                    return Err(e);
                }
            }
            Ok(Ok(ArtResult::Embedded(mime, data))) => {
                serve_bytes(&mut writer, method, &mime, &data).await?;
            }
            Ok(Err(e)) => {
                write_simple_response(&mut writer, 404, "Artwork Not Found", &[]).await?;
                debug!("artwork not found for album {album_id}: {e}");
            }
            Err(e) => {
                write_simple_response(&mut writer, 500, "Internal Server Error", b"db error").await?;
                return Err(format!("db join: {e}"));
            }
        }
        return Ok(());
    };

    let track_id = track_id.split('?').next().unwrap_or(track_id).to_string();

    let result = tokio::task::spawn_blocking(move || {
        let db = Database::open(&db_path).ok()?;
        db.get_track_by_id(&track_id).ok()?
    })
    .await;

    let track = match result {
        Ok(Some(t)) => t,
        Ok(None) => {
            write_simple_response(&mut writer, 404, "Track Not Found", &[]).await?;
            return Ok(());
        }
        Err(e) => {
            write_simple_response(&mut writer, 500, "Internal Server Error", b"db error").await?;
            return Err(format!("db join: {e}"));
        }
    };

    let mut file = match File::open(&track.file_path).await {
        Ok(f) => f,
        Err(e) => {
            write_simple_response(&mut writer, 404, "File Not Found", &[]).await?;
            return Err(format!("open file: {e}"));
        }
    };

    let content_type = content_type_for_path(&track.file_path);
    serve_file_with_range(&mut writer, method, &mut file, content_type, range_header.as_deref()).await?;
    Ok(())
}

async fn serve_static_file(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    method: &str,
    path: &str,
    content_type: &str,
) -> Result<(), String> {
    let mut file = File::open(path)
        .await
        .map_err(|e| format!("open file: {e}"))?;
    serve_file_with_range(writer, method, &mut file, content_type, None).await
}

async fn serve_file_with_range(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    method: &str,
    file: &mut File,
    content_type: &str,
    range_header: Option<&str>,
) -> Result<(), String> {
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
            write_response_headers(writer, 416, "Range Not Satisfiable", &headers).await?;
            return Ok(());
        }
    };

    let content_length = if total_len == 0 { 0 } else { end - start + 1 };
    let mut headers = vec![
        ("Content-Type".to_string(), content_type.to_string()),
        ("Accept-Ranges".to_string(), "bytes".to_string()),
        ("Content-Length".to_string(), content_length.to_string()),
    ];
    if status_code == 206 {
        headers.push((
            "Content-Range".to_string(),
            format!("bytes {}-{}/{}", start, end, total_len),
        ));
    }

    write_response_headers(writer, status_code, status_text, &headers).await?;

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
    response.push_str("Access-Control-Allow-Origin: *\r\n");
    response.push_str("Access-Control-Allow-Headers: Range\r\n");
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

fn percent_decode(input: &str) -> String {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_default()
}
