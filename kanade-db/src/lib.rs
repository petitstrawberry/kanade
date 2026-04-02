//! kanade-db — Purist SQLite library for Kanade.
//!
//! Manages the on-disk database that indexes audio tracks, albums, and
//! artists using only the metadata embedded in the files' own tags.
//! External APIs are never consulted; IDs are deterministic hashes.

pub mod hash;
pub mod repo;
pub mod schema;

pub use repo::Database;
