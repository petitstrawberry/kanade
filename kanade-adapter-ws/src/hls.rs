use std::{
    borrow::Cow,
    collections::HashMap,
    io::{BufWriter, Write},
    num::NonZeroU32,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alac_encoder::{AlacEncoder, FormatDescription, PcmFormat, DEFAULT_FRAMES_PER_PACKET};
use hls_m3u8::{
    MediaPlaylist, MediaSegment,
    tags::{ExtInf, ExtXMap},
    types::PlaylistType,
};
use lofty::{file::{FileType, TaggedFileExt}, prelude::AudioFile, probe::Probe};
use shiguredo_mp4::{
    BoxSize, BoxType, FixedPointNumber, TrackKind, Uint,
    boxes::{
        AudioSampleEntryFields, DflaBox, DopsBox, EsdsBox, FlacBox, FlacMetadataBlock, Mp4aBox,
        OpusBox, SampleEntry, UnknownBox,
    },
    demux::{Input, Mp4FileDemuxer},
    descriptors::{
        DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor, SlConfigDescriptor,
    },
    mux::{Fmp4SegmentMuxer, MuxError, Sample},
};
use thiserror::Error;
use tokio::{fs, sync::Mutex};

const DEFAULT_SEGMENT_DURATION_SECS: u64 = 6;
pub const DEFAULT_MAX_CACHE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HlsSegments {
    root: PathBuf,
    segment_count: usize,
}

impl HlsSegments {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, HlsError> {
        let root = root.into();
        if !root.join("index.m3u8").exists() || !root.join("init.mp4").exists() {
            return Err(HlsError::MissingCache(root));
        }

        let mut segment_count = 0usize;
        loop {
            if root.join(format!("seg{segment_count}.m4s")).exists() {
                segment_count += 1;
            } else {
                break;
            }
        }

        if segment_count == 0 {
            return Err(HlsError::InvalidData(
                "generated HLS cache did not contain any media segments".to_string(),
            ));
        }

        Ok(Self {
            root,
            segment_count,
        })
    }

    pub fn playlist_path(&self) -> PathBuf {
        self.root.join("index.m3u8")
    }

    pub fn init_path(&self) -> PathBuf {
        self.root.join("init.mp4")
    }

    pub fn segment_path(&self, index: usize) -> Option<PathBuf> {
        (index < self.segment_count).then(|| self.root.join(format!("seg{index}.m4s")))
    }

    pub fn segment_count(&self) -> usize {
        self.segment_count
    }

    pub async fn touch(&self) -> Result<(), HlsError> {
        let stamp = self.root.join(".last_access");
        let now = current_unix_secs().to_string();
        fs::write(stamp, now).await?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug)]
pub struct HlsCache {
    root: PathBuf,
    max_size_bytes: u64,
    segment_duration: Duration,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl HlsCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_options(
            root,
            DEFAULT_MAX_CACHE_BYTES,
            Duration::from_secs(DEFAULT_SEGMENT_DURATION_SECS),
        )
    }

    pub fn from_env() -> Self {
        let root = std::env::var("HLS_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let db_dir = std::env::var("DB_PATH")
                    .map(|p| PathBuf::from(p).parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from(".")))
                    .unwrap_or_else(|_| PathBuf::from("."));
                db_dir.join(".hls-cache")
            });
        Self::new(root)
    }

    pub fn with_options(
        root: impl Into<PathBuf>,
        max_size_bytes: u64,
        segment_duration: Duration,
    ) -> Self {
        Self {
            root: root.into(),
            max_size_bytes,
            segment_duration,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn max_size_bytes(&self) -> u64 {
        self.max_size_bytes
    }

    pub async fn get_or_generate(
        &self,
        source_path: &Path,
        track_id: &str,
        variant: &str,
    ) -> Result<HlsSegments, HlsError> {
        fs::create_dir_all(&self.root).await?;
        let key = format!("{track_id}/{variant}");
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
        };
        let _guard = lock.lock().await;

        self.enforce_size_limit(None).await?;

        let variant_dir = self.variant_dir(track_id, variant);
        if let Ok(segments) = HlsSegments::open(&variant_dir) {
            segments.touch().await?;
            return Ok(segments);
        }

        let source_path = source_path.to_path_buf();
        let track_id = track_id.to_string();
        let variant = variant.to_string();
        let cache_root = self.root.clone();
        let segment_duration = self.segment_duration;

        let generated = tokio::task::spawn_blocking(move || {
            generate_hls_with_options(
                &source_path,
                &track_id,
                &variant,
                &cache_root,
                segment_duration,
            )
        })
        .await
        .map_err(HlsError::Join)??;

        generated.touch().await?;
        self.enforce_size_limit(Some(generated.root())).await?;
        Ok(generated)
    }

    fn variant_dir(&self, track_id: &str, variant: &str) -> PathBuf {
        self.root.join(track_id).join(variant)
    }

    async fn enforce_size_limit(&self, preserve: Option<&Path>) -> Result<(), HlsError> {
        let root = self.root.clone();
        let max_size_bytes = self.max_size_bytes;
        let preserve = preserve.map(Path::to_path_buf);
        tokio::task::spawn_blocking(move || {
            enforce_size_limit_blocking(&root, max_size_bytes, preserve.as_deref())
        })
        .await
        .map_err(HlsError::Join)??;
        Ok(())
    }
}

pub fn generate_hls(
    source_path: &Path,
    track_id: &str,
    variant: &str,
    cache_dir: &Path,
) -> Result<HlsSegments, HlsError> {
    generate_hls_with_options(
        source_path,
        track_id,
        variant,
        cache_dir,
        Duration::from_secs(DEFAULT_SEGMENT_DURATION_SECS),
    )
}

fn generate_hls_with_options(
    source_path: &Path,
    track_id: &str,
    variant: &str,
    cache_dir: &Path,
    segment_duration: Duration,
) -> Result<HlsSegments, HlsError> {
    std::fs::create_dir_all(cache_dir)?;
    let target_dir = cache_dir.join(track_id).join(variant);
    let tmp_dir = cache_dir.join(track_id).join(format!(
        ".{variant}.tmp-{}-{}",
        std::process::id(),
        current_unix_secs()
    ));

    if tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    std::fs::create_dir_all(&tmp_dir)?;

    let remux = remux_source(source_path, segment_duration)?;
    write_hls_output(&tmp_dir, &remux, segment_duration)?;

    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)?;
    }
    if let Some(parent) = target_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&tmp_dir, &target_dir)?;

    HlsSegments::open(target_dir)
}

