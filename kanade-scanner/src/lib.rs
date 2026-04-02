pub mod extractor;
pub mod walker;

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use kanade_db::Database;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "mp3", "m4a", "aac", "ogg", "opus", "wav", "aiff", "aif", "wma", "ape", "dsf",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub current_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub elapsed: Duration,
}

pub struct Scanner;

impl Scanner {
    pub fn scan_dir(
        db: &Database,
        root: &Path,
        progress_tx: &Sender<ScanProgress>,
    ) -> Result<ScanResult> {
        let start = Instant::now();
        let mut result = ScanResult {
            added: 0,
            updated: 0,
            removed: 0,
            elapsed: Duration::ZERO,
        };

        let entries = walker::walk_audio_files(root, AUDIO_EXTENSIONS);
        let total = entries.len();
        info!("scan: found {total} audio files in {}", root.display());

        let mut progress = ScanProgress {
            scanned: 0,
            added: 0,
            updated: 0,
            current_dir: None,
        };

        let mut last_dir = String::new();

        for entry in &entries {
            if entry.dir_path != last_dir {
                last_dir = entry.dir_path.clone();
                progress.current_dir = Some(entry.dir_path.clone());
                let _ = progress_tx.send(progress.clone());
            }

            let stored_mtime = db.get_track_mtime(&entry.file_path).unwrap_or(None);

            let needs_update = match stored_mtime {
                Some(stored) if stored == entry.mtime => false,
                _ => true,
            };

            if needs_update {
                match extractor::extract_track(&entry.file_path) {
                    Ok(track) => {
                        let is_new = stored_mtime.is_none();
                        db.upsert_track_with_mtime(&track, Some(entry.mtime))?;

                        if is_new {
                            progress.added += 1;
                            result.added += 1;
                        } else {
                            progress.updated += 1;
                            result.updated += 1;
                        }
                    }
                    Err(e) => {
                        warn!("scan: skipping {}: {e}", entry.file_path);
                    }
                }
            }

            progress.scanned += 1;
        }

        let known_paths: Vec<String> = entries.iter().map(|e| e.file_path.clone()).collect();
        result.removed = db.purge_missing(&known_paths)? as usize;

        let mut seen_dirs = std::collections::HashSet::new();
        for entry in &entries {
            if seen_dirs.insert(&entry.dir_path) {
                if let Some(art) = walker::find_cover_art(Path::new(&entry.dir_path)) {
                    let _ = db.update_album_artwork(&entry.dir_path, Some(&art));
                }
            }
        }

        let _ = progress_tx.send(ScanProgress {
            scanned: total,
            added: result.added,
            updated: result.updated,
            current_dir: Some("done".to_string()),
        });

        result.elapsed = start.elapsed();
        info!(
            "scan complete: +{} ~{} -{} in {:.1}s",
            result.added,
            result.updated,
            result.removed,
            result.elapsed.as_secs_f64()
        );

        Ok(result)
    }

    pub fn scan_once(db: &Database, root: &Path) -> Result<ScanResult> {
        let (tx, _rx) = mpsc::channel();
        Self::scan_dir(db, root, &tx)
    }
}

/// Blocking background scan loop. Runs in a dedicated thread via `spawn_blocking`.
///
/// 1. Performs an immediate full scan on startup.
/// 2. Then sleeps for `interval` and runs incremental scans forever.
/// 3. Logs results via `tracing`.
///
/// The thread blocks on `shutdown_rx` — when the channel is dropped (sender
/// side dropped), the loop exits cleanly.
pub fn background_scan_loop(
    db_path: PathBuf,
    music_dir: PathBuf,
    interval: Duration,
    shutdown_rx: mpsc::Receiver<()>,
) {
    let db = match Database::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            error!("scan: cannot open database at {}: {e}", db_path.display());
            return;
        }
    };

    info!(
        "scan: background loop started, dir={}, interval={:?}",
        music_dir.display(),
        interval
    );

    // Initial scan on startup.
    if let Err(e) = Scanner::scan_once(&db, &music_dir) {
        error!("scan: initial scan failed: {e}");
    }

    // Periodic incremental scans. The incremental logic is inside scan_dir
    // (mtime comparison — unchanged files are skipped).
    loop {
        match shutdown_rx.recv_timeout(interval) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("scan: background loop shutting down");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(e) = Scanner::scan_once(&db, &music_dir) {
                    error!("scan: periodic scan failed: {e}");
                }
            }
        }
    }
}

/// Convenience: spawn the background scan loop on tokio's blocking thread pool.
///
/// Returns a `mpsc::Sender` — dropping it signals the scan loop to stop.
pub fn spawn_background_scan(
    db_path: PathBuf,
    music_dir: PathBuf,
    interval: Duration,
) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        background_scan_loop(db_path, music_dir, interval, rx);
    });
    tx
}
