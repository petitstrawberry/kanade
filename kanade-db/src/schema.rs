use rusqlite::{Connection, Result};

/// DDL for every table and index in the Kanade schema.
///
/// Design principles (Purist Schema):
/// - `tracks.file_path` is the absolute truth; it is the natural primary key.
/// - `tracks.id` is a deterministic SHA-256 hex of `file_path` — no UUID, no
///   auto-increment that could diverge between runs.
/// - `albums.id` is SHA-256 of the album directory path.  All tracks that
///   live inside the same directory are part of the same album.
/// - `artists.id` is SHA-256 of the exact artist tag string.
/// - FTS5 virtual table covers title / album / artist / composer for fast
///   incremental search.
pub static SCHEMA_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- -------------------------------------------------------------------
-- artists
-- Primary key: SHA-256(name_string)
-- -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS artists (
    id   TEXT NOT NULL PRIMARY KEY,   -- SHA-256(name)
    name TEXT NOT NULL UNIQUE
);

-- -------------------------------------------------------------------
-- albums
-- Primary key: SHA-256(directory_path)
-- One directory == one album (the user's folder layout is the truth).
-- -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS albums (
    id       TEXT NOT NULL PRIMARY KEY,   -- SHA-256(dir_path)
    dir_path TEXT NOT NULL UNIQUE,
    title    TEXT                          -- from the first track's tag
);

-- -------------------------------------------------------------------
-- tracks
-- Primary key: file_path (the single source of truth).
-- id is SHA-256(file_path) for stable cross-reference.
-- -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tracks (
    file_path     TEXT NOT NULL PRIMARY KEY,
    id            TEXT NOT NULL UNIQUE,   -- SHA-256(file_path)
    album_id      TEXT REFERENCES albums(id) ON DELETE SET NULL,
    title         TEXT,
    track_number  INTEGER,
    duration_secs REAL,
    format        TEXT,
    sample_rate   INTEGER,
    artist        TEXT,
    album_title   TEXT,
    composer      TEXT
);

CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks (album_id);
CREATE INDEX IF NOT EXISTS idx_tracks_artist   ON tracks (artist);

-- -------------------------------------------------------------------
-- FTS5 full-text search
-- Covers the four most common search fields.
-- content='' means FTS5 stores its own copy — simpler to keep in sync.
-- -------------------------------------------------------------------
CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
    track_id UNINDEXED,   -- SHA-256(file_path), joins back to tracks.id
    title,
    album,
    artist,
    composer,
    tokenize='unicode61'
);

-- Keep the FTS index in sync with the tracks table via triggers.
CREATE TRIGGER IF NOT EXISTS tracks_fts_insert
AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts(rowid, track_id, title, album, artist, composer)
    VALUES (new.rowid, new.id, new.title, new.album_title, new.artist, new.composer);
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_delete
AFTER DELETE ON tracks BEGIN
    DELETE FROM tracks_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_update
AFTER UPDATE ON tracks BEGIN
    DELETE FROM tracks_fts WHERE rowid = old.rowid;
    INSERT INTO tracks_fts(rowid, track_id, title, album, artist, composer)
    VALUES (new.rowid, new.id, new.title, new.album_title, new.artist, new.composer);
END;
"#;

/// Apply the schema DDL to an open connection.
pub fn apply(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}
