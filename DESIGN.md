# Kanade — Design Document

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  Kanade Server (daemon)                │
│                                                         │
│  ┌─────────────────────────────────────────────────┐  │
│  │                    Core                           │  │
│  │  Zones, Queue, Playback State, Library        │  │
│  └──────┬──────────────────────┬─────────────────┘  │
│         │                      │                     │
│  ┌──────▼──────────┐  ┌───────▼──────────┐         │
│  │  kanade-db      │  │  kanade-scanner │         │
│  │  SQLite + FTS5  │  │  bg scan loop   │         │
│  └─────────────────┘  └────────────────┘         │
│                                                     │
│  ┌─────────────────────────────────────────────────┐  │
│  │  Outputs (per-zone, pluggable)                  │  │
│  │  ┌──────────┐  ┌───────────┐  ┌──────────┐   │  │
│  │  │ MPD      │  │ CoreAudio │  │ ALSA     │   │  │
│  │  └──────────┘  └───────────┘  └──────────┘   │  │
│  └─────────────────────────────────────────────────┘  │
│                                                     │
│  ┌──────────────────────────────────────────────────┐ │
│  │  kanade-adapter-ws     kanade-adapter-openhome   │ │
│  │  WebSocket server      OpenHome/UPnP           │ │
│  └──────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
         │
         │  WebSocket (JSON)
         │
   ┌─────▼──────┐  ┌───────────┐  ┌──────────────┐
   │ kanade-tui  │  │ Web (React)│  │ SwiftUI     │
   │ (client)    │  │ (client)   │  │ (client)     │
   └────────────┘  └───────────┘  └──────────────┘
