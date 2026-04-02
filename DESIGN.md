# Kanade — Design Document

## 1. Design Goals

1. **Single-process, multi-client** — one binary runs the server, TUI, and all adapters
2. **Hexagonal architecture** — core domain never depends on I/O adapters
3. **Deterministic IDs** — SHA-256 hashes, no UUIDs, no auto-increment drift
4. **Purist metadata** — file tags are the source of truth; no external APIs
5. **Multi-frontend parity** — TUI, Web (React), and SwiftUI consume the same state model

## 2. Workspace Layout

```
kanade/
├── kanade-core/             Domain layer
├── kanade-db/               SQLite persistence
├── kanade-scanner/          Library scanner (NEW)
├── kanade-adapter-mpd/      MPD output adapter
├── kanade-adapter-ws/       WebSocket adapter
├── kanade-adapter-openhome/ OpenHome adapter
├── kanade-tui/              Terminal UI (NEW)
└── kanade/                  Binary entrypoint
```

## 3. Module Design

### 3.1 kanade-core (existing — no major changes)

Owns: `Track`, `Album`, `Artist`, `PlaybackState`, `CoreController`, port traits.

Port traits:
- **`AudioRenderer`** — play, pause, stop, next, prev, seek, set_volume, set_queue
- **`EventBroadcaster`** — `on_state_changed(&PlaybackState)`

New additions:
- Add `repeat_mode: RepeatMode` to `PlaybackState` (None, One, All) replacing the current `repeat: bool`
- Add `shuffle_mode: bool` to `PlaybackState` (already exists)

### 3.2 kanade-db (existing — minor additions)

Owns: SQLite schema, `Database` struct (sync, wrapped in `spawn_blocking`).

Changes:
- Add `get_all_tracks()` for library browsing
- Add `get_all_albums()` for album listing
- Add `get_album_by_dir()` helper
- Add `purge_missing(file_paths: &[String])` to remove DB entries for deleted files

### 3.3 kanade-scanner (NEW)

**Responsibility**: Walk music directories, extract metadata, store in SQLite.

Dependencies: `lofty`, `walkdir`, `kanade-core`, `kanade-db`

```
kanade-scanner/
├── src/
│   ├── lib.rs
│   ├── walker.rs      — recursive directory traversal
│   ├── extractor.rs   — lofty-based tag + property extraction
│   └── indexer.rs     — batch upsert into kanade-db
```

Design:
- `Scanner::new(db: Arc<Database>)`
- `Scanner::scan_dir(path: &Path) -> Result<ScanResult>` — full scan of a directory
- `Scanner::scan_incremental(path: &Path) -> Result<ScanResult>` — only new/modified files
  - Compares file mtime against a `last_scanned_at` timestamp stored per-file in SQLite
- Returns `ScanResult { added, updated, removed, elapsed }`
- Runs via `tokio::task::spawn_blocking` (lofty + rusqlite are sync)

Supported formats (via lofty): FLAC, MP3, AAC/M4A, OGG/Vorbis, Opus, WAV, AIFF, WMA, APE

Extraction per file:
```
lofty::Probe::open(path) → read()
  → tag: title, artist, album, track_number, composer, genre
  → properties: duration, sample_rate, channels, bit_depth, audio_bitrate
  → file_type → format string ("FLAC", "MP3", etc.)
```

### 3.4 kanade-adapter-mpd (existing — major additions)

Current state: `MpdRenderer` implements `AudioRenderer` (output only — sends commands to MPD, never reads state back).

**New: MPD state sync via idle loop**

MPD supports the `idle` command: the client blocks until the subsystem changes,
then the client reads the new state. This is the canonical way to sync.

Add `MpdStateSync` — a background task that:
1. Sends `idle` to MPD (blocks until player/playlist/mixer/etc changes)
2. On wake: reads `status`, `currentsong`, `playlistinfo`
3. Updates the shared `PlaybackState` accordingly
4. Calls `broadcast()` to push new state to all frontends
5. Loops back to step 1

```
MpdStateSync::new(client: MpdClient, state: Arc<RwLock<PlaybackState>>, broadcasters: [...])
MpdStateSync::run() — spawns a tokio task that loops forever
```

MPD responses to parse:
- `status` → state (play/pause/stop), song index, elapsed time, volume, repeat, random, consume, single
- `currentsong` → file path, title, artist, album, duration
- `playlistinfo` → ordered list of file paths (to rebuild queue)

This makes PlaybackState a **reflection of MPD truth**, not an independent source.

### 3.5 kanade-adapter-ws (existing — additions)

Current: handles playback commands + broadcasts state changes.

New commands to add:
- `BrowseLibrary` — list albums or tracks by album
- `SearchTracks` — FTS5 query, returns matching tracks
- `ScanDir` — trigger a scan (async, result broadcast when done)

Server → Client message types (unified):
```json
{"type": "state", "seq": 42, "payload": { ... PlaybackState ... }}
{"type": "scan_progress", "seq": 43, "payload": {"added": 12, "scanned": 45}}
{"type": "browse_result", "req_id": 7, "payload": [...albums...]}
{"type": "search_result", "req_id": 8, "payload": [...tracks...]}
```

