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
/// Schema version. Increment when adding columns or tables.
pub const SCHEMA_VERSION: i32 = 7;

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
    id           TEXT NOT NULL PRIMARY KEY,   -- SHA-256(dir_path)
    dir_path     TEXT NOT NULL UNIQUE,
    title        TEXT,                         -- from the first track's tag
    artwork_path TEXT                          -- path to cover art image
);

-- -------------------------------------------------------------------
-- tracks
-- Primary key: file_path (the single source of truth).
-- id is SHA-256(file_path) for stable cross-reference.
-- mtime is the file modification timestamp (Unix epoch seconds) used
-- for incremental scanning.
-- -------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tracks (
    file_path     TEXT NOT NULL PRIMARY KEY,
    id            TEXT NOT NULL UNIQUE,   -- SHA-256(file_path)
    album_id      TEXT REFERENCES albums(id) ON DELETE SET NULL,
    title         TEXT,
    track_number  INTEGER,
    disc_number   INTEGER,
    duration_secs REAL,
    format        TEXT,
    sample_rate   INTEGER,
    artist        TEXT,
    album_artist  TEXT,
    album_title   TEXT,
    composer      TEXT,
    genre         TEXT,
    mtime         INTEGER                 -- file modification time (epoch secs)
);

CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks (album_id);
CREATE INDEX IF NOT EXISTS idx_tracks_artist   ON tracks (artist);
CREATE INDEX IF NOT EXISTS idx_tracks_mtime    ON tracks (mtime);

-- -------------------------------------------------------------------
-- FTS5 full-text search
-- Covers the four most common search fields.
-- content='' means FTS5 stores its own copy — simpler to keep in sync.
-- -------------------------------------------------------------------
CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
    track_id UNINDEXED,
    title,
    album,
    artist,
    album_artist,
    composer,
    genre,
    tokenize='unicode61'
);

CREATE TRIGGER IF NOT EXISTS tracks_fts_insert
AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts(rowid, track_id, title, album, artist, album_artist, composer, genre)
    VALUES (new.rowid, new.id, new.title, new.album_title, new.artist, new.album_artist, new.composer, new.genre);
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_delete
AFTER DELETE ON tracks BEGIN
    DELETE FROM tracks_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_update
AFTER UPDATE ON tracks BEGIN
    DELETE FROM tracks_fts WHERE rowid = old.rowid;
    INSERT INTO tracks_fts(rowid, track_id, title, album, artist, album_artist, composer, genre)
    VALUES (new.rowid, new.id, new.title, new.album_title, new.artist, new.album_artist, new.composer, new.genre);
END;
"#;

/// Migrations keyed by schema version.
/// Each migration is idempotent (safe to re-run).
static MIGRATIONS: &[(&str, &str)] = &[
    (
        "1",
        r#"
            -- v1: add mtime column for incremental scanning
            -- Safe to re-run: will fail silently if column already exists.
            -- (We catch the error in apply_migrations.)
            ALTER TABLE tracks ADD COLUMN mtime INTEGER;
        "#,
    ),
    (
        "2",
        r#"
            ALTER TABLE tracks ADD COLUMN genre TEXT;
            UPDATE tracks SET mtime = NULL;
        "#,
    ),
    (
        "3",
        r#"
            ALTER TABLE tracks ADD COLUMN album_artist TEXT;
            UPDATE tracks SET mtime = NULL;
        "#,
    ),
    (
        "4",
        r#"
            ALTER TABLE albums ADD COLUMN artwork_path TEXT;
            UPDATE tracks SET mtime = NULL;
        "#,
    ),
    (
        "5",
        r#"
            ALTER TABLE tracks ADD COLUMN disc_number INTEGER;
            UPDATE tracks SET mtime = NULL;
        "#,
    ),
    (
        "6",
        r#"
            CREATE TABLE IF NOT EXISTS playback_state (
                node_id       TEXT PRIMARY KEY,
                queue         TEXT NOT NULL DEFAULT '[]',
                current_index INTEGER,
                volume        INTEGER NOT NULL DEFAULT 50,
                shuffle       INTEGER NOT NULL DEFAULT 0,
                repeat        TEXT NOT NULL DEFAULT 'off',
                updated_at    INTEGER NOT NULL DEFAULT (unixepoch())
            );
        "#,
    ),
    (
        "7",
        r#"
            ALTER TABLE playback_state ADD COLUMN active_output_id TEXT;
            UPDATE playback_state SET active_output_id = NULL WHERE active_output_id IS NULL;
        "#,
    ),
];

/// Apply the base schema DDL to an open connection.
pub fn apply(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    apply_migrations(conn)
}

/// Run any pending migrations based on `user_version`.
fn apply_migrations(conn: &Connection) -> Result<()> {
    let current: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for &(version, sql) in MIGRATIONS {
        let v: i32 = version.parse().unwrap();
        if v <= current {
            continue;
        }
        // ALTER TABLE ADD COLUMN fails if column exists — that's fine.
        if let Err(e) = conn.execute_batch(sql) {
            tracing::debug!("migration v{version} skipped (may already be applied): {e}");
        }
        conn.pragma_update(None, "user_version", v)?;
        tracing::info!("applied schema migration v{version}");
    }

    Ok(())
}