#[derive(Debug, Error)]
pub enum HlsError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("lofty error: {0}")]
    Lofty(#[from] lofty::error::LoftyError),
    #[error("mp4 mux error: {0}")]
    Mux(#[from] MuxError),
    #[error("mp4 demux error: {0}")]
    Demux(#[from] shiguredo_mp4::demux::DemuxError),
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("unsupported format: {0}")]
    Unsupported(&'static str),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("cache missing: {0}")]
    MissingCache(PathBuf),
}

#[derive(Debug, Clone)]
struct RemuxedTrack {
    timescale: NonZeroU32,
    bytes: Arc<[u8]>,
    sample_entry: Option<Arc<SampleEntry>>,
    samples: Vec<RemuxSampleRef>,
}

#[derive(Debug, Clone)]
struct RemuxSampleRef {
    range: Range<usize>,
    duration: u32,
    sample_entry: Option<Arc<SampleEntry>>,
    keyframe: bool,
    composition_time_offset: Option<i64>,
}

#[derive(Debug, Clone)]
struct SegmentChunk {
    sample_range: Range<usize>,
    duration_secs: f64,
}

#[derive(Debug, Clone)]
struct FlacStreamInfo {
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    max_block_size: u16,
}

const MAX_FMP4_AUDIO_SAMPLE_RATE: u32 = u16::MAX as u32;

struct AlacPcm16;
struct AlacPcm20;
struct AlacPcm24;
struct AlacPcm32;

impl PcmFormat for AlacPcm16 {
    fn bits() -> u32 { 16 }
    fn bytes() -> u32 { 2 }
    fn flags() -> u32 { 4 }
}

impl PcmFormat for AlacPcm20 {
    fn bits() -> u32 { 20 }
    fn bytes() -> u32 { 3 }
    fn flags() -> u32 { 4 }
}

impl PcmFormat for AlacPcm24 {
    fn bits() -> u32 { 24 }
    fn bytes() -> u32 { 3 }
    fn flags() -> u32 { 4 }
}

impl PcmFormat for AlacPcm32 {
    fn bits() -> u32 { 32 }
    fn bytes() -> u32 { 4 }
    fn flags() -> u32 { 4 }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum AlacFormatType {
    AppleLossless,
    LinearPcm,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct AlacFormatDescriptionLayout {
    sample_rate: f64,
    format_id: AlacFormatType,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
}

fn remux_source(source_path: &Path, segment_duration: Duration) -> Result<RemuxedTrack, HlsError> {
    let tagged = Probe::open(source_path)?.read()?;
    let file_type = tagged.file_type();
    let properties = tagged.properties().clone();

    match file_type {
        FileType::Flac => remux_flac(source_path, &properties),
        FileType::Mpeg => remux_mp3(source_path, &properties),
        FileType::Aac => remux_aac_adts(source_path),
        FileType::Wav => remux_wav(source_path, &properties),
        FileType::Aiff => remux_aiff(source_path, &properties),
        FileType::Opus => remux_opus(source_path, &properties),
        FileType::Mp4 => remux_mp4(source_path, segment_duration),
        _ => Err(HlsError::Unsupported("file type is not supported for HLS remux")),
    }
}

fn write_hls_output(
    target_dir: &Path,
    remuxed: &RemuxedTrack,
    segment_duration: Duration,
) -> Result<(), HlsError> {
    let chunks = split_into_segments(remuxed, segment_duration);
    if chunks.is_empty() {
        return Err(HlsError::InvalidData(
            "no HLS chunks were produced from source track".to_string(),
        ));
    }

    let mut muxer = Fmp4SegmentMuxer::new()?;
    let mut playlist_segments = Vec::with_capacity(chunks.len());

    for (index, chunk) in chunks.iter().enumerate() {
        let mut data_offset = 0u64;
        let chunk_samples = &remuxed.samples[chunk.sample_range.clone()];
        let mut mux_samples = Vec::with_capacity(chunk_samples.len());

        for (sample_index, sample) in chunk_samples.iter().enumerate() {
            let size = sample.range.end - sample.range.start;
            mux_samples.push(Sample {
                track_kind: TrackKind::Audio,
                timescale: remuxed.timescale,
                sample_entry: if sample_index == 0 {
                    Some(
                        sample
                            .sample_entry
                            .as_ref()
                            .or(remuxed.sample_entry.as_ref())
                            .ok_or_else(|| {
                                HlsError::InvalidData(
                                    "missing sample entry for media segment".to_string(),
                                )
                            })?
                            .as_ref()
                            .clone(),
                    )
                } else {
                    None
                },
                duration: sample.duration,
                keyframe: sample.keyframe,
                composition_time_offset: sample.composition_time_offset,
                data_offset,
                data_size: size,
            });
            data_offset = data_offset.saturating_add(size as u64);
        }

        let segment_meta = muxer.create_media_segment_metadata(&mux_samples)?;
        let path = target_dir.join(format!("seg{index}.m4s"));
        let mut out = BufWriter::new(std::fs::File::create(&path)?);
        out.write_all(&segment_meta)?;
        for sample in chunk_samples {
            out.write_all(&remuxed.bytes[sample.range.clone()])?;
        }
        out.flush()?;

        let mut builder = MediaSegment::builder();
        if index == 0 {
            builder.map(ExtXMap::new("init.mp4"));
        }
        builder.duration(ExtInf::new(Duration::from_secs_f64(chunk.duration_secs.max(0.001))));
        builder.uri(format!("seg{index}.m4s"));
        playlist_segments.push(
            builder
                .build()
                .map_err(|err| HlsError::InvalidData(err.to_string()))?,
        );
    }

    let init = muxer.init_segment_bytes()?;
    std::fs::write(target_dir.join("init.mp4"), init)?;

    let target_duration = Duration::from_secs(
        chunks
            .iter()
            .map(|chunk| chunk.duration_secs.ceil() as u64)
            .max()
            .unwrap_or_else(|| segment_duration.as_secs().max(1)),
    );
    let playlist = MediaPlaylist::builder()
        .target_duration(target_duration)
        .playlist_type(PlaylistType::Vod)
        .has_end_list(true)
        .has_independent_segments(true)
        .segments(playlist_segments)
        .build()
        .map_err(|err| HlsError::InvalidData(err.to_string()))?;

    std::fs::write(target_dir.join("index.m3u8"), playlist.to_string())?;
    Ok(())
}

fn split_into_segments(remuxed: &RemuxedTrack, segment_duration: Duration) -> Vec<SegmentChunk> {
    let target_duration_units = ((segment_duration.as_secs_f64() * remuxed.timescale.get() as f64)
        .round() as u64)
        .max(1);

    let mut chunks = Vec::new();
    let mut current_start = 0usize;
    let mut current_duration = 0u64;
    let mut current_len = 0usize;
    let mut current_entry: Option<&Arc<SampleEntry>> = None;

    for sample in &remuxed.samples {
        let exceeds_target = current_duration > 0
            && current_duration.saturating_add(sample.duration as u64) > target_duration_units;
        let sample_entry = sample.sample_entry.as_ref().or(remuxed.sample_entry.as_ref());
        let entry_changed = current_len > 0
            && match (current_entry, sample_entry) {
                (Some(existing), Some(next)) => !Arc::ptr_eq(existing, next),
                (None, None) => false,
                _ => true,
            };

        if current_len > 0 && (exceeds_target || entry_changed) {
            chunks.push(SegmentChunk {
                sample_range: current_start..current_start + current_len,
                duration_secs: current_duration as f64 / remuxed.timescale.get() as f64,
            });
            current_start += current_len;
            current_len = 0;
            current_duration = 0;
        }

        current_duration = current_duration.saturating_add(sample.duration as u64);
        current_len += 1;
        current_entry = sample_entry;
    }

    if current_len > 0 {
        chunks.push(SegmentChunk {
            sample_range: current_start..current_start + current_len,
            duration_secs: current_duration as f64 / remuxed.timescale.get() as f64,
        });
    }

    chunks
}

fn remux_mp4(source_path: &Path, _segment_duration: Duration) -> Result<RemuxedTrack, HlsError> {
    let file_data = std::fs::read(source_path)?;
    let bytes: Arc<[u8]> = file_data.into();
    let mut demuxer = Mp4FileDemuxer::new();
    demuxer.handle_input(Input {
        position: 0,
        data: &bytes,
    });

    let tracks = demuxer.tracks()?;
    let audio_track_id = tracks
        .iter()
        .find(|track| track.kind == TrackKind::Audio)
        .map(|track| track.track_id)
        .ok_or_else(|| HlsError::InvalidData("mp4 did not contain an audio track".to_string()))?;
    let timescale = tracks
        .iter()
        .find(|track| track.track_id == audio_track_id)
        .map(|track| track.timescale)
        .ok_or_else(|| HlsError::InvalidData("missing audio track timescale".to_string()))?;

    let mut current_entry: Option<Arc<SampleEntry>> = None;
    let mut samples = Vec::new();
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.track_id != audio_track_id {
            continue;
        }

        if let Some(entry) = sample.sample_entry {
            current_entry = Some(Arc::new(entry.clone()));
        }
        let sample_entry = current_entry.clone().ok_or_else(|| {
            HlsError::InvalidData("audio sample missing sample entry in mp4 source".to_string())
        })?;

        let start = sample.data_offset as usize;
        let end = start.saturating_add(sample.data_size);
        if end > bytes.len() {
            return Err(HlsError::InvalidData(
                "mp4 sample range exceeded source file".to_string(),
            ));
        }

        samples.push(RemuxSampleRef {
            range: start..end,
            duration: sample.duration,
            sample_entry: Some(sample_entry),
            keyframe: sample.keyframe,
            composition_time_offset: sample.composition_time_offset,
        });
    }

    if samples.is_empty() {
        return Err(HlsError::InvalidData(
            "mp4 source did not yield any audio samples".to_string(),
        ));
    }

    Ok(RemuxedTrack {
        timescale,
        bytes,
        sample_entry: None,
        samples,
    })
}

fn remux_flac(source_path: &Path, properties: &lofty::properties::FileProperties) -> Result<RemuxedTrack, HlsError> {
    let bytes = std::fs::read(source_path)?;
    if !bytes.starts_with(b"fLaC") {
        return Err(HlsError::InvalidData("flac magic missing".to_string()));
    }

    let (metadata_blocks, streaminfo, audio_offset) = parse_flac_metadata(&bytes)?;
    let timescale = nonzero(properties.sample_rate().unwrap_or(streaminfo.sample_rate))?;

    let mp4_sample_rate = match streaminfo.sample_rate {
        0..=65535 => streaminfo.sample_rate as u16,
        88200 => 44100,
        96000 => 48000,
        176400 => 58800,
        192000 => 48000,
        _ => 65535,
    };

    let dfla_blocks = rebuild_dfla_blocks(metadata_blocks);

    let entry = Arc::new(SampleEntry::Flac(FlacBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: streaminfo.channels as u16,
            samplesize: streaminfo.bits_per_sample as u16,
            samplerate: FixedPointNumber::new(mp4_sample_rate, 0),
        },
        dfla_box: DflaBox {
            metadata_blocks: dfla_blocks,
        },
        unknown_boxes: vec![],
    }));

    let frame_starts = flac_frame_starts(&bytes, audio_offset);
    if frame_starts.is_empty() {
        return Err(HlsError::InvalidData(
            "no FLAC frames were found in source file".to_string(),
        ));
    }

    let mut samples = Vec::with_capacity(frame_starts.len());
    for (idx, start) in frame_starts.iter().enumerate() {
        let end = frame_starts.get(idx + 1).copied().unwrap_or(bytes.len());
        if end <= *start {
            continue;
        }

        let duration = parse_flac_frame_block_size(&bytes[*start..], &streaminfo)
            .unwrap_or(streaminfo.max_block_size.max(1) as u32);
        samples.push(RemuxSampleRef {
            range: *start..end,
            duration: duration.max(1),
            sample_entry: None,
            keyframe: true,
            composition_time_offset: None,
        });
    }

    Ok(RemuxedTrack {
        timescale,
        bytes: bytes.into(),
        sample_entry: Some(entry),
        samples,
    })
}

fn remux_mp3(source_path: &Path, properties: &lofty::properties::FileProperties) -> Result<RemuxedTrack, HlsError> {
    let bytes = std::fs::read(source_path)?;
    let mut offset = skip_id3v2(&bytes);
    let sample_rate = properties
        .sample_rate()
        .ok_or_else(|| HlsError::InvalidData("missing MP3 sample rate".to_string()))?;
    let channels = properties.channels().unwrap_or(2);
    let timescale = nonzero(sample_rate)?;
    let entry = Arc::new(unknown_audio_sample_entry(
        *b".mp3",
        sample_rate,
        channels,
        properties.bit_depth().unwrap_or(16),
        Vec::new(),
    )?);

    let mut samples = Vec::new();
    while offset + 4 <= bytes.len() {
        if let Some(frame) = parse_mp3_frame(&bytes[offset..]) {
            let end = offset.saturating_add(frame.frame_length);
            if end > bytes.len() {
                break;
            }
            samples.push(RemuxSampleRef {
                range: offset..end,
                duration: frame.samples_per_frame,
                sample_entry: None,
                keyframe: true,
                composition_time_offset: None,
            });
            offset = end;
        } else {
            offset += 1;
        }
    }

    if samples.is_empty() {
        return Err(HlsError::InvalidData("no MP3 frames found".to_string()));
    }

    Ok(RemuxedTrack {
        timescale,
        bytes: bytes.into(),
        sample_entry: Some(entry),
        samples,
    })
}

fn remux_aac_adts(source_path: &Path) -> Result<RemuxedTrack, HlsError> {
    let bytes = std::fs::read(source_path)?;
    let mut offset = 0usize;
    let mut samples = Vec::new();
    let mut sample_rate = None;
    let mut channels = None;
    let mut audio_object_type = None;
    let mut current_entry_key = None;
    let mut current_entry: Option<Arc<SampleEntry>> = None;

    while offset + 7 <= bytes.len() {
        let Some(frame) = parse_adts_frame(&bytes[offset..]) else {
            offset += 1;
            continue;
        };
        let end = offset.saturating_add(frame.frame_length);
        if end > bytes.len() || frame.frame_length <= frame.header_len {
            break;
        }

        sample_rate = Some(frame.sample_rate);
        channels = Some(frame.channel_config);
        audio_object_type = Some(frame.audio_object_type);
        let entry_key = (
            frame.sample_rate,
            frame.channel_config,
            frame.audio_object_type,
            frame.sampling_frequency_index,
        );
        if current_entry_key != Some(entry_key) {
            current_entry = Some(Arc::new(SampleEntry::Mp4a(mp4a_entry(
                frame.sample_rate,
                frame.channel_config,
                16,
                aac_audio_specific_config(
                    frame.audio_object_type,
                    frame.sampling_frequency_index,
                    frame.channel_config,
                )
                .to_vec(),
            )?)));
            current_entry_key = Some(entry_key);
        }

        samples.push(RemuxSampleRef {
            range: offset + frame.header_len..end,
            duration: frame.samples_per_frame,
            sample_entry: current_entry.clone(),
            keyframe: true,
            composition_time_offset: None,
        });
        offset = end;
    }

    let sample_rate = sample_rate
        .ok_or_else(|| HlsError::InvalidData("no AAC ADTS frames found".to_string()))?;
    let _channels = channels.unwrap_or(2);
    let _audio_object_type = audio_object_type.unwrap_or(2);
    Ok(RemuxedTrack {
        timescale: nonzero(sample_rate)?,
        bytes: bytes.into(),
        sample_entry: None,
        samples,
    })
}

fn remux_wav(source_path: &Path, properties: &lofty::properties::FileProperties) -> Result<RemuxedTrack, HlsError> {
    let bytes = std::fs::read(source_path)?;
    let wav = parse_wav_pcm(&bytes)?;
    let sample_rate = properties.sample_rate().unwrap_or(wav.sample_rate);
    let channels = properties.channels().unwrap_or(wav.channels);
    let bit_depth = properties.bit_depth().unwrap_or(wav.bits_per_sample);
    if sample_rate > MAX_FMP4_AUDIO_SAMPLE_RATE {
        return remux_pcm_as_alac(
            &bytes[wav.data_offset..wav.data_offset + wav.data_len],
            sample_rate,
            channels,
            bit_depth,
            false,
        );
    }
    let timescale = nonzero(sample_rate)?;
    let entry = Arc::new(unknown_audio_sample_entry(*b"lpcm", sample_rate, channels, bit_depth, Vec::new())?);
    let bytes_per_frame = (u32::from(channels) * u32::from(bit_depth) / 8).max(1) as usize;

    Ok(RemuxedTrack {
        timescale,
        bytes: bytes.into(),
        sample_entry: Some(entry),
        samples: pcm_chunks(
            wav.data_offset..wav.data_offset + wav.data_len,
            bytes_per_frame,
            sample_rate,
            Duration::from_secs(DEFAULT_SEGMENT_DURATION_SECS),
            None,
        ),
    })
}

fn remux_aiff(source_path: &Path, properties: &lofty::properties::FileProperties) -> Result<RemuxedTrack, HlsError> {
    let bytes = std::fs::read(source_path)?;
    let aiff = parse_aiff_pcm(&bytes)?;
    let sample_rate = properties.sample_rate().unwrap_or(aiff.sample_rate);
    let channels = properties.channels().unwrap_or(aiff.channels);
    let bit_depth = properties.bit_depth().unwrap_or(aiff.bits_per_sample);
    if sample_rate > MAX_FMP4_AUDIO_SAMPLE_RATE {
        return remux_pcm_as_alac(
            &bytes[aiff.data_offset..aiff.data_offset + aiff.data_len],
            sample_rate,
            channels,
            bit_depth,
            true,
        );
    }
    let timescale = nonzero(sample_rate)?;
    let entry = Arc::new(unknown_audio_sample_entry(*b"lpcm", sample_rate, channels, bit_depth, Vec::new())?);
    let bytes_per_frame = (u32::from(channels) * u32::from(bit_depth) / 8).max(1) as usize;

    Ok(RemuxedTrack {
        timescale,
        bytes: bytes.into(),
        sample_entry: Some(entry),
        samples: pcm_chunks(
            aiff.data_offset..aiff.data_offset + aiff.data_len,
            bytes_per_frame,
            sample_rate,
            Duration::from_secs(DEFAULT_SEGMENT_DURATION_SECS),
            None,
        ),
    })
}

fn remux_opus(source_path: &Path, properties: &lofty::properties::FileProperties) -> Result<RemuxedTrack, HlsError> {
    let opus = parse_ogg_opus_packets(&std::fs::read(source_path)?)?;
    let sample_rate = 48_000u32;
    let channels = properties.channels().unwrap_or(opus.channels);
    let entry = Arc::new(SampleEntry::Opus(OpusBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: channels as u16,
            samplesize: properties.bit_depth().unwrap_or(16) as u16,
            samplerate: FixedPointNumber::new(48_000, 0),
        },
        dops_box: DopsBox {
            output_channel_count: channels,
            pre_skip: opus.pre_skip,
            input_sample_rate: sample_rate,
            output_gain: 0,
        },
        unknown_boxes: vec![],
    }));

    let mut payload = Vec::new();
    let mut samples = Vec::with_capacity(opus.packets.len());
    for packet in opus.packets {
        let start = payload.len();
        let duration = opus_packet_duration(&packet).max(1);
        payload.extend_from_slice(&packet);
        let end = payload.len();
        samples.push(RemuxSampleRef {
            duration,
            range: start..end,
            sample_entry: None,
            keyframe: true,
            composition_time_offset: None,
        });
    }

    if samples.is_empty() {
        return Err(HlsError::InvalidData("no Opus packets found".to_string()));
    }

    Ok(RemuxedTrack {
        timescale: nonzero(sample_rate)?,
        bytes: payload.into(),
        sample_entry: Some(entry),
        samples,
    })
}

