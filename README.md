# Kanade

**奏** — A self-hosted music player server written in Rust.

Kanade sits between your music collection and your audio output, providing a
unified playback API that multiple frontends can consume simultaneously.

It uses [MPD](https://www.musicpd.org/) as its audio backend and exposes
playback control over WebSocket (JSON), OpenHome (UPnP/SOAP), and a built-in
TUI — all driven by a single shared state.

## Features

- **MPD backend** — delegates actual audio playback to a local MPD daemon
- **Library management** — scans music directories, extracts metadata with
  [lofty](https://github.com/Serial-ATA/lofty-rs), indexes in SQLite with
  FTS5 full-text search
- **Built-in TUI** — terminal UI via [ratatui](https://github.com/ratatui/ratatui);
  runs in-process for zero-latency state access
- **WebSocket API** — JSON protocol for remote clients (web, CLI, etc.)
- **OpenHome/UPnP** — SOAP/XML adapter for control points like JPLAY
- **Hexagonal architecture** — core domain is adapter-agnostic; swap MPD for
  another renderer without touching business logic

## Architecture

```
                         ┌─────────────────────────────┐
                         │          kanade-core         │
                         │  PlaybackState (RwLock)      │
                         │  CoreController              │
                         │  Port traits                 │
                         └──┬───────────────────────┬───┘
                            │                       │
                AudioRenderer                   EventBroadcaster
               (output port)                   (broadcast port)
                            │                       │
              ┌─────────────▼──────┐    ┌──────────▼──────────┐
              │ kanade-adapter-mpd │    │ kanade-adapter-ws   │
              │ (MPD TCP control)  │    │ (WebSocket / JSON)  │
              └────────────────────┘    ├─────────────────────┤
                                       │ kanade-adapter-oh   │
                                       │ (OpenHome / SOAP)   │
                                       └─────────────────────┘

                         ┌─────────────────────────────┐
                         │          kanade-db           │
                         │  SQLite + FTS5               │
                         │  Tracks, Albums, Artists      │
                         └─────────────────────────────┘
```

## Quick Start

### Prerequisites

- Rust stable toolchain (or [Nix](https://nixos.org/) with the provided flake)
- A running [MPD](https://www.musicpd.org/) daemon

### Build & Run

```sh
cargo run
```

### Configuration

| Environment Variable | Default        | Description                       |
| -------------------- | -------------- | --------------------------------- |
| `MPD_HOST`           | `127.0.0.1`    | MPD daemon host                   |
| `MPD_PORT`           | `6600`         | MPD daemon port                   |
| `WS_ADDR`            | `0.0.0.0:8080` | WebSocket server bind address     |
| `OH_ADDR`            | `0.0.0.0:8090` | OpenHome HTTP server bind address |
| `MUSIC_DIR`          | -              | Root music directory for scanning |
| `DB_PATH`            | `kanade.db`    | SQLite database file path         |
| `RUST_LOG`           | `kanade=info`  | Log level (tracing filter)        |

### Nix Dev Shell

```sh
direnv allow   # or: nix develop
cargo run
```

## Development

```sh
cargo build
cargo test
cargo run
```

## Workspace

| Crate                     | Role                                          |
| ------------------------- | --------------------------------------------- |
| `kanade`                  | Binary entrypoint; wires all adapters         |
| `kanade-core`             | Domain models, state, port traits, controller |
| `kanade-db`               | SQLite persistence, FTS5 search               |
| `kanade-adapter-mpd`      | MPD output adapter (AudioRenderer)            |
| `kanade-adapter-ws`       | WebSocket input + broadcast adapter           |
| `kanade-adapter-openhome` | OpenHome/UPnP input adapter                   |

## License

MIT
