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
        self.upsert_track_with_mtime(track, None)
    }

    /// Insert or replace a track record with its file modification time.
    ///
    /// `mtime` is the Unix epoch seconds from `std::fs::metadata().modified()`.
    /// Used by the scanner for incremental re-extraction.
    #[instrument(skip(self, track))]
    pub fn upsert_track_with_mtime(&self, track: &Track, mtime: Option<i64>) -> anyhow::Result<()> {
        let album_id = Path::new(&track.file_path).parent().map(|dir| {
            let dir_str = dir.to_string_lossy().into_owned();
            let aid = id_of(&dir_str);
            let album = kanade_core::model::Album {
                id: aid.clone(),
                dir_path: dir_str,
                title: track.album_title.clone(),
            };
            let _ = self.upsert_album(&album);
            aid
        });

        self.conn.execute(
            r#"INSERT INTO tracks
               (file_path, id, album_id, title, track_number, duration_secs,
                format, sample_rate, artist, album_artist, album_title, composer, genre, mtime)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
               ON CONFLICT(file_path) DO UPDATE SET
                   id            = excluded.id,
                   album_id      = excluded.album_id,
                   title         = excluded.title,
                   track_number  = excluded.track_number,
                   duration_secs = excluded.duration_secs,
                   format        = excluded.format,
                   sample_rate   = excluded.sample_rate,
                   artist        = excluded.artist,
                   album_artist  = excluded.album_artist,
                   album_title   = excluded.album_title,
                   composer      = excluded.composer,
                   genre         = excluded.genre,
                   mtime         = excluded.mtime"#,
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
                track.album_artist,
                track.album_title,
                track.composer,
                track.genre,
                mtime,
            ],
        )?;
        Ok(())
    }

    /// Fetch the stored mtime for a track, or `None` if not yet scanned.
    pub fn get_track_mtime(&self, file_path: &str) -> anyhow::Result<Option<i64>> {
        let result = self
            .conn
            .query_row(
                "SELECT mtime FROM tracks WHERE file_path = ?1",
                params![file_path],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?;
        Ok(result.flatten())
    }

    /// Fetch a single track by its file path, or `None` if not found.
    pub fn get_track_by_path(&self, file_path: &str) -> anyhow::Result<Option<Track>> {
        let result = self
            .conn
            .query_row(
                r#"SELECT file_path, id, title, track_number, duration_secs,
                          format, sample_rate, artist, album_artist, album_title, composer, genre
                   FROM tracks WHERE file_path = ?1"#,
                params![file_path],
                row_to_track,
            )
            .optional()?;
        Ok(result)
    }

    pub fn get_track_by_id(&self, track_id: &str) -> anyhow::Result<Option<Track>> {
        let result = self
            .conn
            .query_row(
                r#"SELECT file_path, id, title, track_number, duration_secs,
                          format, sample_rate, artist, album_artist, album_title, composer, genre
                   FROM tracks WHERE id = ?1"#,
                params![track_id],
                row_to_track,
            )
            .optional()?;
        Ok(result)
    }

    /// Fetch all tracks belonging to a given album directory.
    pub fn get_tracks_by_album_id(&self, album_id: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT file_path, id, title, track_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre
               FROM tracks WHERE album_id = ?1
               ORDER BY track_number, title"#,
        )?;
        let rows = stmt.query_map(params![album_id], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Fetch all audio file paths currently in the database.
    pub fn get_all_track_paths(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path FROM tracks ORDER BY file_path")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete tracks whose file_path is NOT in `known_paths`.
    ///
    /// Returns the number of rows removed.  Call after a full scan to purge
    /// entries for files that have been deleted from disk.
    pub fn purge_missing(&self, known_paths: &[String]) -> anyhow::Result<u64> {
        if known_paths.is_empty() {
            let removed = self.conn.execute("DELETE FROM tracks", [])?;
            return Ok(removed as u64);
        }

        let placeholders: Vec<String> = known_paths.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM tracks WHERE file_path NOT IN ({})",
            placeholders.join(",")
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = known_paths
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();

        let removed = self.conn.execute(&sql, params.as_slice())?;
        Ok(removed as u64)
    }

    /// Delete a track by file path (e.g. when the file is removed from disk).
    pub fn delete_track(&self, file_path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM tracks WHERE file_path = ?1",
            params![file_path],
        )?;
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

    /// Fetch all albums, ordered by directory path.
    pub fn get_all_albums(&self) -> anyhow::Result<Vec<Album>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, dir_path, title FROM albums ORDER BY dir_path")?;
        let rows = stmt.query_map([], |row| {
            Ok(Album {
                id: row.get(0)?,
                dir_path: row.get(1)?,
                title: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
                |row| {
                    Ok(Artist {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    // ------------------------------------------------------------------
    // Full-text search
    // ------------------------------------------------------------------

    /// Search tracks using FTS5.  Returns matching track IDs (SHA-256 hex).
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT track_id FROM tracks_fts WHERE tracks_fts MATCH ?1 ORDER BY rank")?;
        let rows = stmt.query_map(params![query], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Search tracks using FTS5, returning full `Track` objects.
    pub fn search_tracks(&self, query: &str) -> anyhow::Result<Vec<Track>> {
        let ids = self.search(query)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            r#"SELECT file_path, id, title, track_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre
               FROM tracks WHERE id IN ({}) ORDER BY file_path"#,
            placeholders.join(",")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ------------------------------------------------------------------
    // Artist queries (aggregate)
    // ------------------------------------------------------------------

    /// Fetch all distinct artist names, sorted alphabetically.
    pub fn get_all_artists(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT name FROM (
                    SELECT artist AS name FROM tracks WHERE artist IS NOT NULL
                    UNION
                    SELECT album_artist AS name FROM tracks WHERE album_artist IS NOT NULL
                ) ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_tracks_by_artist(&self, artist: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT file_path, id, title, track_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre
               FROM tracks WHERE artist = ?1 OR album_artist = ?1
               ORDER BY album_title, track_number, title"#,
        )?;
        let rows = stmt.query_map(params![artist, artist], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_albums_by_artist(&self, artist: &str) -> anyhow::Result<Vec<Album>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT DISTINCT a.id, a.dir_path, a.title
               FROM albums a
               JOIN tracks t ON t.album_id = a.id
               WHERE t.artist = ?1 OR t.album_artist = ?1
               ORDER BY a.title, a.dir_path"#,
        )?;
        let rows = stmt.query_map(params![artist, artist], |row| {
            Ok(Album {
                id: row.get(0)?,
                dir_path: row.get(1)?,
                title: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ------------------------------------------------------------------
    // Genre queries (aggregate)
    // ------------------------------------------------------------------

    /// Fetch all distinct genre names, sorted alphabetically.
    pub fn get_all_genres(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT genre FROM tracks WHERE genre IS NOT NULL ORDER BY genre")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Fetch all tracks by a given genre name.
    pub fn get_tracks_by_genre(&self, genre: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT file_path, id, title, track_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre
               FROM tracks WHERE genre = ?1
               ORDER BY artist, album_title, track_number, title"#,
        )?;
        let rows = stmt.query_map(params![genre], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_albums_by_genre(&self, genre: &str) -> anyhow::Result<Vec<Album>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT DISTINCT a.id, a.dir_path, a.title
               FROM albums a
               JOIN tracks t ON t.album_id = a.id
               WHERE t.genre = ?1
               ORDER BY a.title, a.dir_path"#,
        )?;
        let rows = stmt.query_map(params![genre], |row| {
            Ok(Album {
                id: row.get(0)?,
                dir_path: row.get(1)?,
                title: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ------------------------------------------------------------------
    // Transaction helpers
    // ------------------------------------------------------------------

    /// Execute a batch of upserts inside a single transaction.
    /// The closure receives the same `Database` reference but the connection
    /// is already in a transaction — just call `upsert_track_with_mtime`.
    pub fn in_transaction<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&Self) -> anyhow::Result<R>,
    {
        let tx = self.conn.unchecked_transaction()?;
        let result = f(self)?;
        tx.commit()?;
        Ok(result)
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
        album_artist: row.get(8)?,
        album_title: row.get(9)?,
        composer: row.get(10)?,
        genre: row.get(11)?,
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
            album_artist: Some("Test Album Artist".to_string()),
            album_title: Some("Test Album".to_string()),
            composer: Some("Test Composer".to_string()),
            genre: Some("Test Genre".to_string()),
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

    #[test]
    fn upsert_track_with_mtime() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track_with_mtime(&track, Some(1700000000))
            .unwrap();
        let mtime = db.get_track_mtime("/music/album/01.flac").unwrap();
        assert_eq!(mtime, Some(1700000000));
    }

    #[test]
    fn upsert_track_without_mtime() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();
        let mtime = db.get_track_mtime("/music/album/01.flac").unwrap();
        assert_eq!(mtime, None);
    }

    #[test]
    fn purge_missing_removes_deleted_files() {
        let db = Database::open_in_memory().unwrap();
        let t1 = sample_track("/music/album/01.flac");
        let t2 = sample_track("/music/album/02.flac");
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();

        let removed = db
            .purge_missing(&["/music/album/01.flac".to_string()])
            .unwrap();
        assert_eq!(removed, 1);
        assert!(db
            .get_track_by_path("/music/album/01.flac")
            .unwrap()
            .is_some());
        assert!(db
            .get_track_by_path("/music/album/02.flac")
            .unwrap()
            .is_none());
    }

    #[test]
    fn get_all_albums() {
        let db = Database::open_in_memory().unwrap();
        let t1 = sample_track("/music/album_a/01.flac");
        let t2 = sample_track("/music/album_b/01.flac");
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();

        let albums = db.get_all_albums().unwrap();
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn search_tracks_returns_full_tracks() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();

        let results = db.search_tracks("Test").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, Some("Test Track".to_string()));
    }

    #[test]
    fn get_all_track_paths() {
        let db = Database::open_in_memory().unwrap();
        let t1 = sample_track("/music/a.flac");
        let t2 = sample_track("/music/b.flac");
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();

        let paths = db.get_all_track_paths().unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn in_transaction_commits_on_success() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.in_transaction(|db| {
            db.upsert_track(&track)?;
            Ok(())
        })
        .unwrap();
        assert!(db
            .get_track_by_path("/music/album/01.flac")
            .unwrap()
            .is_some());
    }

    #[test]
    fn in_transaction_rollbacks_on_failure() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        let _: Result<(), anyhow::Error> = db.in_transaction(|db| {
            db.upsert_track(&track)?;
            anyhow::bail!("forced rollback")
        });
        assert!(db
            .get_track_by_path("/music/album/01.flac")
            .unwrap()
            .is_none());
    }
}