fn pcm_chunks(
    data_range: Range<usize>,
    bytes_per_frame: usize,
    sample_rate: u32,
    segment_duration: Duration,
    entry: Option<Arc<SampleEntry>>,
) -> Vec<RemuxSampleRef> {
    let frames_per_chunk = (sample_rate as u64 * segment_duration.as_secs().max(1)) as usize;
    let chunk_size = frames_per_chunk.saturating_mul(bytes_per_frame).max(bytes_per_frame);
    let mut offset = data_range.start;
    let mut samples = Vec::new();

    while offset < data_range.end {
        let next_end = (offset + chunk_size).min(data_range.end);
        let aligned_end = next_end - ((next_end - offset) % bytes_per_frame);
        let end = if aligned_end == offset { data_range.end } else { aligned_end };
        let frame_count = ((end - offset) / bytes_per_frame).max(1) as u32;
        samples.push(RemuxSampleRef {
            range: offset..end,
            duration: frame_count,
            sample_entry: entry.clone(),
            keyframe: true,
            composition_time_offset: None,
        });
        offset = end;
    }

    samples
}

fn remux_pcm_as_alac(
    pcm_bytes: &[u8],
    sample_rate: u32,
    channels: u8,
    bit_depth: u8,
    source_big_endian: bool,
) -> Result<RemuxedTrack, HlsError> {
    let channels_u32 = u32::from(channels);
    let bytes_per_sample = pcm_bytes_per_sample(bit_depth)?;
    let bytes_per_frame = bytes_per_sample.saturating_mul(channels as usize);
    if bytes_per_frame == 0 {
        return Err(HlsError::InvalidData("PCM frame size must be non-zero".to_string()));
    }
    if pcm_bytes.len() % bytes_per_frame != 0 {
        return Err(HlsError::InvalidData(
            "PCM payload was not aligned to whole audio frames".to_string(),
        ));
    }

    let normalized_pcm = pcm_to_alac_bytes(pcm_bytes, bit_depth, source_big_endian)?;
    let input_format = alac_pcm_input_format(sample_rate, channels_u32, bit_depth)?;
    let output_format = alac_output_format(sample_rate, DEFAULT_FRAMES_PER_PACKET, channels_u32, bit_depth);
    let mut encoder = AlacEncoder::new(&output_format);
    let magic_cookie = encoder.magic_cookie();
    let mut alac_inner_box = Vec::with_capacity(12 + magic_cookie.len());
    let inner_size = (12 + magic_cookie.len()) as u32;
    alac_inner_box.extend_from_slice(&inner_size.to_be_bytes());
    alac_inner_box.extend_from_slice(b"alac");
    alac_inner_box.extend_from_slice(&0u32.to_be_bytes());
    alac_inner_box.extend_from_slice(&magic_cookie);
    let sample_entry = Arc::new(unknown_audio_sample_entry(
        *b"alac",
        sample_rate,
        channels,
        bit_depth,
        alac_inner_box,
    )?);

    let mut packet_buffer = vec![0u8; output_format.max_packet_size()];
    let chunk_bytes = DEFAULT_FRAMES_PER_PACKET as usize * bytes_per_frame;
    let mut payload = Vec::new();
    let mut samples = Vec::new();

    for pcm_chunk in normalized_pcm.chunks(chunk_bytes) {
        let frame_count = pcm_chunk.len() / bytes_per_frame;
        if frame_count == 0 {
            continue;
        }

        let start = payload.len();
        let encoded_size = encoder.encode(&input_format, pcm_chunk, &mut packet_buffer);
        payload.extend_from_slice(&packet_buffer[..encoded_size]);
        let end = payload.len();
        samples.push(RemuxSampleRef {
            range: start..end,
            duration: frame_count as u32,
            sample_entry: None,
            keyframe: true,
            composition_time_offset: None,
        });
    }

    if samples.is_empty() {
        return Err(HlsError::InvalidData("no ALAC packets were produced from PCM input".to_string()));
    }

    Ok(RemuxedTrack {
        timescale: nonzero(sample_rate)?,
        bytes: payload.into(),
        sample_entry: Some(sample_entry),
        samples,
    })
}

