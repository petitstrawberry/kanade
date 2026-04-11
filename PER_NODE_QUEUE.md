# Kanade — Per-Node Queue Architecture

## Overview

Kanade uses a **per-node queue model**: each output node (speakers, local device)
maintains its own independent queue, playback position, and transport state.
The server is the single source of truth for all node states.

This replaces the previous global-queue design.  Key motivations:

- **No arbitration complexity** — nodes never conflict with each other.
- **Concurrent playback** — living room plays BGM while iPhone listens privately.
- **Seamless handoff** — copy queue+position between nodes, no merge needed.
- **Local playback as a node variant** — the server tracks local playback state
  in real-time, not as a best-effort notification.

## Architecture

```
  ┌─────────────────────────────────────────────────────────────────┐
  │                        Kanade Server                           │
  │                                                                 │
  │  node_states: {                                                 │
  │    "living-room":  { queue: [A,B,C,D], index: 1, pos: 72.0,   │
  │                       status: playing, volume: 75, repeat: all }│
  │    "bedroom":      { queue: [X,Y,Z],   index: 0, pos: 0.0,    │
  │                       status: stopped, volume: 50, repeat: off }│
  │    "iphone-kana":  { queue: [A,B,C,D], index: 3, pos: 45.0,   │
  │                       status: playing, volume: 80, repeat: off,│
  │                       node_type: local }                        │
  │  }                                                              │
  │  selected_node_id: "living-room"                                │
  └─────────────────────────────────────────────────────────────────┘
          │                    │                    │
          ▼                    ▼                    ▼
   ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐
   │ Living Room │    │  Bedroom    │    │  iPhone (local)      │
   │ Node (MPD)  │    │ Node (MPD)  │    │  AVQueuePlayer       │
   │             │    │             │    │  sends NodeStateUpd  │
   │ Server sends│    │ Server sends│    │  every ~1s            │
   │ commands    │    │ commands    │    │  Server sends nothing │
   └─────────────┘    └─────────────┘    └─────────────────────┘
```

## Node Types

| Type | Server sends commands | Server receives state | Lifetime |
|------|----------------------|----------------------|----------|
| `remote` | Yes (play/pause/seek/set_queue...) | NodeStateUpdate via WS | Persistent |
| `local` | **No** | NodeStateUpdate from client | Ephemeral (WS session) |

A local node is a **read-only entry** in the server's node map.  The server
tracks its state for visibility and handoff purposes but never issues commands
to it.  The client manages playback entirely on-device.

## Protocol Changes

### State Broadcast (Server → Client)

```json
{
  "type": "state",
  "state": {
    "nodes": [
      {
        "id": "living-room",
        "name": "Living Room",
        "connected": true,
        "node_type": "remote",
        "queue": [ { "id": "...", "title": "...", ... } ],
        "current_index": 1,
        "position_secs": 72.5,
        "status": "playing",
        "volume": 75,
        "repeat": "all",
        "shuffle": false
      },
      {
        "id": "local-abc123",
        "name": "Kana's iPhone",
        "connected": true,
        "node_type": "local",
        "queue": [ ... ],
        "current_index": 3,
        "position_secs": 45.0,
        "status": "playing",
        "volume": 80,
        "repeat": "off",
        "shuffle": false
      }
    ],
    "selected_node_id": "living-room"
  }
}
```

The `Node` model now includes `queue`, `current_index`, `repeat`, `shuffle`
fields that previously lived on `PlaybackState`.

`PlaybackState` is simplified:
```json
{
  "nodes": [...],
  "selected_node_id": "living-room"
}
```

### New Client Commands

```json
{ "cmd": "local_session_start", "device_name": "Kana's iPhone" }
{ "cmd": "local_session_stop" }
{ "cmd": "local_session_update", "current_index": 3, "position_secs": 45.0,
  "status": "playing", "volume": 80, "queue": [...], "repeat": "off", "shuffle": false }
```

| cmd | Fields | Description |
|-----|--------|-------------|
| `local_session_start` | `device_name` | Register a local playback session |
| `local_session_stop` | — | Remove local session |
| `local_session_update` | full state snapshot | Update local session state |

