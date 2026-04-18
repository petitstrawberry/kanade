# Kanade Protocol

The Kanade Protocol is the native protocol family for Kanade server communication. It comprises a single WebSocket endpoint serving both nodes and clients, plus an HTTP surface for media delivery.

External protocols like OpenHome/UPnP are documented separately.

## Table of Contents

- [Kanade Protocol](#kanade-protocol)
  - [1. Node Protocol](#1-node-protocol) — Server ↔ Output Nodes
  - [2. Client Protocol](#2-client-protocol) — Server ↔ Clients
  - [3. Media Surface](#3-media-surface) — HTTP file delivery
- [External Protocols](#external-protocols)
  - [OpenHome / UPnP](#openhome--upnp) — Control Points → Server
- [Shared Types](#shared-types)

---

## 1. Node Protocol

WebSocket JSON protocol between the Kanade server and output nodes.
Defined in [`kanade-node-protocol`](../kanade-node-protocol/src/lib.rs).

**Server endpoint**: `WS_ADDR` (default `ws://HOST:8080`, shared with clients)

The server identifies a connection as a node by the first message: if it parses as `NodeRegistration`, the connection enters node mode.

### 1.1 Connection Lifecycle

```
  Node                                    Server
    │─── WebSocket connect ────────────────→│  (:8080)
    │                                       │
    │─── NodeRegistration ─────────────────→│  (node sends its name)
    │←── NodeRegistrationAck ──────────────│  (server assigns node_id + media_base_url)
    │                                       │
    │←── NodeCommand ──────────────────────│  (server sends playback commands)
    │─── NodeStateUpdate ─────────────────→│  (node reports playback state)
    │      ...                              │
    │─── WebSocket close ─────────────────→│
```

The first node to connect is assigned `node_id: "default"`. Subsequent nodes
receive a random UUID.

### 1.2 Handshake Messages

#### NodeRegistration (Node → Server)

```json
{ "name": "Living Room" }
```

| Field | Type   | Description                   |
| ----- | ------ | ----------------------------- |
| name  | string | Human-readable node name      |

#### NodeRegistrationAck (Server → Node)

```json
{ "node_id": "default", "media_base_url": "http://192.168.1.10:8081" }
```

| Field           | Type   | Description                                        |
| --------------- | ------ | -------------------------------------------------- |
| node_id         | string | Server-assigned node identifier                     |
| media_base_url  | string | HTTP base URL for constructing track URIs          |

### 1.3 NodeCommand (Server → Node)

Tagged union using `"type"` field. Mirrors the `AudioOutput` trait methods.

```json
{ "type": "play" }
{ "type": "pause" }
{ "type": "stop" }
{ "type": "seek", "position_secs": 30.0 }
{ "type": "set_volume", "volume": 75 }
{ "type": "set_queue", "file_paths": ["/music/track.flac", "/music/track2.flac"], "projection_generation": 4 }
{ "type": "add", "file_paths": ["/music/new.flac"] }
{ "type": "remove", "index": 0 }
{ "type": "move_track", "from": 0, "to": 2 }
```

| Variant     | Fields                                           | Description                              |
| ----------- | ------------------------------------------------ | ---------------------------------------- |
| play        | —                                                | Start or resume playback                 |
| pause       | —                                                | Pause playback                           |
| stop        | —                                                | Stop playback                            |
| seek        | `position_secs: f64`                             | Seek to position (seconds)               |
| set_volume  | `volume: u8` (0–100)                             | Set volume level                         |
| set_queue   | `file_paths: [string]`, `projection_generation`  | Replace the current MPD projection queue |
| add         | `file_paths: [string]`                           | Append tracks to the current projection  |
| remove      | `index: usize`                                   | Remove track at projection position      |
| move_track  | `from: usize`, `to: usize`                       | Move track within the projection         |

### 1.4 NodeStateUpdate (Node → Server)

Periodic state update from the node's audio backend.

```json
{ "status": "playing", "position_secs": 72.5, "volume": 75, "mpd_song_index": 2, "projection_generation": 4 }
```

| Field                  | Type             | Description                                         |
| ---------------------- | ---------------- | --------------------------------------------------- |
| status                 | `PlaybackStatus` | Current playback status                             |
| position_secs          | `f64`            | Current position in seconds                         |
| volume                 | `u8` (0–100)     | Current volume                                      |
| mpd_song_index         | `usize?`         | MPD-local song index inside the current projection  |
| projection_generation  | `u64`            | Generation of the projection this observation uses  |

---

## 2. Client Protocol

WebSocket JSON protocol for clients (web, TUI, custom). Defined in
[`kanade-adapter-ws/command.rs`](../kanade-adapter-ws/src/command.rs).

**Server endpoint**: `WS_ADDR` (default `ws://HOST:8080`, shared with nodes)

The server identifies a connection as a client by the first message: if it parses as a `WsCommand` (has `"cmd"` tag) or `WsRequest` (has `"req_id"`), the connection enters client mode.

### 2.1 Connection Lifecycle

```
  Client                                   Server
    │─── WebSocket connect ────────────────→│  (:8080)
    │                                       │
    │←── State snapshot ───────────────────│  (full state pushed immediately)
    │                                       │
    │─── Command / Request ────────────────→│
    │←── State broadcast ──────────────────│  (pushed on every state change)
    │←── Response ─────────────────────────│  (only for request messages)
    │      ...                              │
    │─── WebSocket close ─────────────────→│
```

Clients are stateless. All state is pushed from the server.

### 2.2 Client → Server Messages

Two top-level message shapes (discriminated by the presence of `cmd` vs `req_id`):

#### Commands (fire-and-forget)

Tagged with `"cmd"`. No response is sent.

```json
{ "cmd": "play" }
{ "cmd": "replace_and_play", "tracks": [{ "id": "...", "title": "...", ... }], "index": 0 }
{ "cmd": "add_to_queue", "track": { "id": "...", ... } }
```

| cmd                  | Additional Fields                    | Description           |
| -------------------- | ------------------------------------ | --------------------- |
| `play`               | —                                    | Start/resume playback |
| `pause`              | —                                    | Pause playback        |
| `stop`               | —                                    | Stop playback         |
| `next`               | —                                    | Next track            |
| `previous`           | —                                    | Previous track        |
| `seek`               | `position_secs: f64`                  | Seek to position      |
| `set_volume`         | `volume: u8`                          | Set volume (0–100)    |
| `set_repeat`         | `repeat: RepeatMode`                  | Set repeat mode       |
| `set_shuffle`        | `shuffle: bool`                       | Toggle shuffle        |
| `select_node`        | `node_id: string`                     | Select output node    |
| `add_to_queue`       | `track: Track`                        | Add single track      |
| `add_tracks_to_queue`| `tracks: [Track]`                     | Add multiple tracks   |
| `play_index`         | `index: usize`                        | Play track at index   |
| `replace_and_play`   | `tracks: [Track]`, `index: usize`     | Replace queue and play from index |
| `remove_from_queue`  | `index: usize`                        | Remove track at index |
| `move_in_queue`      | `from: usize`, `to: usize`           | Reorder track       |
| `clear_queue`        | —                                    | Clear entire queue    |

#### Requests (require response)

Tagged with `"req"` and `"req_id"`. The server replies with a matching `req_id`.

```json
{ "req_id": 1, "req": "get_albums" }
{ "req_id": 2, "req": "get_album_tracks", "album_id": "abc123" }
{ "req_id": 3, "req": "search", "query": "Neru" }
{ "req_id": 4, "req": "get_queue" }
```

| req                 | Additional Fields      | Response type     |
| ------------------- | ---------------------- | ----------------- |
| `get_albums`        | —                      | `albums`          |
| `get_album_tracks`  | `album_id`             | `album_tracks`    |
| `get_artists`       | —                      | `artists`         |
| `get_artist_albums` | `artist`               | `artist_albums`   |
| `get_artist_tracks` | `artist`               | `artist_tracks`   |
| `get_genres`        | —                      | `genres`          |
| `get_genre_albums`  | `genre`                | `genre_albums`    |
| `get_genre_tracks`  | `genre`                | `genre_tracks`    |
| `search`            | `query`                | `search_results`  |
| `get_queue`         | —                      | `queue`           |

### 2.3 Server → Client Messages

Tagged with `"type"`.

#### State Broadcast

Pushed to **all** connected clients on every state change.

```json
{
  "type": "state",
  "state": {
    "nodes": [...],
    "selected_node_id": "browser-desktop-abc123",
    "queue": [ { "id": "...", "title": "...", "artist": "...", ... } ],
    "current_index": 0,
    "shuffle": false,
    "repeat": "all"
  }
}
```

#### Response

Replies to request messages. The `data` field contains the response variant.

```json
{ "type": "response", "req_id": 1, "data": { "albums": [ ... ] } }
{ "type": "response", "req_id": 2, "data": { "tracks": [ ... ] } }
{ "type": "response", "req_id": 4, "data": { "tracks": [ ... ], "current_index": 0 } }
```

| Response variant   | Fields                                       |
| ------------------ | -------------------------------------------- |
| `albums`           | `{ "albums": [Album] }`                     |
| `album_tracks`     | `{ "tracks": [Track] }`                     |
| `artists`          | `{ "artists": [string] }`                   |
| `artist_albums`    | `{ "albums": [Album] }`                     |
| `artist_tracks`    | `{ "tracks": [Track] }`                     |
| `genres`           | `{ "genres": [string] }`                    |
| `genre_albums`     | `{ "albums": [Album] }`                     |
| `genre_tracks`     | `{ "tracks": [Track] }`                     |
| `search_results`   | `{ "tracks": [Track] }`                     |
| `queue`            | `{ "tracks": [Track], "current_index": usize? }` |

---

## 3. Media Surface

HTTP surface for media file delivery to clients. Serves tracks and artwork by
stable IDs backed by the library database.

**Server endpoint**: Unified with WS on `BIND_ADDR` (default `http://HOST:8080`)

### 3.0 Authentication (Session Cookie)

All `/media/*` requests require a valid session cookie. The server generates a per-client session token when a WebSocket connection is established and stores it in an in-memory key store.

**Session lifecycle:**
1. Client connects via `/ws` (mTLS or direct)
2. Server generates a unique session token (UUID)
3. Server sends `{"type":"media_auth","media_auth_key":"<hex>","media_auth_key_id":"<uuid>"}` to the client
4. Client sets `kanade_session=<uuid>` cookie via `HTTPCookieStorage` (Secure, Path=/media)
5. AVPlayer and URLSession automatically include the cookie on `/media/*` requests
6. Server validates the cookie against the key store
7. On WebSocket disconnect, the session token is revoked

**Cookie properties:**
- Name: `kanade_session`
- Value: session token (UUID)
- Path: `/media`
- Secure: yes
- HttpOnly: no (required so the app can manage it)

**Request format:**
```
GET /media/tracks/<track_id> HTTP/1.1
Host: HOST:8080
Cookie: kanade_session=<uuid>
Range: bytes=0-1023  (optional)
```

**Verification:**
- Extract `kanade_session` from Cookie header
- Look up session token in key store
- If not found → HTTP 403
- No expiry on cookie itself — validity is tied to the WebSocket session

### 3.1 Request Format

```
GET /media/tracks/<track_id> HTTP/1.1
Host: HOST:8081
Range: bytes=0-1023  (optional, for partial content)
```

```http
GET /media/art/<album_id> HTTP/1.1
Host: HOST:8081
```

### 3.2 Response Format

**Success** (HTTP 200 or 206 for range requests):

```
HTTP/1.1 200 OK
Content-Type: audio/flac
Content-Length: 12345678
Accept-Ranges: bytes

<binary audio data>
```

**Partial Content** (HTTP 206):

```
HTTP/1.1 206 Partial Content
Content-Type: audio/flac
Content-Range: bytes 0-1023/12345678
Content-Length: 1024

<partial binary data>
```

### 3.3 Resource Mapping

- `/media/tracks/<track_id>` resolves the track via the database and serves the
  underlying audio file with range support.
- `/media/art/<album_id>` serves album artwork from either a discovered artwork
  path or embedded cover art extracted from the first track in the album.

---

## External Protocols

These protocols are implemented as adapters but are not part of the Kanade
Protocol family. They provide interoperability with external control systems.

## OpenHome / UPnP

SOAP/XML protocol for UPnP control points. Implemented in
[`kanade-adapter-openhome`](../kanade-adapter-openhome/src/).

**Server endpoint**: `OH_ADDR` (default `http://HOST:8090`)

### Service Types

| Service                    | URN                                         |
| -------------------------- | ------------------------------------------- |
| Transport                  | `urn:av-openhome-org:service:Transport:1`   |
| Volume                     | `urn:av-openhome-org:service:Volume:1`      |

### SOAP Actions

All actions target the `"default"` node.

**Request format** — HTTP POST with headers:

```
POST / HTTP/1.1
Content-Type: text/xml; charset="utf-8"
SOAPAction: "urn:av-openhome-org:service:Transport:1#Play"
Content-Length: ...

<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:Play xmlns:u="urn:av-openhome-org:service:Transport:1"/>
  </s:Body>
</s:Envelope>
```

| Action               | Parameters              | Description       |
| -------------------- | ----------------------- | ----------------- |
| `Play`               | —                       | Start playback    |
| `Pause`              | —                       | Pause playback    |
| `Stop`               | —                       | Stop playback     |
| `Next`               | —                       | Next track        |
| `Previous`           | —                       | Previous track    |
| `SeekSecondAbsolute` | `<Value>120</Value>`    | Seek (seconds)    |
| `SetVolume`          | `<Value>75</Value>`     | Set volume (0–100)|

### Response Format

**Success** (HTTP 200):

```xml
<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:PlayResponse xmlns:u="urn:av-openhome-org:service:Transport:1"/>
  </s:Body>
</s:Envelope>
```

**Error** (HTTP 200 with SOAP fault):

```xml
<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <s:Fault>
      <faultcode>s:Client</faultcode>
      <faultstring>UPnPError</faultstring>
      <detail>
        <UPnPError xmlns="urn:schemas-upnp-org:control-1-0">
          <errorCode>402</errorCode>
          <errorDescription>Invalid Args</errorDescription>
        </UPnPError>
      </detail>
    </s:Fault>
  </s:Body>
</s:Envelope>
```

| Error Code | Meaning        |
| ---------- | -------------- |
| 402        | Invalid Args   |
| 501        | Action Failed  |

---

## Shared Types

### Track

```json
{
  "id": "sha256hex",
  "file_path": "/music/Album/01-track.flac",
  "album_id": "sha256hex",
  "title": "Track Name",
  "artist": "Artist Name",
  "album_artist": "Album Artist",
  "album_title": "Album Title",
  "composer": "Composer",
  "genre": "Genre",
  "track_number": 1,
  "disc_number": 1,
  "duration_secs": 245.93,
  "format": "FLAC",
  "sample_rate": 48000
}
```

Fields with `null` values are omitted from JSON (`skip_serializing_if = "Option::is_none"`).

### Album

```json
{
  "id": "sha256hex",
  "dir_path": "/music/Album",
  "title": "Album Title",
  "artwork_path": "/music/Album/cover.jpg"
}
```

### Node

```json
{
  "id": "default",
  "name": "Living Room",
  "connected": true,
  "status": "playing",
  "position_secs": 72.5,
  "volume": 75
}
```

### Enumerations

**PlaybackStatus**: `"stopped"` | `"playing"` | `"paused"` | `"loading"`

**RepeatMode**: `"off"` | `"one"` | `"all"`