fn alac_pcm_input_format(
    sample_rate: u32,
    channels: u32,
    bit_depth: u8,
) -> Result<FormatDescription, HlsError> {
    Ok(match bit_depth {
        16 => FormatDescription::pcm::<AlacPcm16>(sample_rate as f64, channels),
        20 => FormatDescription::pcm::<AlacPcm20>(sample_rate as f64, channels),
        24 => FormatDescription::pcm::<AlacPcm24>(sample_rate as f64, channels),
        32 => FormatDescription::pcm::<AlacPcm32>(sample_rate as f64, channels),
        _ => {
            return Err(HlsError::Unsupported(
                "ALAC remux only supports 16/20/24/32-bit PCM input",
            ));
        }
    })
}

fn pcm_bytes_per_sample(bit_depth: u8) -> Result<usize, HlsError> {
    match bit_depth {
        16 => Ok(2),
        20 | 24 => Ok(3),
        32 => Ok(4),
        _ => Err(HlsError::Unsupported(
            "ALAC remux only supports 16/20/24/32-bit PCM input",
        )),
    }
}

fn pcm_to_alac_bytes<'a>(
    pcm_bytes: &'a [u8],
    bit_depth: u8,
    source_big_endian: bool,
) -> Result<Cow<'a, [u8]>, HlsError> {
    let bytes_per_sample = pcm_bytes_per_sample(bit_depth)?;
    let target_big_endian = matches!(bit_depth, 20 | 24);
    if source_big_endian == target_big_endian {
        return Ok(Cow::Borrowed(pcm_bytes));
    }

    let mut converted = Vec::with_capacity(pcm_bytes.len());
    for sample in pcm_bytes.chunks_exact(bytes_per_sample) {
        for byte in sample.iter().rev() {
            converted.push(*byte);
        }
    }
    Ok(Cow::Owned(converted))
}

fn alac_output_format(
    sample_rate: u32,
    frames_per_packet: u32,
    channels: u32,
    bit_depth: u8,
) -> FormatDescription {
    const _: () = assert!(
        std::mem::size_of::<AlacFormatDescriptionLayout>() == std::mem::size_of::<FormatDescription>(),
        "AlacFormatDescriptionLayout must match FormatDescription size"
    );

    let layout = AlacFormatDescriptionLayout {
        sample_rate: sample_rate as f64,
        format_id: AlacFormatType::AppleLossless,
        bytes_per_packet: 0,
        frames_per_packet,
        channels_per_frame: channels,
        bits_per_channel: u32::from(bit_depth),
    };

    // Safety: AlacFormatDescriptionLayout and FormatDescription have identical
    // field types in identical order, both using the default Rust representation.
    // We verify size equality at compile time above.
    let fmt: FormatDescription = unsafe { std::mem::transmute(layout) };

    // Runtime safety check: verify the transmute preserved the field values.
    // If this fails, the Rust compiler has given the two structs different layouts
    // and we cannot safely transmute between them.
    let fmt_bytes = unsafe {
        std::slice::from_raw_parts(
            &fmt as *const FormatDescription as *const u8,
            std::mem::size_of::<FormatDescription>(),
        )
    };
    let layout_bytes = unsafe {
        std::slice::from_raw_parts(
            &layout as *const AlacFormatDescriptionLayout as *const u8,
            std::mem::size_of::<AlacFormatDescriptionLayout>(),
        )
    };
    assert_eq!(
        fmt_bytes, layout_bytes,
        "transmute round-trip failed: layout mismatch between AlacFormatDescriptionLayout and FormatDescription"
    );

    fmt
}

fn mp4a_entry(
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u16,
    decoder_specific_payload: Vec<u8>,
) -> Result<Mp4aBox, HlsError> {
    Ok(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: channels as u16,
            samplesize: bits_per_sample,
            samplerate: FixedPointNumber::new(u16::try_from(sample_rate).unwrap_or(u16::MAX), 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    buffer_size_db: Uint::new(0),
                    max_bitrate: 0,
                    avg_bitrate: 0,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: decoder_specific_payload,
                    }),
                },
                sl_config_descr: SlConfigDescriptor,
            },
        },
        unknown_boxes: vec![],
    })
}

fn unknown_audio_sample_entry(
    box_type: [u8; 4],
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    trailing_payload: Vec<u8>,
) -> Result<SampleEntry, HlsError> {
    let audio = AudioSampleEntryFields {
        data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
        channelcount: channels as u16,
        samplesize: bits_per_sample as u16,
        samplerate: FixedPointNumber::new(u16::try_from(sample_rate).unwrap_or(u16::MAX), 0),
    };
    let mut payload = encode_audio_sample_entry_fields(&audio)?;
    payload.extend_from_slice(&trailing_payload);
    Ok(SampleEntry::Unknown(UnknownBox {
        box_type: BoxType::Normal(box_type),
        box_size: BoxSize::U32((8 + payload.len()) as u32),
        payload,
    }))
}

fn encode_audio_sample_entry_fields(audio: &AudioSampleEntryFields) -> Result<Vec<u8>, HlsError> {
    let mut buf = vec![0u8; 256];
    let len = shiguredo_mp4::Encode::encode(audio, &mut buf)
        .map_err(|err| HlsError::InvalidData(err.to_string()))?;
    buf.truncate(len);
    Ok(buf)
}

fn aac_audio_specific_config(
    audio_object_type: u8,
    sample_rate_index: u8,
    channel_config: u8,
) -> [u8; 2] {
    let byte0 = (audio_object_type << 3) | ((sample_rate_index & 0x0E) >> 1);
    let byte1 = ((sample_rate_index & 0x01) << 7) | ((channel_config & 0x0F) << 3);
    [byte0, byte1]
}