### 3.6 kanade-adapter-openhome (existing — no major changes)

Works as-is for JPLAY and similar control points. May need volume service addition later.

### 3.7 kanade-tui (NEW)

**Responsibility**: Interactive terminal music player. Runs in-process, reads `PlaybackState` directly via `Arc<RwLock<>>`.

Dependencies: `ratatui`, `crossterm`, `kanade-core`, `kanade-db`

```
kanade-tui/
├── src/
│   ├── lib.rs
│   ├── app.rs          — App state machine (modes: browse, queue, search, now_playing)
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── now_playing.rs  — track info, progress bar, playback controls
│   │   ├── queue.rs        — track queue with scroll
│   │   ├── library.rs      — album/artist browser
│   │   └── search.rs       — FTS search input + results
│   └── input.rs        — key event handling
```

Layout (initial):
```
┌──────────────────────────────────────────────┐
│ Now Playing: Track Title - Artist            │
│ ▶ ████████░░░░░░░░░░░░░░ 01:23 / 04:56     │
│                                              │
│ Queue (3/12)                                 │
│ ▸ 1. Track A - Artist A                     │
│   2. Track B - Artist B                     │
│   3. Track C - Artist C                     │
│                                              │
│ [1-9] jump  [n]ext [p]rev [space] play/pause│
│ [s]earch [b]rowse [q]ueue [?] help          │
└──────────────────────────────────────────────┘
```

Key bindings:
- `Space` — play/pause
- `n` / `p` — next/previous
- `↑`/`↓` — navigate list
- `Enter` — select/play item
- `/` — search
- `Tab` — cycle panels (now playing, queue, library, search)
- `+`/`-` — volume
- `s` — stop

The TUI calls `CoreController` methods directly (in-process, no network round-trip).
It reads `PlaybackState` via `Arc<RwLock<>>` for display updates at ~10 Hz tick rate.

### 3.8 kanade (binary) — Wiring

The binary entrypoint orchestrates everything:

```
main()
├── init tracing
├── open kanade-db
├── create MpdRenderer + MpdStateSync
├── create broadcasters (WsBroadcaster, OhBroadcaster)
├── create CoreController
├── create Scanner (Arc<Database>)
├── spawn MpdStateSync::run()      — background task
├── spawn WsServer::run()          — background task
├── spawn OhServer::run()          — background task
└── run kanade-tui (blocking)      — foreground
```

The TUI is the foreground process. When the user quits the TUI, the entire
binary exits (background tasks are cancelled via tokio runtime drop).

## 4. Data Flow

### Playback flow
```
TUI keypress → CoreController.play() → MpdRenderer.execute_play() → MPD TCP
                                                         ↓
MPD state change → MpdStateSync reads idle → updates PlaybackState → broadcast
                                                              ↓
                                      TUI renders, WS pushes JSON, OH caches
```

### Library scan flow
```
TUI: / or Ctrl+S → trigger scan
  → Scanner::scan_dir(MUSIC_DIR) [spawn_blocking]
    → walkdir → lofty extract → Database::upsert_track (batch)
    → ScanResult returned
  → broadcast scan_progress to all frontends
```

## 5. Implementation Phases

### Phase 1: Scanner + DB integration
- [ ] Create `kanade-scanner` crate
- [ ] Implement walker (walkdir), extractor (lofty), indexer (batch upsert)
- [ ] Wire `kanade-db` into main.rs
- [ ] Add `get_all_albums()`, `get_album_tracks()`, `purge_missing()` to DB
- [ ] Add scan command to TUI and WS

### Phase 2: MPD state sync
- [ ] Add `MpdStateSync` to `kanade-adapter-mpd`
- [ ] Parse `status`, `currentsong`, `playlistinfo` responses
- [ ] Sync PlaybackState from MPD on startup + on every idle wake
- [ ] Spawn sync loop in main.rs

### Phase 3: TUI
- [ ] Create `kanade-tui` crate with ratatui + crossterm
- [ ] Implement now-playing view (track info + progress bar)
- [ ] Implement queue view
- [ ] Implement library browser (albums → tracks)
- [ ] Implement search (FTS5)
- [ ] Implement key bindings
- [ ] Wire TUI as foreground process in main.rs

### Phase 4: WS protocol extensions
- [ ] Add browse/search/scan commands to WebSocket protocol
- [ ] Add request/response message types (req_id)
- [ ] Add scan progress broadcast

### Phase 5 (future): Web frontend
- React + TypeScript + Vite
- Same WebSocket protocol as TUI
- Separate repository or `web/` subdirectory

### Phase 6 (future): SwiftUI client
- iOS/macOS native app
- Same WebSocket protocol

## 6. Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Metadata extraction | `lofty` | Single crate, covers tags + audio properties, all major formats, no C deps |
| ID scheme | SHA-256 of natural key | Deterministic, no drift between runs |
| TUI integration | In-process (shared Arc) | Zero latency, simplest wiring, reference implementation |
| MPD sync strategy | `idle` command loop | Canonical MPD pattern, no polling overhead |
| DB access model | Sync in `spawn_blocking` | rusqlite is sync; wrapping is simpler than async SQLite |
| State ownership | MPD is source of truth | Prevents state divergence between Kanade and MPD |
