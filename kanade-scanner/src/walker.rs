use std::{ffi::OsStr, fs, path::Path};

use tracing::warn;
use walkdir::WalkDir;

pub struct AudioFileEntry {
    pub file_path: String,
    pub dir_path: String,
    pub mtime: i64,
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "@eaDir",
    "#recycle",
    ".stversions",
    ".Trash-0",
    ".Trash-1000",
    "$RECYCLE.BIN",
    "System Volume Information",
];

const SKIP_PREFIXES: &[&str] = &[".", "_"];

const COVER_FILENAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "artwork.jpg",
    "artwork.jpeg",
    "artwork.png",
    "album.jpg",
    "album.jpeg",
    "album.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
];

fn should_skip_dir(dir_name: &OsStr) -> bool {
    let name = dir_name.to_string_lossy();
    SKIP_DIRS.iter().any(|skip| name == *skip)
        || SKIP_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix) && name != ".")
}

pub fn walk_audio_files(root: &Path, extensions: &[&str]) -> Vec<AudioFileEntry> {
    let ext_set: Vec<&str> = extensions.iter().copied().collect();
    let mut entries = Vec::new();

    let root_depth = root.components().count();

    for result in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| {
            if entry.path() == root {
                return true;
            }
            let depth = entry.path().components().count() - root_depth;
            if entry.file_type().is_dir() && depth > 0 {
                return !should_skip_dir(entry.file_name());
            }
            true
        })
    {
        match result {
            Ok(entry) => {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase());
                let matches = ext
                    .as_deref()
                    .is_some_and(|e| ext_set.iter().any(|valid| *valid == e));
                if !matches {
                    continue;
                }

                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                entries.push(AudioFileEntry {
                    file_path: path.to_string_lossy().into_owned(),
                    dir_path: path
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    mtime,
                });
            }
            Err(e) => {
                warn!("walk: {}", e);
            }
        }
    }

    entries
}

/// Search a directory for common cover art filenames.
/// Returns the first match found, preferring exact case then case-insensitive.
pub fn find_cover_art(dir: &Path) -> Option<String> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    // Exact case match first
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if COVER_FILENAMES.iter().any(|c| *c == name_str.as_ref()) {
            return Some(entry.path().to_string_lossy().into_owned());
        }
    }

    // Case-insensitive fallback
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_lower = name.to_string_lossy().to_lowercase();
        if COVER_FILENAMES
            .iter()
            .any(|c| c.to_lowercase() == name_lower)
        {
            return Some(entry.path().to_string_lossy().into_owned());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn skips_hidden_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        fs::create_dir_all(dir.path().join("normal")).unwrap();
        fs::write(dir.path().join("normal/test.flac"), "").unwrap();
        fs::write(dir.path().join(".hidden/test.flac"), "").unwrap();

        let entries = walk_audio_files(dir.path(), &["flac"]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].file_path.contains("normal"));
    }

    #[test]
    fn skips_special_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::create_dir_all(dir.path().join("@eaDir")).unwrap();
        fs::create_dir_all(dir.path().join("music")).unwrap();
        fs::write(dir.path().join("music/test.mp3"), "").unwrap();
        fs::write(dir.path().join("node_modules/test.flac"), "").unwrap();

        let entries = walk_audio_files(dir.path(), &["flac", "mp3"]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].file_path.contains("music"));
    }

    #[test]
    fn filters_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("song.flac"), "").unwrap();
        fs::write(dir.path().join("image.jpg"), "").unwrap();
        fs::write(dir.path().join("readme.txt"), "").unwrap();

        let entries = walk_audio_files(dir.path(), &["flac"]);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn case_insensitive_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("song.FLAC"), "").unwrap();
        fs::write(dir.path().join("song.Mp3"), "").unwrap();

        let entries = walk_audio_files(dir.path(), &["flac", "mp3"]);
        assert_eq!(entries.len(), 2);
    }
}
