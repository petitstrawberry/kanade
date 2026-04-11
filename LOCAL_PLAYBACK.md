# Kanade — Local Playback Design

## Overview

Local playback enables Kanade clients (iOS, macOS) to render audio directly
on the device instead of controlling a remote output node.  The device streams
audio from the Media Surface (`:8081`) and manages transport state locally.

This is a **separate mode** from remote-node control — the server remains the
single source of truth for library metadata and remote-node playback, but the
local device owns its own transport authority.

## Architecture

```
  ┌──────────────────────────────────────────────────────┐
  │                     KanadeApp                        │
  │                                                      │
  │  ┌─────────────┐    ┌────────────────────────────┐  │
  │  │ KanadeClient│    │ LocalPlaybackController     │  │
  │  │ (WS :8080)  │    │ ┌────────────────────────┐ │  │
  │  │             │    │ │ AudioRenderer (protocol)│ │  │
  │  │ Library     │    │ │  ├─ AVQueuePlayerRender│ │  │
  │  │ browse/query│    │ │  └─ (future: Engine)   │ │  │
  │  │ Remote node │    │ └────────────────────────┘ │  │
  │  │ control     │    │ LocalQueue                  │  │
  │  └──────┬──────┘    │ repeat / shuffle            │  │
  │         │           └──────────┬─────────────────┘  │
  │         │                      │                     │
  │         │   MediaClient (HTTP :8081)                 │
  │         │   trackURL(trackId) → AVPlayer source      │
  │         │   artwork(albumId)  → cover art            │
  │                                                      │
  │  ┌─────────────────────────────────────────────────┐ │
  │  │                  AppState                       │ │
  │  │  playbackMode: .local | .remote                  │ │
  │  │  Routes perform*() to local or remote            │ │
  │  └─────────────────────────────────────────────────┘ │
  └──────────────────────────────────────────────────────┘
```

## Principles

1. **Local transport authority** — the device owns play/pause/seek/volume when
   in local mode. No server round-trip for transport control.
2. **Server as library** — the server provides library metadata, search, and
   album art.  Local playback streams from Media Surface.
3. **Independent queues** — local mode has its own queue, independent of the
   server's shared queue.  Handoff copies between them.
4. **Explicit handoff** — switching between local and remote playback is a
   user action, not automatic sync.

## Playback Modes

| Mode | Transport authority | Queue owner | Audio source |
|------|--------------------|-------------|--------------|
| Remote | Server (via WS) | Server | Output node |
| Local | Device (AVPlayer) | Device | Media Surface (:8081) |

## Audio Renderer Architecture

The `AudioRenderer` protocol abstracts the underlying audio backend:

```swift
protocol AudioRenderer: AnyObject {
    var state: RendererState { get }
    func loadTracks(_ tracks: [Track], startIndex: Int)
    func play()
    func pause()
    func stop()
    func seek(to positionSecs: Double)
    var duration: Double { get }
    var position: Double { get }
    var isPlaying: Bool { get }
}
```

### AVQueuePlayerRenderer (default)

Uses `AVQueuePlayer` with gapless playback support:

- **Gapless playback**: Pre-loads the next `AVPlayerItem` while the current
  track is still playing.  The queue is maintained with up to N+2 items
  buffered.
- **PCM formats**: FLAC, ALAC, WAV, AIFF, MP3, AAC, Ogg Vorbis, Opus —
  handled natively by AVFoundation.
- **Sample rate switching**: Configures `AVAudioSession` sample rate per track
  when the sample rate changes between tracks.

### Future: AVAudioEngineRenderer

For formats requiring raw PCM output control (DoP, DSD native):

```
AVAudioEngine
  → inputNode (or playerNode)
  → format converter (if needed)
  → outputNode

DoP (DSD over PCM):
  DSD bytes → DoP marker framing → PCM32 @ 176.4/352.8 kHz
  → AVAudioEngine outputNode

DSD Native (future):
  Requires USB DAC with DSD support via CoreAudio
  → AudioDevice custom stream format
```

