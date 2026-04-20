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
- **HLS streaming** — on-demand fMP4 remuxing for iOS/macOS clients (AVPlayer-compatible), with LRU disk cache and future ABR support

## Clients

Kanade can be controlled from multiple frontends:

- **Web** — `kanade-web/`, the browser-based client
- **Terminal** — `kanade-tui`, a terminal UI client
- **Apple platforms** — the native Kanade app for iOS and macOS:
  [petitstrawberry/KanadeApp](https://github.com/petitstrawberry/KanadeApp)

## Quick Start

### Prerequisites

- Docker and Docker Compose
- A running [MPD](https://www.musicpd.org/) daemon for your output node

### 1. Start the server

```sh
docker compose up -d
```

### 2. Start an output node

On the same host:

```sh
docker compose --profile node up -d
```

On a remote host, copy `docker-compose.node.yml` to that machine and run:

```sh
SERVER_ADDR=kanade.example.com:8080 MPD_HOST=127.0.0.1 \
  docker compose -f docker-compose.node.yml up -d
```

The first node to connect is assigned the id `"default"` and is the target
for clients that don't specify a node.

### 3. Connect a client

- **Web**: open `kanade-web/` (Svelte) — `?server=HOST:8080`
- **TUI**: build and run `kanade-tui`

## Build from Source

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
| `HLS_CACHE_DIR`         | *`DB_PATH` parent* `/.hls-cache` | HLS segment cache directory (on-demand fMP4 remux) |
| `SCAN_INTERVAL_SECS`    | `300`                          | Interval between periodic scans        |
| `PUBLIC_HOST`           | —                              | Public hostname or URL (mDNS, media URLs, client-facing). Set to your domain when using a reverse proxy (e.g. `kanade.example.com`). Can include scheme and port. |
| `BIND_ADDR`             | `0.0.0.0:8080`                 | Unified server bind address (WS + HTTP) |
| `MDNS_NAME`             | `Kanade`                       | mDNS service instance name             |
| `RUST_LOG`              | `kanade=info,kanade_core=debug`| Log level (tracing filter)             |

### Reverse Proxy

To serve the Web UI and API from a single port with TLS, use the example
nginx config in `docker/nginx-proxy.conf`. It proxies:

- `/` → static Web UI files
- `/ws` → Kanade server WebSocket
- `/media/` → Kanade server HTTP media

With this setup the Web UI connects automatically — no `?server=` parameter
needed.

### Output Node (`kanade-node`)

| Variable     | Default                  | Description                               |
| ------------ | ------------------------ | ----------------------------------------- |
| `NODE_NAME`  | `node`                   | Human-readable name (id is auto-assigned) |
| `SERVER_ADDR`| `127.0.0.1:8080`         | Kanade server address (host:port)         |
| `MPD_HOST`   | `127.0.0.1`              | Local MPD host                            |
| `MPD_PORT`   | `6600`                   | Local MPD port                            |
| `RUST_LOG`   | `kanade_node=info`       | Log filter                                |

## Architecture

```
  Clients                          Kanade Server (:8080)                       Output Nodes
  ┌──────────┐                     ┌────────────────────┐                    ┌──────────────┐
  │ kanade-  │  ws://host:8080/ws  │                    │  ws://host:8080/ws │ living-room  │
  │ web      │───────────────────▶ │  axum unified      │──────────────────▶ │  (MPD)       │
  └──────────┘                     │  /ws    WebSocket  │                    └──────────────┘
                                   │  /media/ HTTP      │                    ┌──────────────┐
  ┌──────────┐  ws://host:8080/ws  │                    │  ws://host:8080/ws │  study       │
  │ kanade-  │───────────────────▶ │  kanade-core       │──────────────────▶ │  (MPD)       │
  │ tui      │                     │  kanade-db         │                    └──────────────┘
  └──────────┘                     │  kanade-scanner    │                    ┌──────────────┐
                                   └────────────────────┘                    │  kitchen     │
                                                                             └──────────────┘

  :8080  /ws      WebSocket (clients + output nodes)
         /media/  HTTP media surface (track streaming + artwork + HLS)
```

For implementation details and design decisions, see [DESIGN.md](DESIGN.md).

## Protocols

| Protocol | Port | Direction | Format |
| -------- | ---- | --------- | ------ |
| WebSocket | 8080 (`/ws`) | Server ↔ All clients | WebSocket JSON |
| Media Surface | 8080 (`/media/`) | Clients → Server | HTTP |
| HLS Streaming | 8080 (`/media/hls/`) | Clients → Server | HTTP (fMP4/HLS) |

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
| `kanade-adapter-ws`        | WebSocket + HTTP media + HLS remux (axum unified router) |
| `kanade-server-http`       | HTTP media file serving (superseded by kanade-adapter-ws) |
| `kanade-node`              | Output node binary (connects to server, drives MPD) |
| `kanade-tui`               | Terminal UI client (ratatui)                      |
| `kanade-web`               | Web client (Svelte)                               |

## License

MIT License. See [LICENSE](LICENSE) for details.
