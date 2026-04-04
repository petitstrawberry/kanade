# Kanade

**奏** — A self-hosted music player server written in Rust.

Kanade sits between your music collection and your audio output, providing a
unified playback API that multiple frontends can consume simultaneously.

Output nodes connect to the server over the [Kanade Protocol](protocols.md)
and drive local audio backends (MPD, etc.). Clients control playback via
WebSocket (JSON), OpenHome (UPnP/SOAP), or a built-in TUI — all driven by a
single shared state.

## Features

- **Node-based playback** — logical nodes with per-node queues, volume,
  shuffle and repeat; multiple nodes can run on different machines
- **Output node separation** — audio backends run as separate processes
  (`kanade-node`) and connect to the server over WebSocket
- **MPD backend** — the reference output node drives a local
  [MPD](https://www.musicpd.org/) daemon; other backends are possible
- **Library management** — scans music directories, extracts metadata with
  [lofty](https://github.com/Serial-ATA/lofty-rs), indexes in SQLite with
  FTS5 full-text search
- **Kanade Protocol** — native WebSocket/JSON protocol family with Node
  Subprotocol (server ↔ output nodes) and Client Subprotocol (server ↔ clients)
- **OpenHome/UPnP** — external SOAP/XML adapter for control points like JPLAY
- **Hexagonal architecture** — core domain is adapter-agnostic; swap
  backends without touching business logic

## Architecture

```
  Clients                             Kanade Server                           Output Nodes
  ┌──────────┐                       ┌──────────────────┐                 ┌──────────────┐
  │ kanade-  │  WS :8080, HTTP: 8081 │                  │   WS :8082      │ living-room  │
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

See [DESIGN.md](DESIGN.md) for detailed design decisions and data flow.

## Quick Start

### Prerequisites

- Rust stable toolchain (or [Nix](https://nixos.org/) with the provided flake)
- A running [MPD](https://www.musicpd.org/) daemon

### 1. Start the server

```sh
MUSIC_DIR=/path/to/music cargo run -p kanade --release
```

### 2. Start an output node

```sh
cargo run -p kanade-node --release
```

The first node to connect is assigned the id `"default"` and is the target
for clients that don't specify a node.

### 3. Connect a client

- **Web**: open `kanade-web/` (Svelte) — `?server=ws://HOST:8080`
- **TUI**: `cargo run -p kanade-tui --release`
- **OpenHome**: point any control point at `HOST:8090`

### Nix Dev Shell

```sh
direnv allow   # or: nix develop
```

## Configuration

### Server (`kanade`)

| Variable                | Default                        | Description                            |
| ----------------------- | ------------------------------ | -------------------------------------- |
| `MUSIC_DIR`             | —                              | Root music directory for scanning      |
| `DB_PATH`               | `kanade.db`                    | SQLite database file path              |
| `SCAN_INTERVAL_SECS`    | `300`                          | Interval between periodic scans        |
| `MEDIA_ADDR`            | `0.0.0.0:8081`                 | HTTP media server bind address         |
| `MEDIA_PUBLIC_BASE_URL` | `http://127.0.0.1:8081`       | Public base URL for media file access  |
| `NODE_ADDR`             | `0.0.0.0:8082`                 | Kanade protocol listen address         |
| `WS_ADDR`               | `0.0.0.0:8080`                 | WebSocket client API bind address      |
| `OH_ADDR`               | `0.0.0.0:8090`                 | OpenHome HTTP server bind address      |
| `RUST_LOG`              | `kanade=info,kanade_core=debug`| Log level (tracing filter)             |

### Output Node (`kanade-node`)

| Variable     | Default                  | Description                            |
| ------------ | ------------------------ | -------------------------------------- |
| `NODE_NAME`  | `node`                   | Human-readable name (id is auto-assigned) |
| `SERVER_ADDR`| `ws://127.0.0.1:8082`   | Kanade server node endpoint            |
| `MPD_HOST`   | `127.0.0.1`              | Local MPD host                         |
| `MPD_PORT`   | `6600`                   | Local MPD port                         |
| `RUST_LOG`   | `kanade_node=info`       | Log filter                             |

## Protocols

| Protocol | Port | Direction | Format |
| -------- | ---- | --------- | ------ |
| **Kanade Protocol** | | | |
| ├─ Node Subprotocol | 8082 | Server ↔ Output Nodes | WebSocket JSON |
| ├─ Client Subprotocol | 8080 | Server ↔ Clients | WebSocket JSON |
| └─ Media Surface | 8081 | Clients → Server | HTTP |
| **OpenHome / UPnP** | 8090 | Control Points → Server | SOAP/XML |

See [protocols.md](protocols.md) for detailed protocol specifications.

## Workspace

| Crate                      | Role                                              |
| -------------------------- | ------------------------------------------------- |
| `kanade`                   | Headless server binary; wires all adapters        |
| `kanade-core`              | Domain models, state, port traits, controller    |
| `kanade-db`                | SQLite persistence, FTS5 full-text search         |
| `kanade-scanner`           | Library scanner (lofty + dsf-meta)                |
| `kanade-node-protocol`     | Shared Kanade Protocol message types              |
| `kanade-adapter-mpd`       | MPD audio backend (used by `kanade-node`)         |
| `kanade-adapter-node-server`| Server-side node connection handler              |
| `kanade-adapter-ws`        | WebSocket client API + state broadcast            |
| `kanade-adapter-openhome`  | OpenHome/UPnP SOAP adapter                       |
| `kanade-server-http`       | HTTP media file serving                           |
| `kanade-node`              | Output node binary (connects to server, drives MPD) |
| `kanade-tui`               | Terminal UI client (ratatui)                      |
| `kanade-web`               | Web client (Svelte)                               |

## License

MIT License. See [LICENSE](LICENSE) for details.

