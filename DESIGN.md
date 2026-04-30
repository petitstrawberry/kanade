# Kanade — Design Document

## Architecture

```
  Clients / Output Nodes                 Kanade Server
  ┌──────────┐                       ┌──────────────────┐
  │ kanade-  │  WS :8080            │                  │
  │ web      │─────────────────────▶ │  kanade-core     │
  └──────────┘                       │  State · Queue   │
                                      │  Controller      │
  ┌──────────┐  WS :8080             │                  │
  │ kanade-  │─────────────────────▶ │  kanade-db       │
  │ tui      │                       │  SQLite + FTS5   │
  └──────────┘                       │                  │
                                      │  kanade-scanner  │
  ┌──────────────┐  WS :8080         │                  │
  │ living-room  │──────────────────▶ │  kanade-adapter-ws│
  │ (MPD node)   │                    │  (single port)   │
  └──────────────┘                    │                  │
  ┌──────────────┐  WS :8080         │  kanade-server-http
  │ kitchen       │──────────────────▶ │  (:8081 media)   │
  │ (MPD node)   │                    │                  │
  └──────────────┘                    └──────────────────┘

  WS   :8080   All WebSocket clients (web, tui, output nodes)
  HTTP :8081   Media Surface         (track streaming + artwork)
  HTTP :8090   OpenHome / UPnP       (SOAP/XML)
```

## Principles

1. **Server is the single source of truth** — all state lives in the server. Clients are stateless renderers.
2. **Clients and server have independent lifetimes** — start/stop/restart independently.
3. **Output nodes are separate processes** — they connect to the server via WebSocket and can run on different machines.
4. **MPD is one output backend** — the output node (`kanade-node`) drives MPD internally. Other backends (ALSA, CoreAudio, etc.) are future possibilities.
5. **Nodes from day one** — a Node groups one or more Outputs and owns its own queue + playback state.
6. **Hexagonal architecture** — core never depends on I/O adapters.
7. **Single port** — one WebSocket endpoint (`:8080`) serves both clients and output nodes. The server identifies the connection type from the first message.

## Core Concepts

### Node

A Node is a logical audio destination. Each node has:

- A name
- One or more **Outputs** (physical devices that produce sound)
- Its own **queue** (ordered list of tracks to play)
- Its own **playback state** (Playing, Paused, Stopped, position)
- Its own **volume**, **shuffle**, **repeat** settings

```
Node "Living Room"
├── Outputs: [RemoteNodeOutput → kanade-node → MPD]
├── Queue: [Track A, Track B, Track C]
├── State: Playing, position 1:23
├── Volume: 72
├── Shuffle: false
└── Repeat: All
```

When a node plays, it sends audio to all its outputs simultaneously.

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
    async fn set_queue(&self, file_paths: &[String]) -> Result<(), CoreError>;
}
```

`RemoteNodeOutput` is the concrete implementation used on the server. It forwards every call over WebSocket to the connected output node. MPD is one audio backend the output node can use; others are possible.

### kanade protocol

The kanade protocol is a WebSocket JSON protocol used for communication between the Kanade server and output nodes. It shares the single `:8080` WebSocket port with client connections.

#### Handshake

1. Output node connects to `WS_ADDR` (default `0.0.0.0:8080`).
2. Node sends `NodeRegistration` — announces a human-readable `name`.
3. Server sends `NodeRegistrationAck` — assigns a UUID as the `node_id` and provides the `media_base_url` the node must use when constructing track URIs for its local audio backend.

#### Commands (Server → Node)

```json
{"type": "play"}
{"type": "pause"}
{"type": "stop"}
{"type": "seek", "position_secs": 30.0}
{"type": "set_volume", "volume": 75}
{"type": "set_queue", "file_paths": ["..."]}
{"type": "add", "file_paths": ["..."]}
{"type": "remove", "index": 0}
{"type": "move_track", "from": 0, "to": 2}
```

#### State updates (Node → Server)

```json
{"status": "playing", "position_secs": 1.23, "volume": 72, "mpd_song_index": 0, "projection_generation": 1}
```

### Server (daemon)

```
main()
├── open kanade-db
├── create Core (empty outputs — nodes connect dynamically)
├── spawn MediaServer (HTTP, port 8081)
├── spawn scanner (background thread, if MUSIC_DIR set)
├── spawn WsServer (WebSocket, port 8080 — serves both clients and nodes)
└── spawn OpenHomeServer (UPnP/OpenHome, port 8090)
```

When an output node connects:
```
WsServer.accept()
  → wait for first message (up to 10s)
  → NodeRegistration detected → enter node mode
    → NodeRegistrationAck handshake
    → create RemoteNodeOutput (backed by channel)
    → Core.register_output(node_id, remote_output)
    → Core.add_node(Node { id: node_id, ... })
    → relay loop: NodeCommand ↔ NodeStateUpdate