The `AudioRenderer` protocol allows swapping backends without touching the
queue management or transport logic.

## Gapless Playback

AVQueuePlayer provides seamless transitions when items are pre-buffered:

1. When track N begins playing, item N+1 is already in the queue.
2. AVFoundation pre-buffers the next item during the final seconds of N.
3. Crossfade is NOT applied — bit-perfect gapless transition.
4. The `LocalPlaybackController` observes `currentItem` changes and inserts
   the next track when the queue runs low.

```
Timeline:
  [Track A playing...] [Track A ends] [Track B starts immediately]
                        ↑ no silence gap
```

## Queue Management

### Local Queue

```swift
struct LocalQueueState {
    var tracks: [Track]
    var currentIndex: Int?
    var repeatMode: RepeatMode
    var shuffleEnabled: Bool
    // derived playback order (respects shuffle)
    var playbackOrder: [Int]
}
```

- Owned entirely by the client.
- Shuffle generates a random `playbackOrder` over track indices.
- Repeat modes: `.off` (stop at end), `.one` (restart current), `.all` (loop).

### Handoff: Remote → Local

```
User taps "Play on This Device"
  → Fetch server queue: client.getQueue()
  → Fetch current position from selected node
  → Import into LocalQueueState
  → Start local playback from that position
```

### Handoff: Local → Remote

```
User taps "Play on [Node Name]"
  → Export local queue + position to server:
      client.replaceAndPlay(tracks: localQueue, index: currentIndex)
      client.seek(to: localPosition)
      client.selectNode(nodeId)
```

## Session Reporting (Future)

Optional telemetry for cross-client visibility:

```json
{
  "cmd": "sync_local_session",
  "session_id": "stable-uuid-per-install",
  "device_name": "petitstrawberry's iPhone",
  "queue": [ ... ],
  "current_index": 3,
  "position_secs": 127.4,
  "repeat": "all",
  "shuffle": false,
  "timestamp": 1712841600
}
```

This is **non-authoritative** — the server uses it only for display
("Kana's iPhone is playing X") and as a handoff source.  Other clients
cannot control local playback remotely.

## Background Audio

### iOS

- `UIBackgroundModes: [audio]` in Info.plist
- `AVAudioSession.sharedInstance().setCategory(.playback)`
- `MPNowPlayingInfoCenter` — updates lock screen with track metadata and artwork
- `MPRemoteCommandCenter` — handles play/pause/next/previous/seek from lock
  screen, control center, and headphones

### macOS

- `MPNowPlayingInfoCenter` — shows in Control Center
- `MPRemoteCommandCenter` — media key handling

## DoP (DSD over PCM) Path

DSD files cannot be decoded by AVPlayer.  Future `AVAudioEngineRenderer` will
handle this:

```
DSD file (e.g., .dsf, .dff)
  → Server sends via /media/tracks/:id (raw bytes)
  → Client downloads raw DSD frames
  → DoP framing:
      marker (0x05/0xFA alternating) + DSD 8-bit chunks
      → packed into 32-bit PCM frames
      → output at 176.4 kHz (DSD64) or 352.8 kHz (DSD128)
  → AVAudioEngine outputNode sends to DAC
  → DAC detects DoP markers and switches to DSD mode
```

Requirements:
- Server must serve raw DSD data (not transcode to PCM)
- Client needs `AVAudioEngineRenderer` with DoP framer
- DAC must support DoP (most USB DACs do)
- `AVAudioSession` must be configured for the target sample rate

## Files (KanadeApp)

| File | Role |
|------|------|
| `App/AppState.swift` | Routes actions to local or remote; owns playback mode |
| `Playback/LocalPlaybackController.swift` | Queue + transport + renderer coordination |
| `Playback/LocalQueue.swift` | Queue state, shuffle, repeat logic |
| `Playback/AudioRenderer.swift` | Protocol for audio backends |
| `Playback/AVQueuePlayerRenderer.swift` | AVPlayer-based renderer (PCM, gapless) |
| `Playback/NowPlayingManager.swift` | MPNowPlayingInfoCenter + MPRemoteCommandCenter |
