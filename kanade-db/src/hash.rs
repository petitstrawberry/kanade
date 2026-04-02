use sha2::{Digest, Sha256};

/// Compute a deterministic SHA-256 hex ID from an arbitrary string.
///
/// Used to derive:
/// - Track IDs   → `id_of(file_path)`
/// - Album IDs   → `id_of(directory_path)`
/// - Artist IDs  → `id_of(artist_name)`
pub fn id_of(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_gives_same_hash() {
        let a = id_of("/music/artist/album/01 - Track.flac");
        let b = id_of("/music/artist/album/01 - Track.flac");
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_give_different_hashes() {
        let a = id_of("/music/track_a.flac");
        let b = id_of("/music/track_b.flac");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = id_of("some input");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