```

### Output Node (kanade-node)

```
main()
├── connect to SERVER_ADDR (default ws://127.0.0.1:8080)
├── send NodeRegistration
├── receive NodeRegistrationAck (get media_base_url)
├── create MpdRenderer (using media_base_url)
├── create local PlaybackState
├── spawn MpdStateSync (polls local MPD, sends NodeStateUpdate to server)
└── relay loop: NodeCommand → MpdRenderer, MpdStateSync → NodeStateUpdate
```

### Client (TUI, Web, etc.)

A client:
1. Connects to the server via WebSocket (port 8080)
2. Sends commands (play, pause, browse library, add to queue, etc.)
3. Receives state pushes (playback state, node changes, scan progress)
4. Renders UI

A client holds NO persistent state. Everything comes from the server.

## Workspace

```
kanade/
├── kanade-core/                 Domain models, node/output traits, Core
├── kanade-db/                   SQLite persistence, FTS5
├── kanade-scanner/              Library scanner (lofty + dsf-meta)
├── kanade-node-protocol/        Shared kanade protocol message types
├── kanade-adapter-mpd/          MPD AudioOutput + state sync (used by kanade-node)
├── kanade-adapter-node-server/  Server-side node connection handler (legacy, merged into kanade-adapter-ws)
├── kanade-adapter-ws/           WebSocket server (clients + nodes, single port :8080)
├── kanade-adapter-openhome/     OpenHome/UPnP adapter
├── kanade-server-http/          HTTP media server (audio files + artwork, port :8081)
├── kanade-tui/                  TUI client (pure WS client + ratatui)
├── kanade-node/                 Output node binary (connects to server, drives MPD)
└── kanade/                      Server binary entrypoint
```

## Models

### Track

```rust
pub struct Track {
    pub id: String,              // SHA-256(file_path)
    pub file_path: String,
    pub album_id: Option<String>,// SHA-256(dir_path)
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album_title: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_secs: Option<f64>,
    pub format: Option<String>,  // "FLAC", "MP3", "M4A", "DSD (DSD128)", etc.
    pub sample_rate: Option<u32>,
}
```

### Node

```rust
pub struct Node {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub status: PlaybackStatus,
    pub position_secs: f64,
    pub volume: u8,
}
```

### PlaybackState

```rust
pub struct PlaybackState {
    pub nodes: Vec<Node>,
    pub selected_node_id: Option<String>,
    pub queue: Vec<Track>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}
```

```rust
pub enum RepeatMode { Off, One, All }
pub enum PlaybackStatus { Stopped, Playing, Paused, Loading }
```

### Album / Artist

```rust
pub struct Album {
    pub id: String,       // SHA-256(dir_path)
    pub dir_path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub artwork_path: Option<String>,
}

pub struct Artist {
    pub id: String,       // SHA-256(name)
    pub name: String,
}
```

## Data Flow

### Playback
```
Client: {"cmd": "play"}
  → WsServer → Core.play()
    → Core sets node.status = Playing
    → Core forwards to RemoteNodeOutput: output.play()
      → NodeCommand::Play sent over WebSocket to kanade-node
        → kanade-node → MpdRenderer.play() → MPD: "play" command
    → Core broadcasts state to all clients via WS
  → All clients receive {"type": "state", ...}
```

### State sync (node → server)
```
MpdStateSync (on kanade-node) polls MPD every 500ms
  → converts state to NodeStateUpdate JSON
  → Sent over WebSocket to server (:8080)
    → WsServer receives NodeStateUpdate in node mode
    → Core.sync_node_state(...) updates node state + broadcasts
  → All clients receive updated {"type": "state", ...}
```

### Library browsing
```
Client: {"req_id": 1, "req": "get_albums"}
  → WsServer → DB.get_all_albums()
  → WsServer replies: {"type": "response", "req_id": 1, "data": {"albums": [...]}}
Client: {"req_id": 2, "req": "get_album_tracks", "album_id": "xyz"}
  → WsServer → DB.get_tracks_by_album_id("xyz")
  → WsServer replies: {"type": "response", "req_id": 2, "data": {"tracks": [...]}}
```

### Scanning
```
Server startup → scanner.run()
  → walkdir → extract metadata → upsert into DB
  → periodic incremental scan (mtime comparison)
  → state push: {"type": "state", ...} (if library changed)
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| State ownership | Core is sole owner | Clients are stateless; no state divergence |
| Output abstraction | `AudioOutput` trait | Output node is one backend among many; swap freely |
| Output node separation | kanade protocol (WS) | Server and nodes can run on different machines |
| Single port | All WS on :8080 | Simplifies deployment; avoids multi-port issues (e.g. iOS Safari LAN) |
| Node concept | From day one | Multi-room, per-node queue/volume/repeat |
| Client protocol | WebSocket JSON | Language-agnostic, bidirectional, fire-and-forget + req/res |
| Metadata source | File tags only (lofty + dsf-meta) | No external APIs; deterministic IDs via SHA-256 |
| ID scheme | SHA-256 of natural key | Deterministic, no drift between runs |
| Scan strategy | Server-side background loop | Startup scan + periodic incremental |
| DB | SQLite + FTS5 | Embedded, no external service dependency |
| Scanner | lofty (PCM) + dsf-meta (DSD) | Two-tier extraction for format coverage |

## Environment Variables

### Server (`kanade`)

| Variable | Default | Description |
|----------|---------|-------------|
| `MUSIC_DIR` | — | Root music directory |
| `DB_PATH` | `kanade.db` | SQLite database path |
| `SCAN_INTERVAL_SECS` | `300` | Scan interval |
| `MEDIA_ADDR` | `0.0.0.0:8081` | HTTP media server listen address |
| `MEDIA_PUBLIC_BASE_URL` | `http://127.0.0.1:8081` | Public base URL for media files |
| `WS_ADDR` | `0.0.0.0:8080` | WebSocket listen address (clients + nodes) |
| `OH_ADDR` | `0.0.0.0:8090` | OpenHome listen address |
| `RUST_LOG` | `kanade=info,kanade_core=debug` | Log filter |

### Output Node (`kanade-node`)

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_NAME` | `node` | Human-readable node name shown in clients (ID is auto-assigned by server) |
| `SERVER_ADDR` | `ws://127.0.0.1:8080` | Kanade server WebSocket endpoint |
| `MPD_HOST` | `127.0.0.1` | Local MPD host |
| `MPD_PORT` | `6600` | Local MPD port |
| `RUST_LOG` | `kanade_node=info` | Log filter |
