use std::path::Path;

use anyhow::{Context, Result};
use id3::TagLike as _;
use kanade_core::model::Track;
use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::ItemKey,
};
use sha2::{Digest, Sha256};

pub fn extract_track(path: &str) -> Result<Track> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("dsf") => extract_dsf_track(path),
        _ => extract_lofty_track(path),
    }
}

fn extract_dsf_track(path: &str) -> Result<Track> {
    let dsf = dsf_meta::DsfFile::open(Path::new(path))
        .with_context(|| format!("cannot open DSF: {path}"))?;

    let fmt = dsf.fmt_chunk();
    let sample_rate = fmt.sampling_frequency();

    // duration = sample_count / sampling_frequency
    let duration_secs = if fmt.sample_count() > 0 && sample_rate > 0 {
        Some(fmt.sample_count() as f64 / sample_rate as f64)
    } else {
        None
    };

    // Map DSD sample rates to human-readable DSD rate names
    let dsd_rate = dsd_rate_name(sample_rate);
    let format = match dsd_rate {
        Some(name) => Some(format!("DSD ({name})")),
        None => Some("DSD".to_string()),
    };

    let id = id_of(path);

    let mut track = Track {
        id,
        file_path: path.to_string(),
        title: None,
        track_number: None,
        duration_secs,
        format,
        sample_rate: Some(sample_rate),
        artist: None,
        album_title: None,
        composer: None,
    };

    if let Some(tag) = dsf.id3_tag() {
        track.title = tag.title().map(|s| s.to_string());
        track.artist = tag.artist().map(|s| s.to_string());
        track.album_title = tag.album().map(|s| s.to_string());
        track.track_number = tag.track();
        track.composer = tag
            .get("TCOM")
            .and_then(|f| f.content().text())
            .map(|s: &str| s.to_string());
    }

    Ok(track)
}

fn dsd_rate_name(rate: u32) -> Option<&'static str> {
    match rate {
        2_822_400 => Some("DSD64"),
        5_644_800 => Some("DSD128"),
        11_289_600 => Some("DSD256"),
        22_579_200 => Some("DSD512"),
        _ => None,
    }
}

fn extract_lofty_track(path: &str) -> Result<Track> {
    let tagged_file = Probe::open(path)
        .with_context(|| format!("cannot open: {path}"))?
        .read()
        .with_context(|| format!("cannot read: {path}"))?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .context("no tags found")?;

    let props = tagged_file.properties();

    let id = id_of(path);

    let format = match tagged_file.file_type() {
        lofty::file::FileType::Flac => Some("FLAC"),
        lofty::file::FileType::Mpeg => Some("MP3"),
        lofty::file::FileType::Aac => Some("AAC"),
        lofty::file::FileType::Mp4 => Some("M4A"),
        lofty::file::FileType::Vorbis => Some("OGG"),
        lofty::file::FileType::Opus => Some("OPUS"),
        lofty::file::FileType::Wav => Some("WAV"),
        lofty::file::FileType::Aiff => Some("AIFF"),
        lofty::file::FileType::Ape => Some("APE"),
        _ => None,
    }
    .map(|s| s.to_string());

    let title = tag_string(tag, &ItemKey::TrackTitle);
    let artist = tag_string(tag, &ItemKey::TrackArtist);
    let album_title = tag_string(tag, &ItemKey::AlbumTitle);
    let composer = tag_string(tag, &ItemKey::Composer);

    let track_number = tag_string(tag, &ItemKey::TrackNumber).and_then(|s| s.parse::<u32>().ok());

    let duration_secs = props.duration().as_secs_f64();
    let duration_secs = if duration_secs > 0.0 {
        Some(duration_secs)
    } else {
        None
    };

    let sample_rate = props.sample_rate();

    Ok(Track {
        id,
        file_path: path.to_string(),
        title,
        track_number,
        duration_secs,
        format,
        sample_rate,
        artist,
        album_title,
        composer,
    })
}

fn tag_string(tag: &lofty::tag::Tag, key: &ItemKey) -> Option<String> {
    tag.get_string(key).map(|s: &str| s.to_string())
}

fn id_of(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}