fn rebuild_dfla_blocks(mut blocks: Vec<FlacMetadataBlock>) -> Vec<FlacMetadataBlock> {
    if blocks.is_empty() {
        return blocks;
    }
    for block in &mut blocks {
        block.last_metadata_block_flag = Uint::new(0);
    }
    if let Some(last) = blocks.last_mut() {
        last.last_metadata_block_flag = Uint::new(1);
    }
    blocks
}

fn parse_flac_metadata(bytes: &[u8]) -> Result<(Vec<FlacMetadataBlock>, FlacStreamInfo, usize), HlsError> {
    let mut offset = 4usize;
    let mut blocks = Vec::new();
    let mut streaminfo = None;

    loop {
        if offset + 4 > bytes.len() {
            return Err(HlsError::InvalidData(
                "unexpected end of FLAC metadata blocks".to_string(),
            ));
        }
        let header = bytes[offset];
        let is_last = (header & 0x80) != 0;
        let block_type = header & 0x7F;
        let length = ((bytes[offset + 1] as usize) << 16)
            | ((bytes[offset + 2] as usize) << 8)
            | bytes[offset + 3] as usize;
        offset += 4;
        let end = offset.saturating_add(length);
        if end > bytes.len() {
            return Err(HlsError::InvalidData(
                "FLAC metadata block exceeded file bounds".to_string(),
            ));
        }

        let block_data = bytes[offset..end].to_vec();
        if block_type == 0 {
            streaminfo = Some(parse_flac_streaminfo(&block_data)?);
        }
        blocks.push(FlacMetadataBlock {
            last_metadata_block_flag: Uint::new(is_last as u8),
            block_type: Uint::new(block_type),
            block_data,
        });
        offset = end;
        if is_last {
            break;
        }
    }

    Ok((
        blocks,
        streaminfo.ok_or_else(|| {
            HlsError::InvalidData("FLAC streaminfo block missing".to_string())
        })?,
        offset,
    ))
}

fn parse_flac_streaminfo(block: &[u8]) -> Result<FlacStreamInfo, HlsError> {
    if block.len() != 34 {
        return Err(HlsError::InvalidData(
            "FLAC STREAMINFO block must be 34 bytes".to_string(),
        ));
    }

    let max_block_size = u16::from_be_bytes([block[2], block[3]]);
    let sample_rate = ((block[10] as u32) << 12)
        | ((block[11] as u32) << 4)
        | ((block[12] as u32 & 0xF0) >> 4);
    let channels = ((block[12] & 0x0E) >> 1) + 1;
    let bits_per_sample = (((block[12] & 0x01) << 4) | ((block[13] & 0xF0) >> 4)) + 1;

    Ok(FlacStreamInfo {
        sample_rate,
        channels,
        bits_per_sample,
        max_block_size,
    })
}

fn flac_frame_starts(bytes: &[u8], audio_offset: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = audio_offset;
    while offset + 1 < bytes.len() {
        if bytes[offset] == 0xFF && (bytes[offset + 1] & 0xFC) == 0xF8 {
            if validate_flac_frame_header_crc(bytes, offset) {
                starts.push(offset);
            }
            offset += 2;
        } else {
            offset += 1;
        }
    }
    starts
}

fn validate_flac_frame_header_crc(bytes: &[u8], offset: usize) -> bool {
    let frame_header_end = match find_flac_frame_header_end(bytes, offset) {
        Some(end) => end,
        None => return false,
    };
    if frame_header_end >= bytes.len() {
        return false;
    }
    let expected_crc = bytes[frame_header_end];
    let computed_crc = flac_crc8(&bytes[offset..frame_header_end]);
    computed_crc == expected_crc
}

fn find_flac_frame_header_end(bytes: &[u8], offset: usize) -> Option<usize> {
    if offset + 4 > bytes.len() {
        return None;
    }
    let mut pos = offset + 2;
    let block_size_code = (bytes[pos] & 0xF0) >> 4;
    let sample_rate_code = bytes[pos] & 0x0F;
    pos += 1;

    pos += 1;

    let utf8_len = skip_flac_utf8_number(bytes.get(pos..)?)?;
    pos += utf8_len;

    match block_size_code {
        6 => pos += 1,
        7 => pos += 2,
        _ => {}
    }
    match sample_rate_code {
        12 => pos += 1,
        13 | 14 => pos += 2,
        _ => {}
    }
    Some(pos)
}

fn flac_crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn parse_flac_frame_block_size(frame: &[u8], streaminfo: &FlacStreamInfo) -> Option<u32> {
    if frame.len() < 5 || frame[0] != 0xFF || (frame[1] & 0xFC) != 0xF8 {
        return None;
    }

    let block_size_code = (frame[2] & 0xF0) >> 4;
    let sample_rate_code = frame[2] & 0x0F;
    let mut offset = 4usize;
    offset += skip_flac_utf8_number(&frame[offset..])?;

    let block_size = match block_size_code {
        0 => streaminfo.max_block_size as u32,
        1 => 192,
        2 => 576,
        3 => 1152,
        4 => 2304,
        5 => 4608,
        6 => {
            let value = *frame.get(offset)? as u32 + 1;
            offset += 1;
            value
        }
        7 => {
            let value = u16::from_be_bytes([*frame.get(offset)?, *frame.get(offset + 1)?]) as u32 + 1;
            offset += 2;
            value
        }
        8 => 256,
        9 => 512,
        10 => 1024,
        11 => 2048,
        12 => 4096,
        13 => 8192,
        14 => 16384,
        15 => 32768,
        _ => return None,
    };

    match sample_rate_code {
        12 => {
            offset += 1;
        }
        13 | 14 => {
            offset += 2;
        }
        _ => {}
    }
    let _ = frame.get(offset)?;
    Some(block_size)
}

fn skip_flac_utf8_number(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.first()?;
    let len = if first & 0x80 == 0 {
        1
    } else if first & 0xE0 == 0xC0 {
        2
    } else if first & 0xF0 == 0xE0 {
        3
    } else if first & 0xF8 == 0xF0 {
        4
    } else if first & 0xFC == 0xF8 {
        5
    } else if first & 0xFE == 0xFC {
        6
    } else if first == 0xFE {
        7
    } else {
        return None;
    };
    (bytes.len() >= len).then_some(len)
}

#[derive(Debug, Clone, Copy)]
struct Mp3FrameInfo {
    frame_length: usize,
    samples_per_frame: u32,
}

