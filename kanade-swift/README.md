# KanadeSwift

Minimal native client scaffold for Kanade using Swift and SwiftUI.

## What is included

- `KanadeSwift` — shared Swift package with Codable protocol models and a lightweight WebSocket client for the Kanade client subprotocol
- `Examples/KanadeNativeApp` — a small SwiftUI app shell that targets iOS and macOS and reuses the shared package

## Server endpoints

- WebSocket control/state: `ws://HOST:8080`
- HTTP artwork/audio: `http://HOST:8081`

## Opening in Xcode

1. Open `kanade-swift/Package.swift` in Xcode.
2. Select the `KanadeNativeApp` scheme.
3. Run on macOS or an iOS simulator/device.

## Package validation

The shared protocol layer is covered by focused Swift tests:

```sh
cd kanade-swift
swift test
```
