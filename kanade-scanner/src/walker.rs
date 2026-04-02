use std::{ffi::OsStr, path::Path};

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
