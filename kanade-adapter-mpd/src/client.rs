use std::time::Duration;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use tracing::instrument;

use kanade_core::error::CoreError;

/// Thin async MPD TCP client.
///
/// Opens a new TCP connection per command batch.  This is intentionally simple
/// and avoids connection-pool complexity while remaining correct for a
/// single-user, single-zone audio player.
pub struct MpdClient {
    host: String,
    port: u16,
}

impl MpdClient {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port }
    }

    /// Send one or more MPD commands (newline-terminated) and return the
    /// response lines up to `OK\n` or `ACK …`.
    #[instrument(skip(self))]
    pub async fn send(&self, commands: &str) -> Result<Vec<String>, CoreError> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            TcpStream::connect(&addr),
        )
        .await
        .map_err(|_| CoreError::Output(format!("timeout connecting to MPD at {addr}")))?
        .map_err(|e| CoreError::Output(format!("cannot connect to MPD at {addr}: {e}")))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Consume the MPD banner: "OK MPD <version>\n"
        let mut banner = String::new();
        reader.read_line(&mut banner).await.map_err(|e| {
            CoreError::Output(format!("MPD banner read error: {e}"))
        })?;

        // Send the command(s)
        writer
            .write_all(commands.as_bytes())
            .await
            .map_err(|e| CoreError::Output(format!("MPD write error: {e}")))?;

        // Collect response lines
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.map_err(|e| {
                CoreError::Output(format!("MPD read error: {e}"))
            })?;
            let trimmed = line.trim_end().to_string();
            if trimmed == "OK" {
                break;
            }
            if trimmed.starts_with("ACK") {
                return Err(CoreError::Output(format!("MPD error: {trimmed}")));
            }
            lines.push(trimmed);
        }
        Ok(lines)
    }
}