fn parse_mp3_frame(bytes: &[u8]) -> Option<Mp3FrameInfo> {
    if bytes.len() < 4 || bytes[0] != 0xFF || (bytes[1] & 0xE0) != 0xE0 {
        return None;
    }

    let version_id = (bytes[1] >> 3) & 0x03;
    let layer = (bytes[1] >> 1) & 0x03;
    let bitrate_index = (bytes[2] >> 4) & 0x0F;
    let sample_rate_index = (bytes[2] >> 2) & 0x03;
    let padding = ((bytes[2] >> 1) & 0x01) as usize;

    if version_id == 1 || layer == 0 || bitrate_index == 0 || bitrate_index == 0x0F || sample_rate_index == 0x03 {
        return None;
    }

    let is_mpeg1 = version_id == 3;
    let sample_rate_table = match version_id {
        0 => [11025, 12000, 8000],
        2 => [22050, 24000, 16000],
        3 => [44100, 48000, 32000],
        _ => return None,
    };
    let sample_rate = sample_rate_table[sample_rate_index as usize];

    let layer_index = match layer {
        3 => 0,
        2 => 1,
        1 => 2,
        _ => return None,
    };
    let bitrate_kbps = if is_mpeg1 {
        [
            [32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
            [32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
            [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
        ][layer_index][bitrate_index as usize - 1]
    } else {
        [
            [32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
            [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
            [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        ][layer_index][bitrate_index as usize - 1]
    };
    let bitrate = bitrate_kbps * 1000;

    let (samples_per_frame, frame_length) = match layer {
        3 => (384, ((12 * bitrate / sample_rate) as usize + padding) * 4),
        2 => (1152, ((144 * bitrate / sample_rate) as usize) + padding),
        1 if is_mpeg1 => (1152, ((144 * bitrate / sample_rate) as usize) + padding),
        1 => (576, ((72 * bitrate / sample_rate) as usize) + padding),
        _ => return None,
    };

    Some(Mp3FrameInfo {
        frame_length,
        samples_per_frame,
    })
}

fn skip_id3v2(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = ((bytes[6] as usize) << 21)
        | ((bytes[7] as usize) << 14)
        | ((bytes[8] as usize) << 7)
        | (bytes[9] as usize);
    10 + size
}

#[derive(Debug, Clone, Copy)]
struct AdtsFrameInfo {
    frame_length: usize,
    header_len: usize,
    sample_rate: u32,
    sampling_frequency_index: u8,
    channel_config: u8,
    audio_object_type: u8,
    samples_per_frame: u32,
}

fn parse_adts_frame(bytes: &[u8]) -> Option<AdtsFrameInfo> {
    if bytes.len() < 7 || bytes[0] != 0xFF || (bytes[1] & 0xF0) != 0xF0 {
        return None;
    }
    let has_crc = (bytes[1] & 0x01) == 0;
    let audio_object_type = ((bytes[2] >> 6) & 0x03) + 1;
    let sampling_frequency_index = (bytes[2] >> 2) & 0x0F;
    let sample_rate = match sampling_frequency_index {
        0 => 96000,
        1 => 88200,
        2 => 64000,
        3 => 48000,
        4 => 44100,
        5 => 32000,
        6 => 24000,
        7 => 22050,
        8 => 16000,
        9 => 12000,
        10 => 11025,
        11 => 8000,
        12 => 7350,
        _ => return None,
    };
    let channel_config = ((bytes[2] & 0x01) << 2) | ((bytes[3] >> 6) & 0x03);
    let frame_length = (((bytes[3] & 0x03) as usize) << 11)
        | ((bytes[4] as usize) << 3)
        | ((bytes[5] as usize & 0xE0) >> 5);
    let raw_blocks = bytes[6] & 0x03;
    Some(AdtsFrameInfo {
        frame_length,
        header_len: if has_crc { 9 } else { 7 },
        sample_rate,
        sampling_frequency_index,
        channel_config,
        audio_object_type,
        samples_per_frame: 1024 * (u32::from(raw_blocks) + 1),
    })
}

#[derive(Debug, Clone, Copy)]
struct WavInfo {
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    data_offset: usize,
    data_len: usize,
}

fn parse_wav_pcm(bytes: &[u8]) -> Result<WavInfo, HlsError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(HlsError::InvalidData("not a RIFF/WAVE file".to_string()));
    }

    let mut offset = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits_per_sample = None;
    let mut data = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let chunk_end = offset.saturating_add(chunk_size);
        if chunk_end > bytes.len() {
            break;
        }

        match chunk_id {
            b"fmt " if chunk_size >= 16 => {
                channels = Some(u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()) as u8);
                sample_rate = Some(u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()));
                bits_per_sample = Some(u16::from_le_bytes(bytes[offset + 14..offset + 16].try_into().unwrap()) as u8);
            }
            b"data" => {
                data = Some((offset, chunk_size));
            }
            _ => {}
        }

        offset = chunk_end + (chunk_size % 2);
    }

    let (data_offset, data_len) = data.ok_or_else(|| HlsError::InvalidData("WAV data chunk missing".to_string()))?;
    Ok(WavInfo {
        sample_rate: sample_rate.ok_or_else(|| HlsError::InvalidData("WAV sample rate missing".to_string()))?,
        channels: channels.ok_or_else(|| HlsError::InvalidData("WAV channels missing".to_string()))?,
        bits_per_sample: bits_per_sample.unwrap_or(16),
        data_offset,
        data_len,
    })
}

#[derive(Debug, Clone, Copy)]
struct AiffInfo {
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    data_offset: usize,
    data_len: usize,
}

fn parse_aiff_pcm(bytes: &[u8]) -> Result<AiffInfo, HlsError> {
    if bytes.len() < 12 || &bytes[..4] != b"FORM" {
        return Err(HlsError::InvalidData("not an AIFF file".to_string()));
    }
    let form_type = &bytes[8..12];
    if form_type != b"AIFF" && form_type != b"AIFC" {
        return Err(HlsError::InvalidData("unsupported AIFF form type".to_string()));
    }

    let mut offset = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits_per_sample = None;
    let mut data = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let chunk_end = offset.saturating_add(chunk_size);
        if chunk_end > bytes.len() {
            break;
        }

        match chunk_id {
            b"COMM" if chunk_size >= 18 => {
                channels = Some(u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap()) as u8);
                bits_per_sample = Some(u16::from_be_bytes(bytes[offset + 6..offset + 8].try_into().unwrap()) as u8);
                sample_rate = Some(parse_extended_f80(&bytes[offset + 8..offset + 18])? as u32);
            }
            b"SSND" if chunk_size >= 8 => {
                let data_offset_in_chunk = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                let payload_offset = offset + 8 + data_offset_in_chunk;
                let payload_len = chunk_size.saturating_sub(8 + data_offset_in_chunk);
                data = Some((payload_offset, payload_len));
            }
            _ => {}
        }

        offset = chunk_end + (chunk_size % 2);
    }

    let (data_offset, data_len) = data.ok_or_else(|| HlsError::InvalidData("AIFF SSND chunk missing".to_string()))?;
    Ok(AiffInfo {
        sample_rate: sample_rate.ok_or_else(|| HlsError::InvalidData("AIFF sample rate missing".to_string()))?,
        channels: channels.ok_or_else(|| HlsError::InvalidData("AIFF channels missing".to_string()))?,
        bits_per_sample: bits_per_sample.unwrap_or(16),
        data_offset,
        data_len,
    })
}

fn parse_extended_f80(bytes: &[u8]) -> Result<f64, HlsError> {
    if bytes.len() != 10 {
        return Err(HlsError::InvalidData("invalid AIFF extended float".to_string()));
    }
    let exponent = u16::from_be_bytes([bytes[0], bytes[1]]);
    if exponent == 0 && bytes[2..].iter().all(|byte| *byte == 0) {
        return Ok(0.0);
    }
    let sign = if (exponent & 0x8000) != 0 { -1.0 } else { 1.0 };
    let exponent = ((exponent & 0x7FFF) as i32) - 16383;
    let mantissa = u64::from_be_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    Ok(sign * (mantissa as f64) * 2f64.powi(exponent - 63))
}

#[derive(Debug)]
struct OggOpusData {
    channels: u8,
    pre_skip: u16,
    packets: Vec<Vec<u8>>,
}

fn parse_ogg_opus_packets(bytes: &[u8]) -> Result<OggOpusData, HlsError> {
    let mut offset = 0usize;
    let mut packets = Vec::new();
    let mut current_packet = Vec::new();
    let mut opus_head = None;

    while offset + 27 <= bytes.len() {
        if &bytes[offset..offset + 4] != b"OggS" {
            return Err(HlsError::InvalidData("invalid Ogg page header".to_string()));
        }
        let page_segments = bytes[offset + 26] as usize;
        let lace_start = offset + 27;
        let data_start = lace_start + page_segments;
        if data_start > bytes.len() {
            return Err(HlsError::InvalidData("invalid Ogg lacing table".to_string()));
        }
        let total = bytes[lace_start..data_start].iter().map(|v| *v as usize).sum::<usize>();
        let data_end = data_start + total;
        if data_end > bytes.len() {
            return Err(HlsError::InvalidData("invalid Ogg page size".to_string()));
        }

        let mut page_offset = data_start;
        for lace in &bytes[lace_start..data_start] {
            let seg_end = page_offset + *lace as usize;
            current_packet.extend_from_slice(&bytes[page_offset..seg_end]);
            page_offset = seg_end;
            if *lace < 255 {
                packets.push(std::mem::take(&mut current_packet));
            }
        }
        offset = data_end;
    }

    if !current_packet.is_empty() {
        packets.push(current_packet);
    }

    let mut audio_packets = Vec::new();
    let mut channels = 2u8;
    let mut pre_skip = 312u16;
    for packet in packets {
        if packet.starts_with(b"OpusHead") && packet.len() >= 19 {
            channels = packet[9];
            pre_skip = u16::from_le_bytes([packet[10], packet[11]]);
            opus_head = Some(());
        } else if packet.starts_with(b"OpusTags") {
            continue;
        } else {
            audio_packets.push(packet);
        }
    }

    if opus_head.is_none() {
        return Err(HlsError::InvalidData("OpusHead packet missing".to_string()));
    }

    Ok(OggOpusData {
        channels,
        pre_skip,
        packets: audio_packets,
    })
}

fn opus_packet_duration(packet: &[u8]) -> u32 {
    let Some(&toc) = packet.first() else {
        return 960;
    };
    let config = toc >> 3;
    let frames = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        3 => packet.get(1).map(|v| (v & 0x3F).max(1)).unwrap_or(1),
        _ => 1,
    } as u32;

    let samples_per_frame = match config {
        0..=3 => 480,
        4..=7 => 960,
        8..=11 => 1920,
        12..=15 => 2880,
        16..=19 => 120,
        20..=23 => 240,
        24..=27 => 480,
        _ => 960,
    };
    samples_per_frame * frames
}

fn nonzero(value: u32) -> Result<NonZeroU32, HlsError> {
    NonZeroU32::new(value)
        .ok_or_else(|| HlsError::InvalidData("timescale must be non-zero".to_string()))
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
struct CacheEntryInfo {
    root: PathBuf,
    size: u64,
    last_access: u64,
}

fn enforce_size_limit_blocking(
    root: &Path,
    max_size_bytes: u64,
    preserve: Option<&Path>,
) -> Result<(), HlsError> {
    let mut entries = collect_cache_entries(root)?;
    let mut total_size = entries.iter().map(|entry| entry.size).sum::<u64>();
    if total_size <= max_size_bytes {
        return Ok(());
    }

    entries.sort_by_key(|entry| entry.last_access);
    for entry in entries {
        if total_size <= max_size_bytes {
            break;
        }
        if preserve.is_some_and(|path| path == entry.root) {
            continue;
        }
        if entry.root.exists() {
            std::fs::remove_dir_all(&entry.root)?;
        }
        total_size = total_size.saturating_sub(entry.size);
    }

    Ok(())
}

fn collect_cache_entries(root: &Path) -> Result<Vec<CacheEntryInfo>, HlsError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for track_dir in std::fs::read_dir(root)? {
        let track_dir = track_dir?;
        if !track_dir.file_type()?.is_dir() {
            continue;
        }
        for variant_dir in std::fs::read_dir(track_dir.path())? {
            let variant_dir = variant_dir?;
            if !variant_dir.file_type()?.is_dir() {
                continue;
            }
            let root = variant_dir.path();
            if !root.join("index.m3u8").exists() {
                continue;
            }
            let size = dir_size(&root)?;
            let stamp = root.join(".last_access");
            let last_access = std::fs::metadata(&stamp)
                .and_then(|meta| meta.modified())
                .or_else(|_| std::fs::metadata(&root).and_then(|meta| meta.modified()))
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            entries.push(CacheEntryInfo {
                root,
                size,
                last_access,
            });
        }
    }
    Ok(entries)
}

