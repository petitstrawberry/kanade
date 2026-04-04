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

    var currentNode: Node? {
        playbackState.nodes.first(where: { $0.id == playbackState.selectedNodeID }) ?? playbackState.nodes.first
    }

    var isPlaying: Bool {
        currentNode?.status == .playing
    }

    var currentTrackTitle: String {
        playbackState.currentTrack?.title ?? "Nothing Playing"
    }

    var currentTrackSubtitle: String {
        if let currentTrack = playbackState.currentTrack {
            return currentTrack.artist ?? currentTrack.albumTitle ?? "Choose an album to start playback"
        }

        return "Choose an album to start playback"
    }

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
            let command: KanadeCommand = isPlaying ? .pause : .play
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
                    connectionSection
                }

                Section("Albums") {
                    ForEach(model.albums) { album in
                        albumSidebarRow(album)
                            .tag(album)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("Kanade")
        } detail: {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    if let album = model.selectedAlbum {
                        albumHeader(album)
                        tracksSection
                    } else {
                        ContentUnavailableView("Select an Album", systemImage: "music.note.house")
                            .frame(maxWidth: .infinity, minHeight: 360)
                    }

                    if let errorMessage = model.errorMessage {
                        ContentUnavailableView {
                            Label("Connection Error", systemImage: "wifi.exclamationmark")
                        } description: {
                            Text(errorMessage)
                        }
                    }
                }
                .padding(24)
            }
            .background(backgroundStyle)
            .navigationTitle(model.selectedAlbum?.title ?? "Browse")
            .safeAreaInset(edge: .bottom) {
                nowPlaying
                    .padding(.horizontal, 20)
                    .padding(.bottom, 12)
            }
            .task(id: model.selectedAlbum?.id) {
                guard let album = model.selectedAlbum else {
                    return
                }

                if album.id != model.albumTracks.first?.albumID {
                    try? await model.selectAlbum(album)
                }
            }
        }
    }

    private var connectionSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("ws://127.0.0.1:8080", text: $model.serverURLString)
                .textFieldStyle(.roundedBorder)

            Button(model.isConnecting ? "Connecting…" : "Connect", action: model.connect)
                .buttonStyle(.borderedProminent)
                .disabled(model.isConnecting)

            if let node = model.currentNode {
                Divider()
                LabeledContent("Node", value: node.name)
                LabeledContent("Status", value: node.status.rawValue.capitalized)
            }
        }
    }

    private func albumSidebarRow(_ album: Album) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(album.title ?? "Untitled Album")
                .lineLimit(1)
            Text(album.dirPath)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    private var backgroundStyle: some ShapeStyle {
        .background
    }

    private var nowPlaying: some View {
        HStack(spacing: 16) {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.quaternary)
                .frame(width: 52, height: 52)
                .overlay {
                    Image(systemName: "music.note")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }

            VStack(alignment: .leading, spacing: 4) {
                Text(model.currentTrackTitle)
                    .font(.headline)
                    .lineLimit(1)
                Text(model.currentTrackSubtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 12)

            HStack(spacing: 10) {
                playerButton(systemImage: "backward.fill", action: model.previousTrack)
                playerButton(systemImage: model.isPlaying ? "pause.fill" : "play.fill", action: model.togglePlayback, prominent: true)
                playerButton(systemImage: "forward.fill", action: model.nextTrack)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .shadow(color: .black.opacity(0.08), radius: 20, y: 4)
    }

    private func albumHeader(_ album: Album) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .bottom, spacing: 24) {
                artworkView(for: album)
                albumMeta(album)
                Spacer(minLength: 0)
            }

            VStack(alignment: .leading, spacing: 20) {
                artworkView(for: album)
                albumMeta(album)
            }
        }
        .padding(28)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }

    private func artworkView(for album: Album) -> some View {
        AsyncImage(url: model.artworkURL(for: album)) { image in
            image
                .resizable()
                .scaledToFill()
        } placeholder: {
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .fill(.tertiary)
                .overlay {
                    Image(systemName: "music.note.list")
                        .font(.system(size: 42))
                        .foregroundStyle(.secondary)
                }
        }
        .frame(width: 220, height: 220)
        .clipShape(RoundedRectangle(cornerRadius: 28, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 24, y: 12)
    }

    private func albumMeta(_ album: Album) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Album")
                .font(.headline)
                .foregroundStyle(.secondary)
            Text(album.title ?? "Untitled Album")
                .font(.system(size: 34, weight: .bold))
            Text(album.dirPath)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(3)

            HStack(spacing: 12) {
                Button("Play", action: model.playAlbum)
                    .buttonStyle(.borderedProminent)
                Label("\(model.albumTracks.count) songs", systemImage: "music.note.list")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var tracksSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Songs")
                .font(.title2.weight(.semibold))

            LazyVStack(spacing: 10) {
                ForEach(Array(model.albumTracks.enumerated()), id: \.element.id) { index, track in
                    trackRow(track, number: index + 1)
                }
            }
        }
    }

    private func trackRow(_ track: Track, number: Int) -> some View {
        HStack(spacing: 16) {
            Text(number.formatted(.number))
                .font(.headline.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 24, alignment: .trailing)

            VStack(alignment: .leading, spacing: 4) {
                Text(track.title ?? track.filePath)
                    .lineLimit(1)
                Text(track.artist ?? "Unknown Artist")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            if let duration = track.durationSecs {
                Text(formatDuration(duration))
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.secondary)
            }

            Button {
                model.addTrack(track)
            } label: {
                Image(systemName: "plus.circle.fill")
                    .font(.title3)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(.quaternary.opacity(0.4)))
        .contentShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .onTapGesture {
            model.playTrack(track)
        }
    }

    private func playerButton(systemImage: String, action: @escaping () -> Void, prominent: Bool = false) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.headline)
                .frame(width: 36, height: 36)
        }
        .buttonStyle(.plain)
        .background(prominent ? Color.accentColor : Color.secondary.opacity(0.14), in: Circle())
        .foregroundStyle(prominent ? .white : .primary)
    }

    private func formatDuration(_ duration: Double) -> String {
        let totalSeconds = Int(duration.rounded())
        let minutes = totalSeconds / 60
        let seconds = totalSeconds % 60
        return "\(minutes):\(String(format: "%02d", seconds))"
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