```

## Principles

1. **Server is the single source of truth** — all state lives in the server. Clients are stateless renderers.
2. **Clients and server have independent lifetimes** — start/stop/restart independently.
3. **Outputs are pluggable** — MPD, CoreAudio, ALSA, PipeWire, etc. First target: MPD.
4. **Zones from day one** — a Zone groups one or more Outputs and owns its own queue + playback state.
5. **Hexagonal architecture** — core never depends on I/O adapters.

## Core Concepts

### Zone

A Zone is a logical audio destination. Each zone has:

- A name
- One or more **Outputs** (physical devices that produce sound)
- Its own **queue** (ordered list of tracks to play)
- Its own **playback state** (Playing, Paused, Stopped, position)
- Its own **volume**, **shuffle**, **repeat** settings

```
Zone "Living Room"
├── Outputs: [MPD (stereo receiver)]
├── Queue: [Track A, Track B, Track C]
├── State: Playing, position 1:23
├── Volume: 72
├── Shuffle: false
└── Repeat: All
```

When a zone plays, it sends audio to all its outputs simultaneously.

### Output

An Output is a physical audio endpoint. It is a dumb I/O adapter — it receives commands from the Core and makes sound come out of a device.

```rust
#[async_trait]
pub trait AudioOutput: Send + Sync {
    async fn play(&self) -> Result<(), CoreError>;
    async fn pause(&self) -> Result<(), CoreError>;
    async fn stop(&self) -> Result<(), CoreError>;
    async fn seek(&self, position_secs: f64) -> Result<(), CoreError>;
    async fn set_volume(&self, volume: u8) -> Result<(), CoreError>;
    async fn add(&self, file_paths: &[String]) -> Result<(), CoreError>;
    async fn clear(&self) -> Result<(), CoreError>;
    async fn set_queue(&self, file_paths: &[String]) -> Result<(), CoreError>;
}
```

MPD is one implementation. CoreAudio/ALSA/etc. are future implementations. The Core does not care which backend is behind an Output.

### Server (daemon)

```
main()
├── open kanade-db
├── create outputs from config (MPD instances, etc.)
├── create zones from config (each zone → list of output IDs)
├── create Core (zones + library + state)
├── spawn scanner (background thread)
├── spawn WS server
└── spawn OH server
```

The server runs forever. Clients come and go.

### Client (TUI, Web, etc.)

A client:
1. Connects to the server via WebSocket
2. Sends commands (play, pause, browse library, add to queue, etc.)
3. Receives state pushes (playback state, zone changes, scan progress)
4. Renders UI

A client holds NO persistent state. Everything comes from the server.

## Workspace

```
kanade/
├── kanade-core/             Domain models, zone/output traits, Core
├── kanade-db/               SQLite persistence, FTS5
├── kanade-scanner/          Library scanner (lofty + dsf-meta)
├── kanade-adapter-mpd/      MPD AudioOutput implementation
├── kanade-adapter-ws/       WebSocket server (JSON protocol)
├── kanade-adapter-openhome/ OpenHome/UPnP adapter
├── kanade-tui/              TUI client (pure WS client + ratatui)
└── kanade/                  Server binary entrypoint
```

## Models

### Track

```rust
pub struct Track {
    pub id: String,              // SHA-256(file_path)
    pub file_path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_title: Option<String>,
    pub composer: Option<String>,
    pub track_number: Option<u32>,
    pub duration_secs: Option<f64>,
    pub format: Option<String>,  // "FLAC", "MP3", "DSD (DSD128)", etc.
    pub sample_rate: Option<u32>,
}
```

### Zone

```rust
pub struct Zone {
    pub id: String,              // UUID
    pub name: String,
    pub outputs: Vec<String>,    // output IDs
    pub queue: Vec<Track>,
    pub current_index: Option<usize>,
    pub status: PlaybackStatus,
    pub position_secs: f64,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

pub enum RepeatMode { Off, One, All }
pub enum PlaybackStatus { Stopped, Playing, Paused, Loading }
```

### Album / Artist

```rust
pub struct Album {
    pub id: String,       // SHA-256(dir_path)
    pub dir_path: String,
    pub title: Option<String>,
}

pub struct Artist {
    pub id: String,       // SHA-256(name)
    pub name: String,
}
```

## WebSocket Protocol

### Client → Server

Two message types:

**Commands** (fire-and-forget, no response):
```json
{"cmd": "play", "zone_id": "..."}
{"cmd": "pause", "zone_id": "..."}
{"cmd": "stop", "zone_id": "..."}
{"cmd": "next", "zone_id": "..."}
{"cmd": "previous", "zone_id": "..."}
{"cmd": "seek", "zone_id": "...", "position_secs": 30.0}
{"cmd": "set_volume", "zone_id": "...", "volume": 75}
{"cmd": "set_shuffle", "zone_id": "...", "shuffle": true}
{"cmd": "set_repeat", "zone_id": "...", "repeat": "all"}
{"cmd": "add_to_queue", "zone_id": "...", "track": {...}}
{"cmd": "clear_queue", "zone_id": "..."}
```

**Requests** (expect response with matching `req_id`):
```json
{"req_id": 1, "req": "get_zones"}
{"req_id": 2, "req": "get_albums"}
{"req_id": 3, "req": "get_album_tracks", "album_id": "..."}
{"req_id": 4, "req": "search", "query": "..."}
{"req_id": 5, "req": "get_queue", "zone_id": "..."}
```

### Server → Client

**State push** (sent on any state change):
```json
{"type": "state", "zones": [{"id": "...", "name": "Living Room", "status": "playing", ...}]}
```

**Response** (replies to requests):
```json
{"type": "response", "req_id": 2, "data": {"albums": [...]}}
{"type": "response", "req_id": 3, "data": {"tracks": [...]}}
{"type": "response", "req_id": 5, "data": {"tracks": [...], "current_index": 0}}
```

## Data Flow

### Playback
```
Client: {"cmd": "play", "zone_id": "abc"}
  → WS server → Core.play_zone("abc")
    → Core sets zone.status = Playing
    → Core forwards to all outputs in zone: output.play()
      → MPD: "play" command
    → Core broadcasts state to all clients via WS
  → All clients receive {"type": "state", ...}
```

### Library browsing
```
Client: {"req_id": 1, "req": "get_albums"}
  → WS server → DB.get_all_albums()
  → WS server replies: {"type": "response", "req_id": 1, "data": {"albums": [...]}}
Client: {"req_id": 2, "req": "get_album_tracks", "album_id": "xyz"}
  → WS server → DB.get_tracks_by_album_id("xyz")
  → WS server replies: {"type": "response", "req_id": 2, "data": {"tracks": [...]}}
Client: {"cmd": "add_to_queue", "zone_id": "abc", "track": {...}}
  → WS server → Core.add_to_queue("abc", track)
    → Core forwards: output.add([file_path])
    → Core broadcasts state
```

### Scanning
```
Server startup → scanner.run()
  → walkdir → extract metadata → upsert into DB
  → periodic incremental scan (mtime comparison)
  → state push: {"type": "scan_progress", ...}
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| State ownership | Core is sole owner | Clients are stateless; no state divergence |
| Output abstraction | `AudioOutput` trait | MPD is one backend among many; swap freely |
| Zone concept | From day one | Multi-room, per-zone queue/volume/repeat |
| Client protocol | WebSocket JSON | Language-agnostic, bidirectional, fire-and-forget + req/res |
| Metadata source | File tags only (lofty + dsf-meta) | No external APIs; deterministic IDs via SHA-256 |
| ID scheme | SHA-256 of natural key | Deterministic, no drift between runs |
| Scan strategy | Server-side background loop | Startup scan + periodic incremental |
| DB | SQLite + FTS5 | Embedded, no external service dependency |
| Scanner | lofty (PCM) + dsf-meta (DSD) | Two-tier extraction for format coverage |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MUSIC_DIR` | — | Root music directory |
| `DB_PATH` | `kanade.db` | SQLite database path |
| `SCAN_INTERVAL_SECS` | `300` | Scan interval |
| `WS_ADDR` | `0.0.0.0:8080` | WebSocket listen address |
| `OH_ADDR` | `0.0.0.0:8090` | OpenHome listen address |
| `MPD_HOST` | `127.0.0.1` | Default MPD host |
| `MPD_PORT` | `6600` | Default MPD port |
| `LOG_PATH` | `kanade.log` | Log file path |
| `RUST_LOG` | `kanade=info` | Log filter |