fn dir_size(path: &Path) -> Result<u64, HlsError> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(dir_size(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_flac(dir: &Path) -> PathBuf {
        let sample_rate: u32 = 44100;
        let channels: u8 = 2;
        let bits_per_sample: u8 = 16;
        let block_size: u16 = 4096;
        let total_samples: u32 = sample_rate * 60;
        let num_frames = (total_samples + block_size as u32 - 1) / block_size as u32;

        let mut out = Vec::new();

        // fLaC magic
        out.extend_from_slice(b"fLaC");

        // STREAMINFO metadata block (header: is_last=1, type=0, length=34)
        out.push(0x80); // is_last=1, block_type=0 (STREAMINFO)
        out.extend_from_slice(&34u32.to_be_bytes()[1..]); // 24-bit length = 34

        // STREAMINFO body (34 bytes)
        let mut si = [0u8; 34];
        // min_block_size (16-bit) = block_size
        si[0..2].copy_from_slice(&block_size.to_be_bytes());
        // max_block_size (16-bit) = block_size
        si[2..4].copy_from_slice(&block_size.to_be_bytes());
        // min_frame_size (24-bit) = 0 (unknown)
        // max_frame_size (24-bit) = 0 (unknown)
        // sample_rate (20-bit) = 44100
        let sr_bits = (sample_rate << 4) as u32; // shift to align to 20-bit position
        si[10] = (sr_bits >> 24) as u8;
        si[11] = (sr_bits >> 16) as u8;
        si[12] = ((sr_bits >> 8) as u8) & 0xF0;
        // channels - 1 (3-bit) | bits_per_sample - 1 (5-bit)
        si[12] |= ((channels - 1) << 1) as u8;
        si[13] = (((channels - 1) & 0x7) << 7) as u8;
        si[12] |= ((bits_per_sample - 1) >> 1) as u8;
        si[13] |= (((bits_per_sample - 1) & 1) << 7) as u8;
        // total_samples (36-bit)
        let ts = total_samples as u64;
        si[13] |= ((ts >> 28) & 0x0F) as u8;
        si[14..18].copy_from_slice(&(ts as u32).to_be_bytes());
        // MD5 = zeros (16 bytes, already zero)
        out.extend_from_slice(&si);

        // Generate FLAC audio frames (uncompressed / VERBATIM subframe)
        for frame_idx in 0..num_frames {
            let samples_this_frame = if frame_idx == num_frames - 1 {
                total_samples - (frame_idx as u32) * (block_size as u32)
            } else {
                block_size as u32
            };

            let frame_bytes = build_flac_frame(
                samples_this_frame,
                block_size,
                sample_rate,
                channels,
                bits_per_sample,
                frame_idx as u32,
            );
            out.extend_from_slice(&frame_bytes);
        }

        let path = dir.join("test_silence.flac");
        std::fs::write(&path, &out).unwrap();
        path
    }

    fn build_flac_frame(
        blocksize: u32,
        _fixed_blocksize: u16,
        _sample_rate: u32,
        channels: u8,
        bits_per_sample: u8,
        frame_number: u32,
    ) -> Vec<u8> {
        let mut frame = Vec::new();

        // Sync code: 0xFFF8 (variable blocksize=0, fixed blocksize)
        frame.push(0xFF);
        frame.push(0xF8);

        // Block size bits (6-8 bits in header)
        // 0b1100 = 4096 samples
        frame.push(0xC0 | (channels - 1) << 4 | 0x0); // channel_assignment=stereo, sample_size=16bit(0x1)
        // Fix: sample size 16bit = 0x4 in 3-bit field
        // Actually: byte2 = blocking_strategy(1) | block_size(4) | sample_rate(4)
        // byte2 already set above. Let me redo:
        // Byte 2: block_size(4 bits) | sample_rate(4 bits)
        //   block_size for 4096 = 0xC
        //   sample_rate for 44100 = 0x9
        frame[2] = 0xC9;

        // Byte 3: channel_assignment(4 bits) | sample_size(3 bits) | reserved(1 bit)
        //   channels stereo = 0x1 (left/right)
        //   bits_per_sample 16 = 0x4
        frame.push(0x14 | 0x00); // 0x14

        // Frame number (UTF-8 encoded, for fixed blocksize = frame number)
        let encoded = encode_utf8_uint(frame_number);
        frame.extend_from_slice(&encoded);

        // CRC-8 of header so far (byte 0..n, before CRC)
        let crc = crc8(&frame);
        frame.push(crc);

        // Subframes: one per channel, VERBATIM (encoding=0x00000001)
        for _ in 0.. channels {
            // Subframe header: zero bit + subframe type (6 bits) + wasted bits flag (1 bit)
            // VERBATIM = type 0b000001 = 0x01
            // Packed as: 0 (1 bit) | 000001 (6 bits) | 0 (1 bit) = 0x02
            frame.push(0x02);
            // Verbatim samples: blocksize * (bits_per_sample / 8) bytes of zeros
            let bytes = blocksize as usize * (bits_per_sample as usize / 8);
            frame.extend(std::iter::repeat(0u8).take(bytes));
        }

        // CRC-16 of entire frame (including header and subframes)
        let crc = crc16(&frame);
        frame.extend_from_slice(&crc.to_be_bytes());

        frame
    }

    fn encode_utf8_uint(val: u32) -> Vec<u8> {
        if val < 0x80 {
            vec![val as u8]
        } else if val < 0x800 {
            vec![0xC0 | (val >> 6) as u8, 0x80 | (val & 0x3F) as u8]
        } else if val < 0x10000 {
            vec![
                0xE0 | (val >> 12) as u8,
                0x80 | ((val >> 6) & 0x3F) as u8,
                0x80 | (val & 0x3F) as u8,
            ]
        } else {
            vec![
                0xF0 | (val >> 18) as u8,
                0x80 | ((val >> 12) & 0x3F) as u8,
                0x80 | ((val >> 6) & 0x3F) as u8,
                0x80 | (val & 0x3F) as u8,
            ]
        }
    }

    fn crc8(data: &[u8]) -> u8 {
        data.iter().fold(0u8, |crc, &byte| {
            let mut c = crc ^ byte;
            for _ in 0..8 {
                if c & 0x80 != 0 {
                    c = (c << 1) ^ 0x07;
                } else {
                    c <<= 1;
                }
            }
            c
        })
    }

    fn crc16(data: &[u8]) -> u16 {
        data.iter().fold(0u16, |crc, &byte| {
            let mut c = crc ^ ((byte as u16) << 8);
            for _ in 0..8 {
                if c & 0x8000 != 0 {
                    c = (c << 1) ^ 0x8005;
                } else {
                    c <<= 1;
                }
            }
            c
        })
    }

    fn setup_test_flac() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        generate_test_flac(dir.path());
        dir
    }

    #[tokio::test]
    async fn test_hls_cache_generate_flac() {
        let flac_dir = setup_test_flac();
        let flac_path = flac_dir.path().join("test_silence.flac");
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = HlsCache::with_options(
            cache_dir.path(),
            DEFAULT_MAX_CACHE_BYTES,
            Duration::from_secs(6),
        );

        let segments = cache
            .get_or_generate(&flac_path, "test-track-001", "lossless")
            .await
            .expect("HLS generation from FLAC should succeed");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(!init_data.is_empty());
        assert!(init_data.len() >= 8);

        assert!(segments.playlist_path().exists());
        let playlist = std::fs::read_to_string(segments.playlist_path()).unwrap();
        assert!(playlist.contains("#EXTM3U"));
        assert!(playlist.contains("#EXT-X-MAP"));
        assert!(playlist.contains("seg0.m4s"));

        assert!(segments.segment_count() >= 1);
        let seg0 = segments.segment_path(0).unwrap();
        assert!(seg0.exists());
        assert!(!std::fs::read(&seg0).unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_hls_cache_hit_on_second_request() {
        let flac_dir = setup_test_flac();
        let flac_path = flac_dir.path().join("test_silence.flac");
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = HlsCache::with_options(
            cache_dir.path(),
            DEFAULT_MAX_CACHE_BYTES,
            Duration::from_secs(6),
        );

        let seg1 = cache
            .get_or_generate(&flac_path, "test-track-cache-hit", "lossless")
            .await
            .unwrap();

        let seg2 = cache
            .get_or_generate(&flac_path, "test-track-cache-hit", "lossless")
            .await
            .unwrap();

        assert_eq!(seg1.root(), seg2.root());
        assert_eq!(seg1.segment_count(), seg2.segment_count());
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let flac_dir = setup_test_flac();
        let flac_path = flac_dir.path().join("test_silence.flac");
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = HlsCache::with_options(
            cache_dir.path(),
            1,
            Duration::from_secs(6),
        );

        let _seg_a = cache
            .get_or_generate(&flac_path, "track-a", "lossless")
            .await
            .unwrap();

        let _seg_b = cache
            .get_or_generate(&flac_path, "track-b", "lossless")
            .await
            .unwrap();

        let track_a_dir = cache_dir.path().join("track-a").join("lossless");
        assert!(!track_a_dir.exists());

        let track_b_dir = cache_dir.path().join("track-b").join("lossless");
        assert!(track_b_dir.exists());
    }

    #[test]
    fn test_generate_hls_sync() {
        let flac_dir = setup_test_flac();
        let flac_path = flac_dir.path().join("test_silence.flac");
        let cache_dir = tempfile::tempdir().unwrap();

        let segments = generate_hls(
            &flac_path,
            "sync-test-track",
            "lossless",
            cache_dir.path(),
        )
        .expect("sync HLS generation should succeed");

        assert!(segments.init_path().exists());
        assert!(segments.playlist_path().exists());
        assert!(segments.segment_count() >= 1);
    }

    fn generate_test_aiff(dir: &Path, sample_rate: u32, bits_per_sample: u8, channels: u8, duration_secs: f64) -> PathBuf {
        let num_frames = (sample_rate as f64 * duration_secs).round() as u32;
        let bytes_per_sample = (bits_per_sample as usize + 7) / 8;
        let bytes_per_frame = channels as usize * bytes_per_sample;
        let data_len = num_frames as usize * bytes_per_frame;

        let comm_size: u32 = 18;
        let ssnd_size: u32 = 8 + data_len as u32;
        let form_size = 4 + 8 + comm_size + 8 + ssand_size_padded(ssnd_size);

        let mut out = Vec::with_capacity(12 + form_size as usize);

        // FORM header
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&form_size.to_be_bytes());
        out.extend_from_slice(b"AIFF");

        // COMM chunk
        out.extend_from_slice(b"COMM");
        out.extend_from_slice(&comm_size.to_be_bytes());
        out.extend_from_slice(&(channels as u16).to_be_bytes());
        out.extend_from_slice(&num_frames.to_be_bytes());
        out.extend_from_slice(&(bits_per_sample as u16).to_be_bytes());
        out.extend_from_slice(&encode_aiff_sample_rate(sample_rate));

        // SSND chunk
        out.extend_from_slice(b"SSND");
        out.extend_from_slice(&ssnd_size.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend(std::iter::repeat(0u8).take(data_len));
        if data_len % 2 != 0 {
            out.push(0);
        }

        let path = dir.join(format!("test_{sample_rate}_{bits_per_sample}bit_{channels}ch.aiff"));
        std::fs::write(&path, &out).unwrap();
        path
    }

    fn ssand_size_padded(ssnd_size: u32) -> u32 {
        let data_len = ssnd_size - 8;
        ssnd_size + (data_len % 2)
    }

    /// Encode a sample rate as AIFF 80-bit extended (simplified for integer rates).
    fn encode_aiff_sample_rate(rate: u32) -> [u8; 10] {
        // For integer sample rates, the 80-bit extended format is:
        // sign(1) | exponent(15) | mantissa(64)
        // value = mantissa * 2^(exponent - 16383 - 63)
        // For rate = N, we need mantissa * 2^(exp - 16446) = N
        // Simple approach: find the highest set bit, set exponent accordingly
        let mut mantissa = rate as u64;
        let mut shift = 0i32;
        if rate > 0 {
            while mantissa < (1u64 << 63) {
                mantissa <<= 1;
                shift += 1;
            }
        }
        let exponent = (16446 - shift) as u16; // 16383 + 63 - shift
        let sign_bit: u16 = 0; // positive
        let exp_field = sign_bit | exponent;
        let mut buf = [0u8; 10];
        buf[0..2].copy_from_slice(&exp_field.to_be_bytes());
        buf[2..10].copy_from_slice(&mantissa.to_be_bytes());
        buf
    }

    /// Generate a minimal WAV file with the given parameters.
    fn generate_test_wav(dir: &Path, sample_rate: u32, bits_per_sample: u8, channels: u8, duration_secs: f64) -> PathBuf {
        let num_frames = (sample_rate as f64 * duration_secs).round() as u32;
        let bytes_per_sample = (bits_per_sample as usize + 7) / 8;
        let data_len = num_frames as usize * channels as usize * bytes_per_sample;

        let fmt_size: u32 = 16;
        let file_size = 36 + data_len as u32;

        let mut out = Vec::with_capacity(44 + data_len);

        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(b"WAVE");

        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&fmt_size.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(channels as u16).to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
        out.extend_from_slice(&byte_rate.to_le_bytes());
        let block_align = channels as u16 * bytes_per_sample as u16;
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&(bits_per_sample as u16).to_le_bytes());

        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        out.extend(std::iter::repeat(0u8).take(data_len));

        let path = dir.join(format!("test_{sample_rate}_{bits_per_sample}bit_{channels}ch.wav"));
        std::fs::write(&path, &out).unwrap();
        path
    }

    #[test]
    fn test_alac_remux_96khz_32bit_aiff() {
        let dir = tempfile::tempdir().unwrap();
        let aiff_path = generate_test_aiff(dir.path(), 96000, 32, 2, 0.5);

        let cache_dir = tempfile::tempdir().unwrap();
        let segments = generate_hls(&aiff_path, "alac-test-96k-32bit", "lossless", cache_dir.path())
            .expect("HLS generation from 96kHz AIFF should succeed via ALAC");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(
            init_data.windows(4).any(|w| w == b"alac"),
            "init.mp4 should contain ALAC sample entry, got: {:?}",
            &init_data[..init_data.len().min(100)]
        );

        assert!(segments.playlist_path().exists());
        assert!(segments.segment_count() >= 1);

        let seg0 = segments.segment_path(0).unwrap();
        let seg_data = std::fs::read(&seg0).unwrap();
        assert!(!seg_data.is_empty());
    }

    #[test]
    fn test_alac_remux_96khz_16bit_aiff() {
        let dir = tempfile::tempdir().unwrap();
        let aiff_path = generate_test_aiff(dir.path(), 96000, 16, 2, 0.5);

        let cache_dir = tempfile::tempdir().unwrap();
        let segments = generate_hls(&aiff_path, "alac-test-96k-16bit", "lossless", cache_dir.path())
            .expect("HLS generation from 96kHz 16-bit AIFF should succeed via ALAC");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(
            init_data.windows(4).any(|w| w == b"alac"),
            "init.mp4 should contain ALAC sample entry for 96kHz 16-bit"
        );
        assert!(segments.segment_count() >= 1);
    }

    #[test]
    fn test_lpcm_remux_44khz_aiff_stays_lpcm() {
        let dir = tempfile::tempdir().unwrap();
        let aiff_path = generate_test_aiff(dir.path(), 44100, 16, 2, 0.5);

        let cache_dir = tempfile::tempdir().unwrap();
        let segments = generate_hls(&aiff_path, "lpcm-test-44k", "lossless", cache_dir.path())
            .expect("HLS generation from 44.1kHz AIFF should succeed via LPCM");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(
            !init_data.windows(4).any(|w| w == b"alac"),
            "44.1kHz AIFF should use LPCM, not ALAC"
        );
        assert!(
            init_data.windows(4).any(|w| w == b"lpcm"),
            "44.1kHz AIFF should use LPCM sample entry"
        );
    }

    #[test]
    fn test_alac_remux_96khz_wav() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = generate_test_wav(dir.path(), 96000, 24, 2, 0.5);

        let cache_dir = tempfile::tempdir().unwrap();
        let segments = generate_hls(&wav_path, "alac-test-96k-wav", "lossless", cache_dir.path())
            .expect("HLS generation from 96kHz WAV should succeed via ALAC");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(
            init_data.windows(4).any(|w| w == b"alac"),
            "init.mp4 should contain ALAC sample entry for 96kHz WAV"
        );
        assert!(segments.segment_count() >= 1);
    }

    #[test]
    fn test_alac_remux_mono() {
        let dir = tempfile::tempdir().unwrap();
        let aiff_path = generate_test_aiff(dir.path(), 96000, 24, 1, 0.5);

        let cache_dir = tempfile::tempdir().unwrap();
        let segments = generate_hls(&aiff_path, "alac-test-mono", "lossless", cache_dir.path())
            .expect("HLS generation from mono 96kHz AIFF should succeed via ALAC");

        assert!(segments.init_path().exists());
        let init_data = std::fs::read(segments.init_path()).unwrap();
        assert!(
            init_data.windows(4).any(|w| w == b"alac"),
            "init.mp4 should contain ALAC sample entry for mono"
        );
    }
}
