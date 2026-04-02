use serde::{Deserialize, Serialize};

/// A single audio track loaded from its file tags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    /// Canonical identifier: SHA-256 of the absolute file path (hex string).
    pub id: String,
    /// Absolute path to the audio file.
    pub file_path: String,
    pub title: Option<String>,
    pub track_number: Option<u32>,
    /// Duration in seconds.
    pub duration_secs: Option<f64>,
    /// Audio format / codec (e.g. "FLAC", "MP3", "AAC").
    pub format: Option<String>,
    pub sample_rate: Option<u32>,
    /// Artist tag exactly as stored in the file.
    pub artist: Option<String>,
    /// Album title tag exactly as stored in the file.
    pub album_title: Option<String>,
    /// Composer tag exactly as stored in the file.
    pub composer: Option<String>,
}

/// An album derived deterministically from a directory path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Album {
    /// SHA-256 of the album directory path (hex string).
    pub id: String,
    /// Absolute path to the directory that contains the tracks.
    pub dir_path: String,
    /// Album title (taken from the first track's tag, or directory name).
    pub title: Option<String>,
}

/// An artist derived from an exact tag string match.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artist {
    /// SHA-256 of the exact artist name string (hex string).
    pub id: String,
    pub name: String,
}
