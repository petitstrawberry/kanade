# Kanade — Design Document

## Architecture

```
  Clients                             Kanade Server                           Output Nodes
  ┌──────────┐                       ┌──────────────────┐                 ┌──────────────┐
  │ kanade-  │  WS :8080, HTTP: 8081  │                  │   WS :8082      │ living-room  │
  │ web      │─────────────────────▶ │  kanade-core     │────────────────▶│  (MPD)       │
  └──────────┘                       │  State · Queue   │                 └──────────────┘
                                     │  Controller      │                 ┌──────────────┐
                                     │                  │   WS :8082      │  study       │
                                     │  kanade-db       │────────────────▶│  (MPD)       │
                                     │  SQLite + FTS5   │                 └──────────────┘
  ┌──────────┐  WS :8080             │                  │   WS :8082      ┌──────────────┐
  │ kanade-  │─────────────────────▶ │  kanade-scanner  │────────────────▶│  kitchen     │
  │ tui      │                       └──────────────────┘                 │  (MPD)       │
  └──────────┘                                                            └──────────────┘

  WS   :8080   Client Subprotocol   (server ↔ web, tui)
  WS   :8082   Node Subprotocol      (server ↔ output nodes)
  HTTP :8081 Media Surface         (track streaming + artwork)
```

## Principles

1. **Server is the single source of truth** — all state lives in the server. Clients are stateless renderers.
2. **Clients and server have independent lifetimes** — start/stop/restart independently.
3. **Output nodes are separate processes** — they connect to the server via the kanade protocol and can run on different machines.
4. **MPD is one output backend** — the output node (`kanade-node`) drives MPD internally. Other backends (ALSA, CoreAudio, etc.) are future possibilities.
5. **Nodes from day one** — a Node groups one or more Outputs and owns its own queue + playback state.
6. **Hexagonal architecture** — core never depends on I/O adapters.

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

`RemoteNodeOutput` is the concrete implementation used on the server. It forwards every call over the kanade protocol WebSocket to the connected output node. MPD is one audio backend the output node can use; others are possible.

### kanade protocol

The kanade protocol is a WebSocket JSON protocol used exclusively for communication between the Kanade server and output nodes.

#### Handshake

1. Output node connects to `NODE_ADDR` (default `0.0.0.0:8082`).
2. Node → Server: `NodeRegistration` — announces a human-readable `name`.
3. Server → Node: `NodeRegistrationAck` — assigns a UUID as the `node_id` and provides the `media_base_url` the node must use when constructing track URIs for its local audio backend.

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
{"status": "playing", "position_secs": 1.23, "volume": 72, "current_index": 0}
```

### Server (daemon)

```
main()
├── open kanade-db
├── create Core (empty outputs — nodes connect dynamically)
├── spawn MediaServer (HTTP, port 8081)
├── spawn scanner (background thread, if MUSIC_DIR set)
├── spawn NodeServer (kanade protocol, port 8082)
├── spawn WsServer (client WebSocket, port 8080)
└── spawn OpenHomeServer (UPnP/OpenHome, port 8090)
```

When an output node connects:
```
NodeServer.accept()
  → NodeRegistration handshake
  → create RemoteNodeOutput (backed by channel)
  → Core.register_output(node_id, remote_output)
  → Core.add_node(Node { id: node_id, ... })
  → relay loop: NodeCommand ↔ NodeStateUpdate
```

### Output Node (kanade-node)

```
main()
├── connect to SERVER_ADDR (NODE_ADDR on server, default ws://127.0.0.1:8082)
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
├── kanade-adapter-node-server/  Server-side kanade protocol adapter
├── kanade-adapter-ws/           WebSocket server (JSON protocol, client-facing)
├── kanade-adapter-openhome/     OpenHome/UPnP adapter (client-facing)
├── kanade-server-http/          HTTP media server (audio files + artwork)
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

### Node

```rust
pub struct Node {
    pub id: String,              // matches node_id of the connected output node
    pub name: String,
    pub output_ids: Vec<String>, // output IDs (one per node in typical setup)
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

## WebSocket Protocol (client-facing)

### Client → Server

Two message types:

**Commands** (fire-and-forget, no response):
```json
{"cmd": "play", "node_id": "..."}
{"cmd": "pause", "node_id": "..."}
{"cmd": "stop", "node_id": "..."}
{"cmd": "next", "node_id": "..."}
{"cmd": "previous", "node_id": "..."}
{"cmd": "seek", "node_id": "...", "position_secs": 30.0}
{"cmd": "set_volume", "node_id": "...", "volume": 75}
{"cmd": "set_shuffle", "node_id": "...", "shuffle": true}
{"cmd": "set_repeat", "node_id": "...", "repeat": "all"}
{"cmd": "add_to_queue", "node_id": "...", "track": {...}}
{"cmd": "clear_queue", "node_id": "..."}
```

**Requests** (expect response with matching `req_id`):
```json
{"req_id": 1, "req": "get_nodes"}
{"req_id": 2, "req": "get_albums"}
{"req_id": 3, "req": "get_album_tracks", "album_id": "..."}
{"req_id": 4, "req": "search", "query": "..."}
{"req_id": 5, "req": "get_queue", "node_id": "..."}
```

### Server → Client

**State push** (sent on any state change):
```json
{"type": "state", "nodes": [{"id": "...", "name": "Living Room", "status": "playing", ...}]}
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
Client: {"cmd": "play", "node_id": "living-room"}
  → WsServer → Core.play_node("living-room")
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
  → NodeEventBroadcaster converts state to NodeStateUpdate JSON
  → Sent over WebSocket to server
    → NodeServer receives NodeStateUpdate
    → Core.sync_node_state(...) updates node state + broadcasts
  → All clients receive updated {"type": "state", ...}
```

### Library browsing
```
Client: {"req_id": 1, "req": "get_albums"}
  → WS server → DB.get_all_albums()
  → WS server replies: {"type": "response", "req_id": 1, "data": {"albums": [...]}}
Client: {"req_id": 2, "req": "get_album_tracks", "album_id": "xyz"}
  → WS server → DB.get_tracks_by_album_id("xyz")
  → WS server replies: {"type": "response", "req_id": 2, "data": {"tracks": [...]}}
Client: {"cmd": "add_to_queue", "node_id": "living-room", "track": {...}}
  → WS server → Core.add_to_queue("living-room", track)
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
| Output abstraction | `AudioOutput` trait | Output node is one backend among many; swap freely |
| Output node separation | kanade protocol (WS) | Server and nodes can run on different machines |
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
| `NODE_ADDR` | `0.0.0.0:8082` | kanade protocol listen address (output nodes) |
| `WS_ADDR` | `0.0.0.0:8080` | WebSocket listen address (clients) |
| `OH_ADDR` | `0.0.0.0:8090` | OpenHome listen address |
| `LOG_PATH` | `kanade.log` | Log file path |
| `RUST_LOG` | `kanade=info` | Log filter |

### Output Node (`kanade-node`)

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_NAME` | `node` | Human-readable node name shown in clients (ID is auto-assigned by server) |
| `SERVER_ADDR` | `ws://127.0.0.1:8082` | kanade server node endpoint URL |
| `MPD_HOST` | `127.0.0.1` | Local MPD host |
| `MPD_PORT` | `6600` | Local MPD port |
| `RUST_LOG` | `kanade_node=info` | Log filter |
