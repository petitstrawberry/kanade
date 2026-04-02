use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use tracing::instrument;

use kanade_core::model::{Album, Artist, Track};

use crate::{hash::id_of, schema};

/// High-level database handle.
///
/// All operations are synchronous and use a single `Connection` that can be
/// wrapped in a `tokio::task::spawn_blocking` call when used from async code.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the SQLite database at `path`, applying the schema.
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        schema::apply(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply(&conn)?;
        Ok(Self { conn })
    }

    // ------------------------------------------------------------------
    // Track operations
    // ------------------------------------------------------------------

    /// Insert or replace a track record.
    ///
    /// Automatically upserts the parent album row so the foreign-key
    /// constraint (`tracks.album_id → albums.id`) is always satisfied.
    #[instrument(skip(self, track))]
    pub fn upsert_track(&self, track: &Track) -> anyhow::Result<()> {
        // Derive album from the directory that contains the file.
        let album_id = Path::new(&track.file_path).parent().map(|dir| {
            let dir_str = dir.to_string_lossy().into_owned();
            let aid = id_of(&dir_str);
            // Ensure the album row exists before the FK reference below.
            let album = kanade_core::model::Album {
                id: aid.clone(),
                dir_path: dir_str,
                title: track.album_title.clone(),
            };
            // Ignore error — if the album row already exists we just keep it.
            let _ = self.upsert_album(&album);
            aid
        });

        self.conn.execute(
            r#"INSERT INTO tracks
               (file_path, id, album_id, title, track_number, duration_secs,
                format, sample_rate, artist, album_title, composer)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(file_path) DO UPDATE SET
                   id            = excluded.id,
                   album_id      = excluded.album_id,
                   title         = excluded.title,
                   track_number  = excluded.track_number,
                   duration_secs = excluded.duration_secs,
                   format        = excluded.format,
                   sample_rate   = excluded.sample_rate,
                   artist        = excluded.artist,
                   album_title   = excluded.album_title,
                   composer      = excluded.composer"#,
            params![
                track.file_path,
                track.id,
                album_id,
                track.title,
                track.track_number,
                track.duration_secs,
                track.format,
                track.sample_rate,
                track.artist,
                track.album_title,
                track.composer,
            ],
        )?;
        Ok(())
    }

    /// Fetch a single track by its file path, or `None` if not found.
    pub fn get_track_by_path(&self, file_path: &str) -> anyhow::Result<Option<Track>> {
        let result = self
            .conn
            .query_row(
                r#"SELECT file_path, id, title, track_number, duration_secs,
                          format, sample_rate, artist, album_title, composer
                   FROM tracks WHERE file_path = ?1"#,
                params![file_path],
                row_to_track,
            )
            .optional()?;
        Ok(result)
    }

    /// Fetch all tracks belonging to a given album directory.
    pub fn get_tracks_by_album_id(&self, album_id: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT file_path, id, title, track_number, duration_secs,
                      format, sample_rate, artist, album_title, composer
               FROM tracks WHERE album_id = ?1
               ORDER BY track_number, title"#,
        )?;
        let rows = stmt.query_map(params![album_id], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete a track by file path (e.g. when the file is removed from disk).
    pub fn delete_track(&self, file_path: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM tracks WHERE file_path = ?1", params![file_path])?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Album operations
    // ------------------------------------------------------------------

    /// Insert or replace an album record.
    pub fn upsert_album(&self, album: &Album) -> anyhow::Result<()> {
        self.conn.execute(
            r#"INSERT INTO albums (id, dir_path, title)
               VALUES (?1, ?2, ?3)
               ON CONFLICT(id) DO UPDATE SET
                   dir_path = excluded.dir_path,
                   title    = excluded.title"#,
            params![album.id, album.dir_path, album.title],
        )?;
        Ok(())
    }

    /// Fetch an album by its directory-hash ID.
    pub fn get_album_by_id(&self, album_id: &str) -> anyhow::Result<Option<Album>> {
        let result = self
            .conn
            .query_row(
                "SELECT id, dir_path, title FROM albums WHERE id = ?1",
                params![album_id],
                |row| {
                    Ok(Album {
                        id: row.get(0)?,
                        dir_path: row.get(1)?,
                        title: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    // ------------------------------------------------------------------
    // Artist operations
    // ------------------------------------------------------------------

    /// Insert or replace an artist record.
    pub fn upsert_artist(&self, artist: &Artist) -> anyhow::Result<()> {
        self.conn.execute(
            r#"INSERT INTO artists (id, name)
               VALUES (?1, ?2)
               ON CONFLICT(id) DO UPDATE SET name = excluded.name"#,
            params![artist.id, artist.name],
        )?;
        Ok(())
    }

    /// Fetch an artist by its name-hash ID.
    pub fn get_artist_by_id(&self, artist_id: &str) -> anyhow::Result<Option<Artist>> {
        let result = self
            .conn
            .query_row(
                "SELECT id, name FROM artists WHERE id = ?1",
                params![artist_id],
                |row| Ok(Artist { id: row.get(0)?, name: row.get(1)? }),
            )
            .optional()?;
        Ok(result)
    }

    // ------------------------------------------------------------------
    // Full-text search
    // ------------------------------------------------------------------

    /// Search tracks using FTS5.  Returns matching track IDs (SHA-256 hex).
    ///
    /// `query` accepts standard FTS5 query syntax, e.g. `"blue moon"` or
    /// `artist:Miles`.
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id FROM tracks_fts WHERE tracks_fts MATCH ?1 ORDER BY rank",
        )?;
        let rows = stmt.query_map(params![query], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Helper — map a rusqlite Row to a Track
// ---------------------------------------------------------------------------

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        file_path: row.get(0)?,
        id: row.get(1)?,
        title: row.get(2)?,
        track_number: row.get(3)?,
        duration_secs: row.get(4)?,
        format: row.get(5)?,
        sample_rate: row.get(6)?,
        artist: row.get(7)?,
        album_title: row.get(8)?,
        composer: row.get(9)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::id_of;

    fn sample_track(file_path: &str) -> Track {
        Track {
            id: id_of(file_path),
            file_path: file_path.to_string(),
            title: Some("Test Track".to_string()),
            track_number: Some(1),
            duration_secs: Some(180.0),
            format: Some("FLAC".to_string()),
            sample_rate: Some(44100),
            artist: Some("Test Artist".to_string()),
            album_title: Some("Test Album".to_string()),
            composer: Some("Test Composer".to_string()),
        }
    }

    #[test]
    fn upsert_and_get_track() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();

        let fetched = db.get_track_by_path("/music/album/01.flac").unwrap();
        assert_eq!(fetched, Some(track));
    }

    #[test]
    fn upsert_track_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();
        db.upsert_track(&track).unwrap(); // must not error
        let fetched = db.get_track_by_path("/music/album/01.flac").unwrap();
        assert_eq!(fetched, Some(track));
    }

    #[test]
    fn delete_track() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();
        db.delete_track("/music/album/01.flac").unwrap();
        let fetched = db.get_track_by_path("/music/album/01.flac").unwrap();
        assert_eq!(fetched, None);
    }

    #[test]
    fn get_tracks_by_album_id() {
        let db = Database::open_in_memory().unwrap();
        let t1 = sample_track("/music/album/01.flac");
        let t2 = sample_track("/music/album/02.flac");
        let other = sample_track("/music/other/01.flac");
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();
        db.upsert_track(&other).unwrap();

        let album_id = id_of("/music/album");
        let tracks = db.get_tracks_by_album_id(&album_id).unwrap();
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn upsert_and_get_album() {
        let db = Database::open_in_memory().unwrap();
        let album = Album {
            id: id_of("/music/my_album"),
            dir_path: "/music/my_album".to_string(),
            title: Some("My Album".to_string()),
        };
        db.upsert_album(&album).unwrap();
        let fetched = db.get_album_by_id(&album.id).unwrap();
        assert_eq!(fetched, Some(album));
    }

    #[test]
    fn upsert_and_get_artist() {
        let db = Database::open_in_memory().unwrap();
        let artist = Artist {
            id: id_of("Miles Davis"),
            name: "Miles Davis".to_string(),
        };
        db.upsert_artist(&artist).unwrap();
        let fetched = db.get_artist_by_id(&artist.id).unwrap();
        assert_eq!(fetched, Some(artist));
    }

    #[test]
    fn fts_search_returns_matching_track() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();

        let ids = db.search("Test").unwrap();
        assert!(ids.contains(&track.id));
    }

    #[test]
    fn fts_search_no_match() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();

        let ids = db.search("ZZZnonexistent").unwrap();
        assert!(ids.is_empty());
    }
}
