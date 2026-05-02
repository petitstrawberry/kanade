use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use tracing::instrument;

use kanade_core::model::{
    Album, Artist, MatchMode, NodeType, Playlist, PlaylistKind, SmartField, SmartFilter,
    SmartOperator, SmartSort, Track,
};

use crate::{hash::id_of, schema};

/// High-level database handle.
///
/// All operations are synchronous and use a single `Connection` that can be
/// wrapped in a `tokio::task::spawn_blocking` call when used from async code.
pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedNodeState {
    pub node_id: String,
    pub queue_file_paths: Vec<String>,
    pub current_index: Option<usize>,
    pub active_output_id: Option<String>,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: String,
    pub node_type: NodeType,
    pub device_id: Option<String>,
    pub disconnected_at: Option<i64>,
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
                artist: None,
                artwork_path: None,
            };
            let _ = self.upsert_album(&album);
            aid
        });

        self.conn.execute(
            r#"INSERT INTO tracks
               (file_path, id, album_id, title, track_number, disc_number, duration_secs,
                format, sample_rate, artist, album_artist, album_title, composer, genre, mtime)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
               ON CONFLICT(file_path) DO UPDATE SET
                   id            = excluded.id,
                   album_id      = excluded.album_id,
                   title         = excluded.title,
                   track_number  = excluded.track_number,
                   disc_number   = excluded.disc_number,
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
                track.disc_number,
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
                 r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                           format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
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
                r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                           format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
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
            r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
                FROM tracks WHERE album_id = ?1
                 ORDER BY disc_number, track_number, title"#,
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

    pub fn save_node_state(
        &self,
        node_id: &str,
        queue_file_paths: &[String],
        current_index: Option<usize>,
        volume: u8,
        shuffle: bool,
        repeat: &str,
        node_type: NodeType,
        device_id: Option<&str>,
        disconnected_at: Option<i64>,
    ) -> anyhow::Result<()> {
        let queue_json = serde_json::to_string(queue_file_paths)?;
        let current_index = current_index.map(|i| i as i64);
        let volume = i64::from(volume);
        let shuffle = i64::from(shuffle as u8);
        let node_type = match node_type {
            NodeType::Remote => "remote",
            NodeType::Local => "local",
        };

        self.conn.execute(
            r#"INSERT INTO playback_state (node_id, queue, current_index, volume, shuffle, repeat, node_type, device_id, disconnected_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch())
               ON CONFLICT(node_id) DO UPDATE SET
                   queue         = excluded.queue,
                   current_index = excluded.current_index,
                   volume        = excluded.volume,
                   shuffle       = excluded.shuffle,
                   repeat        = excluded.repeat,
                   node_type     = excluded.node_type,
                   device_id     = excluded.device_id,
                   disconnected_at = excluded.disconnected_at,
                   updated_at    = unixepoch()"#,
            params![
                node_id,
                queue_json,
                current_index,
                volume,
                shuffle,
                repeat,
                node_type,
                device_id,
                disconnected_at,
            ],
        )?;
        Ok(())
    }

    pub fn prune_node_states_except(&self, keep_node_ids: &[String]) -> anyhow::Result<()> {
        if keep_node_ids.is_empty() {
            self.conn.execute(
                "DELETE FROM playback_state WHERE node_id != '__global__'",
                [],
            )?;
            return Ok(());
        }

        let placeholders = vec!["?"; keep_node_ids.len()].join(",");
        let sql = format!(
            "DELETE FROM playback_state WHERE node_id != '__global__' AND node_id NOT IN ({})",
            placeholders
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = keep_node_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        self.conn.execute(&sql, params.as_slice())?;
        Ok(())
    }

    pub fn load_all_node_states(&self) -> anyhow::Result<Vec<SavedNodeState>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, queue, current_index, volume, shuffle, repeat, active_output_id, node_type, device_id, disconnected_at
                 FROM playback_state ORDER BY node_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();

        while let Some(row) = rows.next()? {
            let node_id: String = row.get(0)?;
            let queue_json: String = row.get(1)?;
            let queue_file_paths: Vec<String> = serde_json::from_str(&queue_json)?;

            let current_index = row
                .get::<_, Option<i64>>(2)?
                .map(|i| {
                    usize::try_from(i).map_err(|_| anyhow::anyhow!("invalid current_index: {i}"))
                })
                .transpose()?;

            let volume_i64: i64 = row.get(3)?;
            let volume = u8::try_from(volume_i64)
                .map_err(|_| anyhow::anyhow!("invalid volume: {volume_i64}"))?;
            let shuffle = row.get::<_, i64>(4)? != 0;
            let repeat: String = row.get(5)?;
            let active_output_id: Option<String> = row.get(6)?;
            let node_type = match row.get::<_, String>(7)?.as_str() {
                "local" => NodeType::Local,
                _ => NodeType::Remote,
            };
            let device_id: Option<String> = row.get(8)?;
            let disconnected_at: Option<i64> = row.get(9)?;

            out.push(SavedNodeState {
                node_id,
                queue_file_paths,
                current_index,
                active_output_id,
                volume,
                shuffle,
                repeat,
                node_type,
                device_id,
                disconnected_at,
            });
        }

        Ok(out)
    }

    pub fn save_playback_state(
        &self,
        queue_file_paths: &[String],
        current_index: Option<usize>,
        active_output_id: Option<String>,
        shuffle: bool,
        repeat: &str,
    ) -> anyhow::Result<()> {
        let queue_json = serde_json::to_string(queue_file_paths)?;
        let current_index = current_index.map(|i| i as i64);
        let shuffle = i64::from(shuffle as u8);

        self.conn.execute(
            r#"INSERT INTO playback_state (node_id, queue, current_index, active_output_id, volume, shuffle, repeat, updated_at)
               VALUES ('__global__', ?1, ?2, ?3, 50, ?4, ?5, unixepoch())
               ON CONFLICT(node_id) DO UPDATE SET
                   queue         = excluded.queue,
                   current_index = excluded.current_index,
                    active_output_id = excluded.active_output_id,
                   shuffle       = excluded.shuffle,
                   repeat        = excluded.repeat,
                   updated_at    = unixepoch()"#,
            params![queue_json, current_index, active_output_id, shuffle, repeat],
        )?;
        Ok(())
    }

    pub fn load_playback_state(&self) -> anyhow::Result<Option<SavedNodeState>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, queue, current_index, volume, shuffle, repeat, active_output_id
                 FROM playback_state WHERE node_id = '__global__'",
        )?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            let queue_json: String = row.get(1)?;
            let queue_file_paths: Vec<String> = serde_json::from_str(&queue_json)?;

            let current_index = row
                .get::<_, Option<i64>>(2)?
                .map(|i| {
                    usize::try_from(i).map_err(|_| anyhow::anyhow!("invalid current_index: {i}"))
                })
                .transpose()?;

            let shuffle = row.get::<_, i64>(4)? != 0;
            let repeat: String = row.get(5)?;
            let active_output_id: Option<String> = row.get(6)?;

            return Ok(Some(SavedNodeState {
                node_id: "__global__".to_string(),
                queue_file_paths,
                current_index,
                active_output_id,
                volume: 50,
                shuffle,
                repeat,
                node_type: NodeType::Remote,
                device_id: None,
                disconnected_at: None,
            }));
        }

        Ok(None)
    }

    // ------------------------------------------------------------------
    // Album operations
    // ------------------------------------------------------------------

    /// Insert or replace an album record.
    pub fn upsert_album(&self, album: &Album) -> anyhow::Result<()> {
        self.conn.execute(
            r#"INSERT INTO albums (id, dir_path, title, artwork_path)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(id) DO UPDATE SET
                   dir_path     = excluded.dir_path,
                   title        = excluded.title,
                   artwork_path = excluded.artwork_path"#,
            params![album.id, album.dir_path, album.title, album.artwork_path],
        )?;
        Ok(())
    }

    pub fn update_album_artwork(
        &self,
        dir_path: &str,
        artwork_path: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE albums SET artwork_path = ?1 WHERE dir_path = ?2",
            params![artwork_path, dir_path],
        )?;
        Ok(())
    }

    pub fn get_album_artwork_path(&self, album_id: &str) -> anyhow::Result<Option<String>> {
        let result = self
            .conn
            .query_row(
                "SELECT artwork_path FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(result.flatten())
    }

    /// Fetch an album by its directory-hash ID.
    pub fn get_album_by_id(&self, album_id: &str) -> anyhow::Result<Option<Album>> {
        let sql = format!(
            "SELECT a.id, a.dir_path, a.title, a.artwork_path, artist_sub.artist \
             FROM albums a {} WHERE a.id = ?1",
            ALBUM_ARTIST_JOIN
        );
        let result = self
            .conn
            .query_row(&sql, params![album_id], row_to_album)
            .optional()?;
        Ok(result)
    }

    /// Fetch all albums, ordered by directory path.
    pub fn get_all_albums(&self) -> anyhow::Result<Vec<Album>> {
        let sql = format!(
            "SELECT a.id, a.dir_path, a.title, a.artwork_path, artist_sub.artist \
             FROM albums a {} ORDER BY a.dir_path",
            ALBUM_ARTIST_JOIN
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_album)?;
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
            r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
                FROM tracks WHERE id IN ({}) ORDER BY album_title, disc_number, track_number, title"#,
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
            r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
                FROM tracks WHERE artist = ?1 OR album_artist = ?1
                 ORDER BY album_title, disc_number, track_number, title"#,
        )?;
        let rows = stmt.query_map(params![artist], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_albums_by_artist(&self, artist: &str) -> anyhow::Result<Vec<Album>> {
        let sql = format!(
            "SELECT DISTINCT a.id, a.dir_path, a.title, a.artwork_path, artist_sub.artist \
             FROM albums a \
             JOIN tracks t ON t.album_id = a.id \
             {} \
             WHERE t.artist = ?1 OR t.album_artist = ?1 \
             ORDER BY a.title, a.dir_path",
            ALBUM_ARTIST_JOIN
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![artist], row_to_album)?;
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
            r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
                FROM tracks WHERE genre = ?1
                ORDER BY artist, album_title, disc_number, track_number, title"#,
        )?;
        let rows = stmt.query_map(params![genre], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_albums_by_genre(&self, genre: &str) -> anyhow::Result<Vec<Album>> {
        let sql = format!(
            "SELECT DISTINCT a.id, a.dir_path, a.title, a.artwork_path, artist_sub.artist \
             FROM albums a \
             JOIN tracks t ON t.album_id = a.id \
             {} \
             WHERE t.genre = ?1 \
             ORDER BY a.title, a.dir_path",
            ALBUM_ARTIST_JOIN
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![genre], row_to_album)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ------------------------------------------------------------------
    // Playlist operations
    // ------------------------------------------------------------------

    /// Create a playlist row and (for normal playlists) initialise its track
    /// list. Returns the persisted `Playlist` (with normalised timestamps).
    pub fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
        kind: &PlaylistKind,
    ) -> anyhow::Result<Playlist> {
        let id = generate_playlist_id(name);
        self.create_playlist_with_id(&id, name, description, kind)
    }

    /// Create a playlist with a caller-supplied id (mostly useful for tests
    /// and re-imports). Fails if the id already exists.
    pub fn create_playlist_with_id(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        kind: &PlaylistKind,
    ) -> anyhow::Result<Playlist> {
        let kind_str = match kind {
            PlaylistKind::Normal => "normal",
            PlaylistKind::Smart { .. } => "smart",
        };
        let smart_filter_json = match kind {
            PlaylistKind::Normal => None,
            PlaylistKind::Smart { .. } => Some(serde_json::to_string(kind)?),
        };

        self.conn.execute(
            r#"INSERT INTO playlists (id, name, description, kind, smart_filter, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch())"#,
            params![id, name, description, kind_str, smart_filter_json],
        )?;

        self.get_playlist(id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {id} not found after insert"))
    }

    /// Fetch a single playlist by id.
    pub fn get_playlist(&self, id: &str) -> anyhow::Result<Option<Playlist>> {
        self.conn
            .query_row(
                "SELECT id, name, description, kind, smart_filter, created_at, updated_at
                 FROM playlists WHERE id = ?1",
                params![id],
                row_to_playlist,
            )
            .optional()?
            .transpose()
    }

    /// List all playlists ordered by name.
    pub fn get_all_playlists(&self) -> anyhow::Result<Vec<Playlist>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, kind, smart_filter, created_at, updated_at
             FROM playlists ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_playlist)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Update the metadata (and, for smart playlists, the filter) of a
    /// playlist. The playlist `kind` may not change between normal and smart.
    pub fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        kind: Option<&PlaylistKind>,
    ) -> anyhow::Result<()> {
        let existing = self
            .get_playlist(id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {id} not found"))?;

        if let Some(new_kind) = kind {
            let same_variant = matches!(
                (&existing.kind, new_kind),
                (PlaylistKind::Normal, PlaylistKind::Normal)
                    | (PlaylistKind::Smart { .. }, PlaylistKind::Smart { .. })
            );
            if !same_variant {
                anyhow::bail!("cannot change playlist kind between normal and smart");
            }
        }

        let new_name = name.unwrap_or(&existing.name);
        let new_description: Option<String> = match description {
            Some(d) => d.map(str::to_string),
            None => existing.description.clone(),
        };
        let new_smart_filter_json = match kind.unwrap_or(&existing.kind) {
            PlaylistKind::Normal => None,
            k @ PlaylistKind::Smart { .. } => Some(serde_json::to_string(k)?),
        };

        self.conn.execute(
            r#"UPDATE playlists SET
                   name         = ?2,
                   description  = ?3,
                   smart_filter = ?4,
                   updated_at   = unixepoch()
               WHERE id = ?1"#,
            params![id, new_name, new_description, new_smart_filter_json],
        )?;
        Ok(())
    }

    /// Delete a playlist (cascades to `playlist_tracks`).
    pub fn delete_playlist(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Touch the `updated_at` timestamp of a playlist.
    fn touch_playlist(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE playlists SET updated_at = unixepoch() WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Replace the entire ordered track list of a normal playlist.
    /// `track_ids` is taken in order; positions are 0-based and contiguous.
    pub fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> anyhow::Result<()> {
        let pl = self
            .get_playlist(playlist_id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {playlist_id} not found"))?;
        if !matches!(pl.kind, PlaylistKind::Normal) {
            anyhow::bail!("cannot edit tracks of a smart playlist");
        }
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO playlist_tracks (playlist_id, position, track_id)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (idx, tid) in track_ids.iter().enumerate() {
                stmt.execute(params![playlist_id, idx as i64, tid])?;
            }
        }
        self.touch_playlist(playlist_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Append tracks to the end of a normal playlist.
    pub fn append_playlist_tracks(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> anyhow::Result<()> {
        if track_ids.is_empty() {
            return Ok(());
        }
        let pl = self
            .get_playlist(playlist_id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {playlist_id} not found"))?;
        if !matches!(pl.kind, PlaylistKind::Normal) {
            anyhow::bail!("cannot edit tracks of a smart playlist");
        }
        let tx = self.conn.unchecked_transaction()?;
        let next_pos: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO playlist_tracks (playlist_id, position, track_id)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (offset, tid) in track_ids.iter().enumerate() {
                stmt.execute(params![playlist_id, next_pos + offset as i64, tid])?;
            }
        }
        self.touch_playlist(playlist_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Remove the entry at `position` from a normal playlist and renumber the
    /// remaining entries.
    pub fn remove_playlist_track(&self, playlist_id: &str, position: usize) -> anyhow::Result<()> {
        let pl = self
            .get_playlist(playlist_id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {playlist_id} not found"))?;
        if !matches!(pl.kind, PlaylistKind::Normal) {
            anyhow::bail!("cannot edit tracks of a smart playlist");
        }
        let tx = self.conn.unchecked_transaction()?;
        let removed = self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND position = ?2",
            params![playlist_id, position as i64],
        )?;
        if removed > 0 {
            self.conn.execute(
                "UPDATE playlist_tracks SET position = position - 1
                 WHERE playlist_id = ?1 AND position > ?2",
                params![playlist_id, position as i64],
            )?;
            self.touch_playlist(playlist_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Move the entry at `from` to `to` within a normal playlist, shifting
    /// the entries in between.
    pub fn move_playlist_track(
        &self,
        playlist_id: &str,
        from: usize,
        to: usize,
    ) -> anyhow::Result<()> {
        if from == to {
            return Ok(());
        }
        let pl = self
            .get_playlist(playlist_id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {playlist_id} not found"))?;
        if !matches!(pl.kind, PlaylistKind::Normal) {
            anyhow::bail!("cannot edit tracks of a smart playlist");
        }

        let tx = self.conn.unchecked_transaction()?;
        // Read the current list, reorder in memory, and rewrite.
        let mut ids: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT track_id FROM playlist_tracks
                 WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![playlist_id], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if from >= ids.len() {
            anyhow::bail!("from index {from} out of range");
        }
        let to = to.min(ids.len() - 1);
        let item = ids.remove(from);
        ids.insert(to, item);
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO playlist_tracks (playlist_id, position, track_id)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (idx, tid) in ids.iter().enumerate() {
                stmt.execute(params![playlist_id, idx as i64, tid])?;
            }
        }
        self.touch_playlist(playlist_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Resolve a playlist's contents to full `Track` records, in playlist
    /// order. Works for both normal and smart playlists.
    pub fn get_playlist_tracks(&self, playlist_id: &str) -> anyhow::Result<Vec<Track>> {
        let pl = self
            .get_playlist(playlist_id)?
            .ok_or_else(|| anyhow::anyhow!("playlist {playlist_id} not found"))?;
        match pl.kind {
            PlaylistKind::Normal => self.get_normal_playlist_tracks(playlist_id),
            PlaylistKind::Smart {
                filter,
                limit,
                sort_by,
            } => self.evaluate_smart_filter(&filter, sort_by, limit),
        }
    }

    fn get_normal_playlist_tracks(&self, playlist_id: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT t.file_path, t.id, t.title, t.track_number, t.disc_number, t.duration_secs,
                      t.format, t.sample_rate, t.artist, t.album_artist, t.album_title,
                      t.composer, t.genre, t.album_id
               FROM playlist_tracks pt
               JOIN tracks t ON t.id = pt.track_id
               WHERE pt.playlist_id = ?1
               ORDER BY pt.position"#,
        )?;
        let rows = stmt.query_map(params![playlist_id], row_to_track)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Evaluate a smart filter against the `tracks` table and return the
    /// matching tracks. Exposed for tests and reuse.
    pub fn evaluate_smart_filter(
        &self,
        filter: &SmartFilter,
        sort_by: Option<SmartSort>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<Track>> {
        // An empty filter intentionally matches nothing — guards against an
        // unconfigured smart playlist accidentally returning the whole library.
        if filter.conditions.is_empty() {
            return Ok(Vec::new());
        }

        let mut where_parts: Vec<String> = Vec::with_capacity(filter.conditions.len());
        let mut values: Vec<String> = Vec::with_capacity(filter.conditions.len());
        for cond in &filter.conditions {
            let column = smart_field_column(cond.field);
            let (sql_op, bind_value) = match cond.op {
                SmartOperator::Equals => ("=", cond.value.clone()),
                SmartOperator::NotEquals => ("!=", cond.value.clone()),
                SmartOperator::Contains => ("LIKE", format!("%{}%", escape_like(&cond.value))),
                SmartOperator::NotContains => {
                    ("NOT LIKE", format!("%{}%", escape_like(&cond.value)))
                }
                SmartOperator::StartsWith => ("LIKE", format!("{}%", escape_like(&cond.value))),
                SmartOperator::EndsWith => ("LIKE", format!("%{}", escape_like(&cond.value))),
            };
            let escape_clause = matches!(
                cond.op,
                SmartOperator::Contains
                    | SmartOperator::NotContains
                    | SmartOperator::StartsWith
                    | SmartOperator::EndsWith
            )
            .then_some(" ESCAPE '\\'")
            .unwrap_or("");
            where_parts.push(format!(
                "({col} IS NOT NULL AND {col} {op} ?{escape})",
                col = column,
                op = sql_op,
                escape = escape_clause,
            ));
            values.push(bind_value);
        }

        let joiner = match filter.match_mode {
            MatchMode::All => " AND ",
            MatchMode::Any => " OR ",
        };
        let where_clause = where_parts.join(joiner);
        let order_clause = match sort_by {
            Some(SmartSort::Title) => "ORDER BY title COLLATE NOCASE",
            Some(SmartSort::Artist) => {
                "ORDER BY artist COLLATE NOCASE, album_title, disc_number, track_number"
            }
            Some(SmartSort::Album) => {
                "ORDER BY album_title COLLATE NOCASE, disc_number, track_number"
            }
            Some(SmartSort::Genre) => {
                "ORDER BY genre COLLATE NOCASE, artist, album_title, disc_number, track_number"
            }
            None => "ORDER BY artist, album_title, disc_number, track_number, title",
        };
        let limit_clause = match limit {
            Some(n) => format!(" LIMIT {n}"),
            None => String::new(),
        };

        let sql = format!(
            r#"SELECT file_path, id, title, track_number, disc_number, duration_secs,
                      format, sample_rate, artist, album_artist, album_title, composer, genre, album_id
               FROM tracks
               WHERE {where_clause}
               {order_clause}{limit_clause}"#,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_track)?;
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
// Helpers — shared SQL fragment and row mappers for album queries
// ---------------------------------------------------------------------------

/// LEFT JOIN fragment that resolves the album artist from the associated
/// tracks.  Prefers `album_artist`; falls back to `artist`.  The result is
/// exposed as the column alias `artist_sub.artist`.
const ALBUM_ARTIST_JOIN: &str = r#"LEFT JOIN (
    SELECT album_id, COALESCE(album_artist, artist) AS artist
    FROM tracks
    WHERE album_artist IS NOT NULL OR artist IS NOT NULL
    GROUP BY album_id
) artist_sub ON artist_sub.album_id = a.id"#;

fn row_to_album(row: &rusqlite::Row<'_>) -> rusqlite::Result<Album> {
    Ok(Album {
        id: row.get(0)?,
        dir_path: row.get(1)?,
        title: row.get(2)?,
        artwork_path: row.get(3)?,
        artist: row.get(4)?,
    })
}

// ---------------------------------------------------------------------------
// Helper — map a rusqlite Row to a Track
// ---------------------------------------------------------------------------

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        file_path: row.get(0)?,
        id: row.get(1)?,
        album_id: row.get(13)?,
        title: row.get(2)?,
        track_number: row.get(3)?,
        disc_number: row.get(4)?,
        duration_secs: row.get(5)?,
        format: row.get(6)?,
        sample_rate: row.get(7)?,
        artist: row.get(8)?,
        album_artist: row.get(9)?,
        album_title: row.get(10)?,
        composer: row.get(11)?,
        genre: row.get(12)?,
    })
}

/// Map a `playlists` row to a `Playlist` model.
///
/// Returns a nested `Result` because the JSON deserialisation of the
/// `smart_filter` column is fallible at the application layer rather than at
/// the rusqlite layer.
fn row_to_playlist(row: &rusqlite::Row<'_>) -> rusqlite::Result<anyhow::Result<Playlist>> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let description: Option<String> = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let smart_filter_json: Option<String> = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    let updated_at: i64 = row.get(6)?;
    Ok((|| -> anyhow::Result<Playlist> {
        let kind = match kind_str.as_str() {
            "normal" => PlaylistKind::Normal,
            "smart" => {
                let json = smart_filter_json
                    .ok_or_else(|| anyhow::anyhow!("smart playlist {id} missing filter"))?;
                serde_json::from_str::<PlaylistKind>(&json)?
            }
            other => anyhow::bail!("unknown playlist kind: {other}"),
        };
        Ok(Playlist {
            id,
            name,
            description,
            kind,
            created_at,
            updated_at,
        })
    })())
}

fn smart_field_column(field: SmartField) -> &'static str {
    match field {
        SmartField::Title => "title",
        SmartField::Artist => "artist",
        SmartField::AlbumArtist => "album_artist",
        SmartField::Album => "album_title",
        SmartField::Composer => "composer",
        SmartField::Genre => "genre",
    }
}

/// Escape SQL `LIKE` metacharacters so they match literally. The query uses
/// `ESCAPE '\\'` so we escape `\\`, `%` and `_`.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Generate a deterministic-ish playlist id. Includes a random suffix so
/// re-using the same name yields a fresh row.
fn generate_playlist_id(name: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    crate::hash::id_of(&format!("playlist:{name}:{now_ns}"))
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
            album_id: None,
            title: Some("Test Track".to_string()),
            track_number: Some(1),
            disc_number: None,
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

        let mut expected = track;
        expected.album_id = Some(id_of("/music/album"));
        let fetched = db.get_track_by_path("/music/album/01.flac").unwrap();
        assert_eq!(fetched, Some(expected));
    }

    #[test]
    fn upsert_track_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();
        db.upsert_track(&track).unwrap(); // must not error
        let mut expected = track;
        expected.album_id = Some(id_of("/music/album"));
        let fetched = db.get_track_by_path("/music/album/01.flac").unwrap();
        assert_eq!(fetched, Some(expected));
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
            artist: None,
            artwork_path: None,
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
    fn album_artist_derived_from_tracks() {
        let db = Database::open_in_memory().unwrap();
        let track = sample_track("/music/album/01.flac");
        db.upsert_track(&track).unwrap();

        let album_id = id_of("/music/album");

        // artist should be derived from the track's album_artist field
        let album = db.get_album_by_id(&album_id).unwrap().unwrap();
        assert_eq!(album.artist, Some("Test Album Artist".to_string()));

        // get_all_albums should also populate artist
        let albums = db.get_all_albums().unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, Some("Test Album Artist".to_string()));
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

    #[test]
    fn save_and_load_node_state() {
        let db = Database::open_in_memory().unwrap();
        let queue = vec![
            "/music/album/01.flac".to_string(),
            "/music/album/02.flac".to_string(),
        ];

        db.save_node_state(
            "node-a",
            &queue,
            Some(1),
            77,
            true,
            "all",
            NodeType::Remote,
            None,
            None,
        )
        .unwrap();

        let states = db.load_all_node_states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(
            states[0],
            SavedNodeState {
                node_id: "node-a".to_string(),
                queue_file_paths: queue,
                current_index: Some(1),
                active_output_id: None,
                volume: 77,
                shuffle: true,
                repeat: "all".to_string(),
                node_type: NodeType::Remote,
                device_id: None,
                disconnected_at: None,
            }
        );
    }

    #[test]
    fn save_node_state_upsert() {
        let db = Database::open_in_memory().unwrap();

        db.save_node_state(
            "node-a",
            &["/music/album/01.flac".to_string()],
            Some(0),
            50,
            false,
            "off",
            NodeType::Remote,
            None,
            None,
        )
        .unwrap();

        db.save_node_state(
            "node-a",
            &[
                "/music/album/02.flac".to_string(),
                "/music/album/03.flac".to_string(),
            ],
            Some(1),
            90,
            true,
            "one",
            NodeType::Local,
            Some("device-a"),
            Some(123),
        )
        .unwrap();

        let states = db.load_all_node_states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].node_id, "node-a");
        assert_eq!(
            states[0].queue_file_paths,
            vec![
                "/music/album/02.flac".to_string(),
                "/music/album/03.flac".to_string()
            ]
        );
        assert_eq!(states[0].current_index, Some(1));
        assert_eq!(states[0].volume, 90);
        assert!(states[0].shuffle);
        assert_eq!(states[0].repeat, "one");
        assert_eq!(states[0].node_type, NodeType::Local);
        assert_eq!(states[0].device_id.as_deref(), Some("device-a"));
        assert_eq!(states[0].disconnected_at, Some(123));
    }

    #[test]
    fn load_node_states_empty() {
        let db = Database::open_in_memory().unwrap();
        let states = db.load_all_node_states().unwrap();
        assert!(states.is_empty());
    }

    #[test]
    fn save_and_load_playback_state_with_active_output() {
        let db = Database::open_in_memory().unwrap();
        let queue = vec![
            "/music/album/01.flac".to_string(),
            "/music/album/02.flac".to_string(),
        ];

        db.save_playback_state(&queue, Some(1), Some("node-a".to_string()), true, "all")
            .unwrap();

        let state = db.load_playback_state().unwrap().unwrap();
        assert_eq!(state.node_id, "__global__");
        assert_eq!(state.queue_file_paths, queue);
        assert_eq!(state.current_index, Some(1));
        assert_eq!(state.active_output_id.as_deref(), Some("node-a"));
        assert!(state.shuffle);
        assert_eq!(state.repeat, "all");
    }

    // --- Playlist tests --------------------------------------------------

    fn track_with(file_path: &str, title: &str, artist: &str, album: &str, genre: &str) -> Track {
        Track {
            id: id_of(file_path),
            file_path: file_path.to_string(),
            album_id: None,
            title: Some(title.to_string()),
            artist: Some(artist.to_string()),
            album_artist: Some(artist.to_string()),
            album_title: Some(album.to_string()),
            composer: Some("Composer".to_string()),
            genre: Some(genre.to_string()),
            track_number: Some(1),
            disc_number: None,
            duration_secs: Some(180.0),
            format: Some("FLAC".to_string()),
            sample_rate: Some(44100),
        }
    }

    #[test]
    fn create_and_get_normal_playlist() {
        let db = Database::open_in_memory().unwrap();
        let pl = db
            .create_playlist("Favourites", Some("My favs"), &PlaylistKind::Normal)
            .unwrap();
        assert_eq!(pl.name, "Favourites");
        assert_eq!(pl.description.as_deref(), Some("My favs"));
        assert!(matches!(pl.kind, PlaylistKind::Normal));

        let fetched = db.get_playlist(&pl.id).unwrap().unwrap();
        assert_eq!(fetched, pl);

        let all = db.get_all_playlists().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn normal_playlist_track_operations() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "T1", "A", "Alb", "Rock");
        let t2 = track_with("/m/a/02.flac", "T2", "A", "Alb", "Rock");
        let t3 = track_with("/m/a/03.flac", "T3", "A", "Alb", "Rock");
        for t in [&t1, &t2, &t3] {
            db.upsert_track(t).unwrap();
        }
        let pl = db
            .create_playlist("Mix", None, &PlaylistKind::Normal)
            .unwrap();

        // append
        db.append_playlist_tracks(&pl.id, &[t1.id.clone(), t2.id.clone()])
            .unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, t1.id);
        assert_eq!(tracks[1].id, t2.id);

        // append more
        db.append_playlist_tracks(&pl.id, &[t3.id.clone()]).unwrap();
        assert_eq!(db.get_playlist_tracks(&pl.id).unwrap().len(), 3);

        // move t3 (idx 2) to position 0
        db.move_playlist_track(&pl.id, 2, 0).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks[0].id, t3.id);
        assert_eq!(tracks[1].id, t1.id);
        assert_eq!(tracks[2].id, t2.id);

        // remove position 1 (t1)
        db.remove_playlist_track(&pl.id, 1).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, t3.id);
        assert_eq!(tracks[1].id, t2.id);

        // set replaces
        db.set_playlist_tracks(&pl.id, &[t1.id.clone()]).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, t1.id);
    }

    #[test]
    fn smart_playlist_evaluation_match_all() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "Hello", "Alice", "First", "Rock");
        let t2 = track_with("/m/a/02.flac", "World", "Alice", "First", "Jazz");
        let t3 = track_with("/m/b/01.flac", "Goodbye", "Bob", "Second", "Rock");
        for t in [&t1, &t2, &t3] {
            db.upsert_track(t).unwrap();
        }
        let kind = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::All,
                conditions: vec![
                    kanade_core::model::SmartCondition {
                        field: SmartField::Artist,
                        op: SmartOperator::Equals,
                        value: "Alice".to_string(),
                    },
                    kanade_core::model::SmartCondition {
                        field: SmartField::Genre,
                        op: SmartOperator::Equals,
                        value: "Rock".to_string(),
                    },
                ],
            },
            limit: None,
            sort_by: None,
        };
        let pl = db.create_playlist("Alice Rock", None, &kind).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, t1.id);
    }

    #[test]
    fn smart_playlist_evaluation_match_any_contains() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "Hello World", "Alice", "First", "Rock");
        let t2 = track_with("/m/a/02.flac", "Goodbye", "Bob", "Second", "Jazz");
        let t3 = track_with("/m/c/01.flac", "Untitled", "Carol", "Third", "Rock");
        for t in [&t1, &t2, &t3] {
            db.upsert_track(t).unwrap();
        }
        let kind = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::Any,
                conditions: vec![
                    kanade_core::model::SmartCondition {
                        field: SmartField::Title,
                        op: SmartOperator::Contains,
                        value: "World".to_string(),
                    },
                    kanade_core::model::SmartCondition {
                        field: SmartField::Artist,
                        op: SmartOperator::Equals,
                        value: "Carol".to_string(),
                    },
                ],
            },
            limit: None,
            sort_by: Some(SmartSort::Title),
        };
        let pl = db.create_playlist("Mix", None, &kind).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&t1.id.as_str()));
        assert!(ids.contains(&t3.id.as_str()));
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn smart_playlist_empty_filter_matches_nothing() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "T", "A", "Alb", "Rock");
        db.upsert_track(&t1).unwrap();
        let kind = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::All,
                conditions: vec![],
            },
            limit: None,
            sort_by: None,
        };
        let pl = db.create_playlist("Empty", None, &kind).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert!(tracks.is_empty());
    }

    #[test]
    fn smart_playlist_like_escapes_metacharacters() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "100% Pure", "A", "Alb", "Rock");
        let t2 = track_with("/m/a/02.flac", "100 Pure", "A", "Alb", "Rock");
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();
        let kind = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::All,
                conditions: vec![kanade_core::model::SmartCondition {
                    field: SmartField::Title,
                    op: SmartOperator::Contains,
                    value: "100%".to_string(),
                }],
            },
            limit: None,
            sort_by: None,
        };
        let pl = db.create_playlist("Pct", None, &kind).unwrap();
        let tracks = db.get_playlist_tracks(&pl.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, t1.id);
    }

    #[test]
    fn cannot_edit_smart_playlist_tracks() {
        let db = Database::open_in_memory().unwrap();
        let kind = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::All,
                conditions: vec![kanade_core::model::SmartCondition {
                    field: SmartField::Genre,
                    op: SmartOperator::Equals,
                    value: "Rock".to_string(),
                }],
            },
            limit: None,
            sort_by: None,
        };
        let pl = db.create_playlist("Smart", None, &kind).unwrap();
        let err = db.append_playlist_tracks(&pl.id, &["x".to_string()]);
        assert!(err.is_err());
        let err = db.set_playlist_tracks(&pl.id, &["x".to_string()]);
        assert!(err.is_err());
    }

    #[test]
    fn update_and_delete_playlist() {
        let db = Database::open_in_memory().unwrap();
        let pl = db
            .create_playlist("Old", Some("desc"), &PlaylistKind::Normal)
            .unwrap();
        db.update_playlist(&pl.id, Some("New"), Some(None), None)
            .unwrap();
        let updated = db.get_playlist(&pl.id).unwrap().unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.description, None);

        // changing kind variant must fail
        let smart = PlaylistKind::Smart {
            filter: SmartFilter {
                match_mode: MatchMode::All,
                conditions: vec![],
            },
            limit: None,
            sort_by: None,
        };
        assert!(db
            .update_playlist(&pl.id, None, None, Some(&smart))
            .is_err());

        db.delete_playlist(&pl.id).unwrap();
        assert!(db.get_playlist(&pl.id).unwrap().is_none());
    }

    #[test]
    fn delete_playlist_cascades_tracks() {
        let db = Database::open_in_memory().unwrap();
        let t1 = track_with("/m/a/01.flac", "T", "A", "Alb", "Rock");
        db.upsert_track(&t1).unwrap();
        let pl = db
            .create_playlist("X", None, &PlaylistKind::Normal)
            .unwrap();
        db.append_playlist_tracks(&pl.id, &[t1.id.clone()]).unwrap();
        db.delete_playlist(&pl.id).unwrap();
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1",
                params![pl.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
