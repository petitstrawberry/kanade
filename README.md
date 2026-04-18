# Kanade

**奏 (Kanade)** — a self-hosted music server for people who want their own library,
their own players, and their own output devices.

Use Kanade to scan your music collection, browse it from multiple clients, and
play it through one or more output nodes in your home. You can control the same
library from the web UI or the terminal UI.

Kanade is built for users first: point it at your music folder, start the
server, connect an output node, and play. If you want the implementation
details later, see [DESIGN.md](DESIGN.md) and [protocols.md](protocols.md).

## Features

- **Play your own library** — scan local music folders and browse albums,
  artists, tracks, and search results from your own server
- **Control from multiple clients** — use the web UI or terminal UI against
  the same playback state
- **Send audio to different rooms** — connect one or more output nodes and
  control them independently
- **Per-node playback controls** — each node keeps its own queue, volume,
  shuffle, and repeat state
- **MPD output today, more backends later** — the reference node drives a
  local [MPD](https://www.musicpd.org/) daemon
- **Built-in media serving** — artwork and track streaming are exposed by the
  server so clients can browse and play from a single place

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

- **Web**: open `kanade-web/` (Svelte) — `?server=HOST:8080`
- **TUI**: `cargo run -p kanade-tui --release`

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
| `SERVER_HOST`           | —                              | Public hostname (mDNS advertisement + media URL fallback) |
| `BIND_ADDR`             | `0.0.0.0:8080`                 | Unified server bind address (WS + HTTP) |
| `MEDIA_PUBLIC_BASE_URL` | auto (from `SERVER_HOST`)      | Public base URL for media file access (overrides `SERVER_HOST`) |
| `MDNS_NAME`             | `Kanade`                       | mDNS service instance name             |
| `RUST_LOG`              | `kanade=info,kanade_core=debug`| Log level (tracing filter)             |

### Output Node (`kanade-node`)

| Variable     | Default                  | Description                            |
| ------------ | ------------------------ | -------------------------------------- |
| `NODE_NAME`  | `node`                   | Human-readable name (id is auto-assigned) |
| `SERVER_ADDR`| `127.0.0.1:8080`        | Kanade server address (host:port)        |
| `MPD_HOST`   | `127.0.0.1`              | Local MPD host                         |
| `MPD_PORT`   | `6600`                   | Local MPD port                         |
| `RUST_LOG`   | `kanade_node=info`       | Log filter                             |

## Docker

### Server

```sh
docker compose up -d
```

### Output node (same host)

```sh
docker compose --profile node up -d
```

Node connects to a host MPD by default (`MPD_HOST=host.docker.internal`).

### Output node (remote host)

Copy `docker-compose.node.yml` to the target machine and run:

```sh
SERVER_ADDR=kanade.example.com:8080 MPD_HOST=127.0.0.1 \
  docker compose -f docker-compose.node.yml up -d
```

## Architecture

```
  Clients                          Kanade Server (:8080)                     Output Nodes
  ┌──────────┐                     ┌────────────────────┐                  ┌──────────────┐
  │ kanade-  │  ws://host:8080/ws  │                    │  ws://host:8080/ws │ living-room  │
  │ web      │───────────────────▶ │  axum unified      │───────────────▶ │  (MPD)       │
  └──────────┘                     │  /ws    WebSocket  │                  └──────────────┘
                                   │  /media/ HTTP      │                  ┌──────────────┐
  ┌──────────┐  ws://host:8080/ws  │                    │  ws://host:8080/ws │  study       │
  │ kanade-  │───────────────────▶ │  kanade-core       │───────────────▶ │  (MPD)       │
  │ tui      │                     │  kanade-db         │                  └──────────────┘
  └──────────┘                     │  kanade-scanner    │                  ┌──────────────┐
                                   └────────────────────┘                  │  kitchen     │
                                                                          └──────────────┘

  :8080  /ws      WebSocket (clients + output nodes)
         /media/  HTTP media surface (track streaming + artwork)
```

For implementation details and design decisions, see [DESIGN.md](DESIGN.md).

## Protocols

| Protocol | Port | Direction | Format |
| -------- | ---- | --------- | ------ |
| WebSocket | 8080 (`/ws`) | Server ↔ All clients | WebSocket JSON |
| Media Surface | 8080 (`/media/`) | Clients → Server | HTTP |

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
| `kanade-adapter-node-server`| Server-side node handler (merged into kanade-adapter-ws) |
| `kanade-adapter-ws`        | WebSocket + HTTP media (axum unified router)     |
| `kanade-server-http`       | HTTP media file serving (superseded by kanade-adapter-ws) |
| `kanade-node`              | Output node binary (connects to server, drives MPD) |
| `kanade-tui`               | Terminal UI client (ratatui)                      |
| `kanade-web`               | Web client (Svelte)                               |

## License

MIT License. See [LICENSE](LICENSE) for details.
