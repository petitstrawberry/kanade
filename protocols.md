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
| media_auth_key  | string? | Hex-encoded HMAC signing key (nodes only, for client-side URL signing) |
| media_auth_key_id | string? | UUID key identifier (nodes only) |

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
| `sign_urls`         | `paths: [string]`      | `signed_urls`     |

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
| `signed_urls`      | `{ "urls": { string: string } }` |

---

## 3. Media Surface

HTTP surface for media file delivery to clients. Serves tracks and artwork by
stable IDs backed by the library database.

**Server endpoint**: Unified with WS on `BIND_ADDR` (default `http://HOST:8080`)

### 3.0 Authentication (Signed URLs)

All `/media/*` requests require a valid signed URL. The server generates a per-session HMAC-SHA256 key when a WebSocket connection is established. Signing keys never leave the server — clients request signed URLs from the server over the WebSocket connection.

**Session lifecycle:**
1. Client connects via `/ws`
2. Server generates a per-session HMAC-SHA256 key (32 bytes) with a UUID key identifier
3. Server sends `{"type":"media_auth","media_auth_key_id":"<uuid>"}` to the client (key stays server-side)
4. Client requests signed URLs via the `sign_urls` WS request
5. Server returns fully signed URLs with embedded expiry and HMAC signature
6. Client uses signed URLs directly in `<img>`, `<audio>`, or HTTP requests
7. On WebSocket disconnect, the signing key is revoked

**Signed URL format:**
```
/media/<path>?kid=<key_id>&exp=<unix_timestamp>&sig=<hmac_hex>
```

| Param | Description |
|-------|-------------|
| `kid` | UUID key identifier (tells server which key to use) |
| `exp` | Unix timestamp expiry (15 minutes from signing) |
| `sig` | HMAC-SHA256 signature hex-encoded |

**Signing algorithm:**
```
message = "GET:{path}:{exp}"
signature = HMAC-SHA256(key_bytes, message)
```

**Requesting signed URLs (client → server):**
```json
{ "req_id": 1, "req": "sign_urls", "paths": ["/media/art/abc123", "/media/tracks/def456"] }
```

**Response (server → client):**
```json
{ "type": "response", "req_id": 1, "data": { "signed_urls": {
  "/media/art/abc123": "https://host/media/art/abc123?kid=uuid&exp=1234567890&sig=hex",
  "/media/tracks/def456": "https://host/media/tracks/def456?kid=uuid&exp=1234567890&sig=hex"
}}}
```

**Verification (server-side):**
1. Extract `kid`, `exp`, `sig` from query parameters
2. Reject if `exp` is in the past
3. Look up key bytes by `kid` in the key store
4. Compute `HMAC-SHA256(key, "GET:{path}:{exp}")`
5. Constant-time compare with provided `sig`
6. If mismatch → HTTP 403

**Security properties:**
- Signing keys are never exposed to clients
- URLs expire after 15 minutes
- Per-session keys, revoked on WebSocket disconnect
- Constant-time signature comparison prevents timing attacks
- `Referrer-Policy: no-referrer` on all media elements prevents URL leakage via Referer headers

**Node connections** receive the raw signing key (`media_auth_key`) in their registration ack and sign URLs client-side, since nodes construct media URLs independently without a persistent request/response channel.

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

### 3.4 HLS Streaming (fMP4)

HTTP Live Streaming endpoints for on-demand audio streaming. Audio files are remuxed (not transcoded) into fMP4 segments and cached on disk. First request generates segments; subsequent requests serve from cache.

**Endpoints:**

| Endpoint | Content-Type | Description |
|---|---|---|
| `GET /media/hls/{track_id}/{variant}/index.m3u8` | `application/vnd.apple.mpegurl` | HLS playlist (VOD, EXT-X-INDEPENDENT-SEGMENTS) |
| `GET /media/hls/{track_id}/{variant}/init.mp4` | `video/mp4` | Initialization segment (ftyp + moov) |
| `GET /media/hls/{track_id}/{variant}/seg{N}.m4s` | `video/mp4` | Media segment N (moof + mdat) |

**Authentication:** Same signed URL mechanism as other /media/* endpoints (kid/exp/sig query parameters).

**Variant:** Quality profile name, used for future ABR support. Current valid value: `lossless` (remux without transcoding).

**Supported formats for remux:**

| Source Format | fMP4 Codec | FourCC |
|---|---|---|
| FLAC | FLAC (lossless) | `fLaC` |
| AAC (ADTS / M4A) | AAC | `mp4a` |
| ALAC (M4A) | ALAC | `alac` |
| MP3 | MP3 | `.mp3` |
| WAV (PCM) | LPCM | `lpcm` |
| AIFF (PCM) | LPCM | `lpcm` |
| Opus (Ogg) | Opus | `Opus` |

Formats not listed (DSD, APE, Ogg Vorbis) are not supported for HLS streaming.

**On-demand generation:**
1. Client requests `index.m3u8` with signed URL
2. Server checks cache (`.hls-cache/{track_id}/{variant}/`)
3. Cache miss: remux source file into fMP4 segments (6-second duration)
4. Cache hit: serve cached segments directly
5. LRU eviction when cache exceeds configured limit (default 10GB)

**Future ABR extension:**
- `lossless` variant: source format remuxed as-is (current)
- `high` variant: AAC 256kbps transcoded (future)
- `low` variant: AAC 128kbps transcoded (future)
- `master.m3u8`: master playlist referencing all variants (future)
- Directory structure (`{track_id}/{variant}/`) already supports multiple variants

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
