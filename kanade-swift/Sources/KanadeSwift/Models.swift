import Foundation

public struct Track: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let filePath: String
    public let albumID: String?
    public let title: String?
    public let artist: String?
    public let albumArtist: String?
    public let albumTitle: String?
    public let composer: String?
    public let genre: String?
    public let trackNumber: UInt32?
    public let discNumber: UInt32?
    public let durationSecs: Double?
    public let format: String?
    public let sampleRate: UInt32?

    public init(
        id: String,
        filePath: String,
        albumID: String? = nil,
        title: String? = nil,
        artist: String? = nil,
        albumArtist: String? = nil,
        albumTitle: String? = nil,
        composer: String? = nil,
        genre: String? = nil,
        trackNumber: UInt32? = nil,
        discNumber: UInt32? = nil,
        durationSecs: Double? = nil,
        format: String? = nil,
        sampleRate: UInt32? = nil
    ) {
        self.id = id
        self.filePath = filePath
        self.albumID = albumID
        self.title = title
        self.artist = artist
        self.albumArtist = albumArtist
        self.albumTitle = albumTitle
        self.composer = composer
        self.genre = genre
        self.trackNumber = trackNumber
        self.discNumber = discNumber
        self.durationSecs = durationSecs
        self.format = format
        self.sampleRate = sampleRate
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case filePath = "file_path"
        case albumID = "album_id"
        case title
        case artist
        case albumArtist = "album_artist"
        case albumTitle = "album_title"
        case composer
        case genre
        case trackNumber = "track_number"
        case discNumber = "disc_number"
        case durationSecs = "duration_secs"
        case format
        case sampleRate = "sample_rate"
    }
}

public struct Album: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let dirPath: String
    public let title: String?
    public let artworkPath: String?

    public init(id: String, dirPath: String, title: String? = nil, artworkPath: String? = nil) {
        self.id = id
        self.dirPath = dirPath
        self.title = title
        self.artworkPath = artworkPath
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case dirPath = "dir_path"
        case title
        case artworkPath = "artwork_path"
    }
}

public enum RepeatMode: String, Codable, CaseIterable, Sendable {
    case off
    case one
    case all
}

public enum PlaybackStatus: String, Codable, CaseIterable, Sendable {
    case stopped
    case playing
    case paused
    case loading
}

public struct Node: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let name: String
    public let connected: Bool
    public let status: PlaybackStatus
    public let positionSecs: Double
    public let volume: UInt8

    public init(
        id: String,
        name: String,
        connected: Bool = true,
        status: PlaybackStatus,
        positionSecs: Double,
        volume: UInt8
    ) {
        self.id = id
        self.name = name
        self.connected = connected
        self.status = status
        self.positionSecs = positionSecs
        self.volume = volume
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case name
        case connected
        case status
        case positionSecs = "position_secs"
        case volume
    }
}

public struct PlaybackState: Codable, Equatable, Sendable {
    public let nodes: [Node]
    public let selectedNodeID: String?
    public let queue: [Track]
    public let currentIndex: Int?
    public let shuffle: Bool
    public let repeatMode: RepeatMode

    public init(
        nodes: [Node],
        selectedNodeID: String? = nil,
        queue: [Track] = [],
        currentIndex: Int? = nil,
        shuffle: Bool = false,
        repeatMode: RepeatMode = .off
    ) {
        self.nodes = nodes
        self.selectedNodeID = selectedNodeID
        self.queue = queue
        self.currentIndex = currentIndex
        self.shuffle = shuffle
        self.repeatMode = repeatMode
    }

    public var currentTrack: Track? {
        guard let currentIndex, queue.indices.contains(currentIndex) else {
            return nil
        }

        return queue[currentIndex]
    }

    private enum CodingKeys: String, CodingKey {
        case nodes
        case selectedNodeID = "selected_node_id"
        case queue
        case currentIndex = "current_index"
        case shuffle
        case repeatMode = "repeat"
    }
}

public struct QueueSnapshot: Codable, Equatable, Sendable {
    public let tracks: [Track]
    public let currentIndex: Int?

    public init(tracks: [Track], currentIndex: Int?) {
        self.tracks = tracks
        self.currentIndex = currentIndex
    }

    private enum CodingKeys: String, CodingKey {
        case tracks
        case currentIndex = "current_index"
    }
}