The server assigns a node ID (prefixed `local-`) and includes it in the
`local_session_start` response.  Subsequent `local_session_update` messages
update the node's state in the server's node map and broadcast to all clients.

### Handoff

Handoff is a **node-to-node queue copy** operation:

```json
{ "cmd": "handoff", "from_node_id": "local-abc123", "to_node_id": "living-room" }
```

The server copies `queue`, `current_index`, `position_secs` from the source
node to the target node, then issues appropriate commands to the target
(remote) node.

No client-side queue management needed for the transfer — the server has
both nodes' queues.

## Client Behavior

### Local Playback Lifecycle

```
1. User taps "This Device"
   → Client sends: local_session_start
   → Server responds with: { node_id: "local-xyz789" }
   → Client stores localNodeId

2. Local playback begins
   → Client sends: local_session_update (every ~1s while playing)
   → Server updates node map, broadcasts to all clients
   → Other clients see "Kana's iPhone: Track D playing" in output picker

3. User taps "Living Room" (handoff)
   → Client sends: handoff(from_node_id: localNodeId, to_node_id: "living-room")
   → Server copies queue+position to living-room node
   → Server sends commands to living-room node
   → Client sends: local_session_stop
   → Client stops local playback

4. WS disconnects (tunnel, background)
   → Client stops sending updates
   → Server marks local node with last-known state
   → On reconnect: client sends local_session_update with current state

5. User stops local playback
   → Client sends: local_session_stop
   → Server removes local node from node map
```

### Backward Compatibility

The `PlaybackState` decoder must handle both old and new formats:

- **Old format**: has top-level `queue`, `current_index`, `shuffle`, `repeat`
  → wrap into a synthetic selected-node state
- **New format**: nodes carry their own queues
  → use directly

This allows gradual server migration — old servers continue to work with
new clients.

## Queue Operations

When operating on a **remote** node, all queue commands (`replace_and_play`,
`add_to_queue`, `remove_from_queue`, etc.) are sent to the server which
applies them to the selected node's queue.

When operating on the **local** node, queue operations are purely local
(`LocalQueue`). The updated state is propagated to the server via
`local_session_update`.

No queue operation ever touches another node's queue implicitly.

## Handoff Scenarios

| Scenario | Flow |
|----------|------|
| Remote → Local | `handoff(remote_id → local_id)` or client-side copy |
| Local → Remote | `handoff(local_id → remote_id)` |
| Remote → Remote | `handoff(node_a → node_b)` |
| Local → Local (same device) | N/A (only one local session per client) |

All handoffs are explicit user actions.  No automatic sync.

## Files

### KanadeKit

| File | Change |
|------|--------|
| `Models/Node.swift` | Add `queue`, `currentIndex`, `repeat`, `shuffle`, `nodeType` |
| `Models/PlaybackState.swift` | Remove `queue`, `currentIndex`, `shuffle`, `repeat` |
| `Models/Enums.swift` | Add `NodeType` enum |
| `Protocol/WsCommand.swift` | Add `localSessionStart`, `localSessionStop`, `localSessionUpdate`, `handoff` |
| `Client/KanadeClient.swift` | Add `localSessionStart()`, `localSessionStop()`, `localSessionUpdate()`, `handoff()` |

### KanadeApp

| File | Change |
|------|--------|
| `App/AppState.swift` | `effectiveQueue` → selected node's queue; local session lifecycle |
| `Playback/LocalPlaybackController.swift` | Periodic `localSessionUpdate` (~1s); start/stop lifecycle |
| `Components/OutputPickerMenuContent.swift` | Show local nodes in output picker with "playing" status |
| `Features/Nodes/NodesView.swift` | Display per-node queue info |

## Migration Path

1. **KanadeKit models** — Add new fields to `Node`, deprecate old `PlaybackState` fields
2. **KanadeKit protocol** — Add new commands, keep old ones working
3. **KanadeApp** — Update AppState to read from per-node state
4. **Server (Rust)** — Refactor `kanade-core` State to per-node queues
5. **Server protocol** — Update state broadcast format
6. **Remove backward compat** — Once server is deployed
