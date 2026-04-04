#if canImport(SwiftUI)
import KanadeSwift
import SwiftUI

@available(iOS 17.0, macOS 14.0, *)
@main
struct KanadeNativeApp: App {
    @State private var model = KanadeAppModel()

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
    }
}

@available(iOS 17.0, macOS 14.0, *)
@MainActor
@Observable
final class KanadeAppModel {
    var serverURLString = "ws://127.0.0.1:8080"
    var albums: [Album] = []
    var selectedAlbum: Album?
    var albumTracks: [Track] = []
    var playbackState = PlaybackState(nodes: [])
    var isConnecting = false
    var errorMessage: String?

    private var client: KanadeClient?

    func connect() {
        guard let url = URL(string: serverURLString) else {
            errorMessage = "Invalid server URL"
            return
        }

        isConnecting = true
        errorMessage = nil

        let client = KanadeClient(webSocketURL: url) { [weak self] state in
            self?.playbackState = state
        }
        self.client = client

        Task {
            await client.connect()

            do {
                try await loadAlbums()
            } catch {
                self.errorMessage = error.localizedDescription
            }

            self.isConnecting = false
        }
    }

    func loadAlbums() async throws {
        guard let client else {
            return
        }

        if case let .albums(albums) = try await client.request(.getAlbums) {
            self.albums = albums
            if selectedAlbum == nil {
                selectedAlbum = albums.first
            }

            if let selectedAlbum {
                try await selectAlbum(selectedAlbum)
            }
        }
    }

    func selectAlbum(_ album: Album) async throws {
        guard let client else {
            return
        }

        if case let .albumTracks(tracks) = try await client.request(.getAlbumTracks(albumID: album.id)) {
            selectedAlbum = album
            albumTracks = tracks
        }
    }

    func togglePlayback() {
        guard let client else {
            return
        }

        Task {
            let command: KanadeCommand = playbackState.currentTrack != nil && playbackState.nodes.first?.status == .playing ? .pause : .play
            try? await client.send(command)
        }
    }

    func nextTrack() {
        guard let client else {
            return
        }

        Task {
            try? await client.send(.next)
        }
    }

    func previousTrack() {
        guard let client else {
            return
        }

        Task {
            try? await client.send(.previous)
        }
    }

    func playAlbum() {
        guard let client, !albumTracks.isEmpty else {
            return
        }

        let tracks = albumTracks
        Task {
            try? await client.send(.replaceAndPlay(tracks: tracks, index: 0))
        }
    }

    func addTrack(_ track: Track) {
        guard let client else {
            return
        }

        Task {
            try? await client.send(.addToQueue(track))
        }
    }

    func playTrack(_ track: Track) {
        guard let client, let index = albumTracks.firstIndex(of: track) else {
            return
        }

        let tracks = albumTracks
        Task {
            try? await client.send(.replaceAndPlay(tracks: tracks, index: index))
        }
    }

    func artworkURL(for album: Album) -> URL? {
        guard let client else {
            return nil
        }

        return client.artworkURL(for: album.id)
    }
}

@available(iOS 17.0, macOS 14.0, *)
struct ContentView: View {
    @Bindable var model: KanadeAppModel

    var body: some View {
        NavigationSplitView {
            List(selection: $model.selectedAlbum) {
                Section("Connection") {
                    TextField("ws://127.0.0.1:8080", text: $model.serverURLString)
                        .textFieldStyle(.roundedBorder)

                    Button(model.isConnecting ? "Connecting…" : "Connect", action: model.connect)
                        .disabled(model.isConnecting)
                }

                Section("Albums") {
                    ForEach(model.albums) { album in
                        Label(album.title ?? album.dirPath, systemImage: "square.stack")
                            .tag(album)
                    }
                }
            }
            .navigationTitle("Kanade")
        } detail: {
            VStack(alignment: .leading, spacing: 16) {
                nowPlaying

                if let album = model.selectedAlbum {
                    HStack(spacing: 16) {
                        AsyncImage(url: model.artworkURL(for: album)) { image in
                            image.resizable().scaledToFill()
                        } placeholder: {
                            RoundedRectangle(cornerRadius: 16)
                                .fill(.secondary.opacity(0.15))
                                .overlay(Image(systemName: "music.note.list").font(.largeTitle))
                        }
                        .frame(width: 180, height: 180)
                        .clipShape(RoundedRectangle(cornerRadius: 16))

                        VStack(alignment: .leading, spacing: 12) {
                            Text(album.title ?? "Untitled Album")
                                .font(.title.bold())
                            Text(album.dirPath)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                            Button("Play Album", action: model.playAlbum)
                                .buttonStyle(.borderedProminent)
                        }
                    }

                    List(model.albumTracks) { track in
                        HStack {
                            VStack(alignment: .leading) {
                                Text(track.title ?? track.filePath)
                                Text(track.artist ?? "Unknown Artist")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button {
                                model.addTrack(track)
                            } label: {
                                Image(systemName: "plus.circle")
                            }
                            .buttonStyle(.borderless)
                        }
                        .contentShape(Rectangle())
                        .onTapGesture {
                            model.playTrack(track)
                        }
                    }
                    .listStyle(.plain)
                } else {
                    ContentUnavailableView("Connect to Kanade", systemImage: "music.note.house")
                }

                if let errorMessage = model.errorMessage {
                    Text(errorMessage)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            }
            .padding()
        }
    }

    private var nowPlaying: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Now Playing")
                .font(.headline)
            Text(model.playbackState.currentTrack?.title ?? "Nothing playing")
                .font(.title3)
            Text(model.playbackState.currentTrack?.artist ?? "Select an album to start playback")
                .foregroundStyle(.secondary)
            HStack {
                Button(action: model.previousTrack) {
                    Image(systemName: "backward.fill")
                }
                Button(action: model.togglePlayback) {
                    Image(systemName: model.playbackState.nodes.first?.status == .playing ? "pause.fill" : "play.fill")
                }
                Button(action: model.nextTrack) {
                    Image(systemName: "forward.fill")
                }
            }
            .buttonStyle(.bordered)
        }
    }
}
#else
import Foundation

@main
struct KanadeNativeApp {
    static func main() {
        print("KanadeNativeApp requires SwiftUI and runs on macOS or iOS.")
    }
}
#endif
