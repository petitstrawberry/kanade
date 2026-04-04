import Foundation

public enum KanadeCommand: Sendable {
    case play
    case pause
    case stop
    case next
    case previous
    case seek(positionSecs: Double)
    case setVolume(UInt8)
    case setRepeat(RepeatMode)
    case setShuffle(Bool)
    case selectNode(nodeID: String)
    case addToQueue(Track)
    case addTracksToQueue([Track])
    case playIndex(Int)
    case removeFromQueue(Int)
    case moveInQueue(from: Int, to: Int)
    case clearQueue
    case replaceAndPlay(tracks: [Track], index: Int)
}

extension KanadeCommand: Encodable {
    private enum CodingKeys: String, CodingKey {
        case cmd
        case positionSecs = "position_secs"
        case volume
        case repeatMode = "repeat"
        case shuffle
        case nodeID = "node_id"
        case track
        case tracks
        case index
        case from
        case to
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)

        switch self {
        case .play:
            try container.encode("play", forKey: .cmd)
        case .pause:
            try container.encode("pause", forKey: .cmd)
        case .stop:
            try container.encode("stop", forKey: .cmd)
        case .next:
            try container.encode("next", forKey: .cmd)
        case .previous:
            try container.encode("previous", forKey: .cmd)
        case let .seek(positionSecs):
            try container.encode("seek", forKey: .cmd)
            try container.encode(positionSecs, forKey: .positionSecs)
        case let .setVolume(volume):
            try container.encode("set_volume", forKey: .cmd)
            try container.encode(volume, forKey: .volume)
        case let .setRepeat(repeatMode):
            try container.encode("set_repeat", forKey: .cmd)
            try container.encode(repeatMode, forKey: .repeatMode)
        case let .setShuffle(shuffle):
            try container.encode("set_shuffle", forKey: .cmd)
            try container.encode(shuffle, forKey: .shuffle)
        case let .selectNode(nodeID):
            try container.encode("select_node", forKey: .cmd)
            try container.encode(nodeID, forKey: .nodeID)
        case let .addToQueue(track):
            try container.encode("add_to_queue", forKey: .cmd)
            try container.encode(track, forKey: .track)
        case let .addTracksToQueue(tracks):
            try container.encode("add_tracks_to_queue", forKey: .cmd)
            try container.encode(tracks, forKey: .tracks)
        case let .playIndex(index):
            try container.encode("play_index", forKey: .cmd)
            try container.encode(index, forKey: .index)
        case let .removeFromQueue(index):
            try container.encode("remove_from_queue", forKey: .cmd)
            try container.encode(index, forKey: .index)
        case let .moveInQueue(from, to):
            try container.encode("move_in_queue", forKey: .cmd)
            try container.encode(from, forKey: .from)
            try container.encode(to, forKey: .to)
        case .clearQueue:
            try container.encode("clear_queue", forKey: .cmd)
        case let .replaceAndPlay(tracks, index):
            try container.encode("replace_and_play", forKey: .cmd)
            try container.encode(tracks, forKey: .tracks)
            try container.encode(index, forKey: .index)
        }
    }
}

public enum KanadeRequest: Sendable {
    case getAlbums
    case getAlbumTracks(albumID: String)
    case getArtists
    case getArtistAlbums(artist: String)
    case getArtistTracks(artist: String)
    case getGenres
    case getGenreAlbums(genre: String)
    case getGenreTracks(genre: String)
    case search(query: String)
    case getQueue
}

public enum ClientMessage: Sendable {
    case command(KanadeCommand)
    case request(id: Int, KanadeRequest)
}

extension ClientMessage: Encodable {
    private enum CodingKeys: String, CodingKey {
        case reqID = "req_id"
        case req
        case albumID = "album_id"
        case artist
        case genre
        case query
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case let .command(command):
            try command.encode(to: encoder)
        case let .request(id, request):
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(id, forKey: .reqID)

            switch request {
            case .getAlbums:
                try container.encode("get_albums", forKey: .req)
            case let .getAlbumTracks(albumID):
                try container.encode("get_album_tracks", forKey: .req)
                try container.encode(albumID, forKey: .albumID)
            case .getArtists:
                try container.encode("get_artists", forKey: .req)
            case let .getArtistAlbums(artist):
                try container.encode("get_artist_albums", forKey: .req)
                try container.encode(artist, forKey: .artist)
            case let .getArtistTracks(artist):
                try container.encode("get_artist_tracks", forKey: .req)
                try container.encode(artist, forKey: .artist)
            case .getGenres:
                try container.encode("get_genres", forKey: .req)
            case let .getGenreAlbums(genre):
                try container.encode("get_genre_albums", forKey: .req)
                try container.encode(genre, forKey: .genre)
            case let .getGenreTracks(genre):
                try container.encode("get_genre_tracks", forKey: .req)
                try container.encode(genre, forKey: .genre)
            case let .search(query):
                try container.encode("search", forKey: .req)
                try container.encode(query, forKey: .query)
            case .getQueue:
                try container.encode("get_queue", forKey: .req)
            }
        }
    }
}

public enum KanadeResponse: Sendable, Equatable {
    case albums([Album])
    case albumTracks([Track])
    case artists([String])
    case artistAlbums([Album])
    case artistTracks([Track])
    case genres([String])
    case genreAlbums([Album])
    case genreTracks([Track])
    case searchResults([Track])
    case queue(QueueSnapshot)
}

extension KanadeResponse: Decodable {
    private enum CodingKeys: String, CodingKey {
        case albums
        case albumTracks = "album_tracks"
        case artists
        case artistAlbums = "artist_albums"
        case artistTracks = "artist_tracks"
        case genres
        case genreAlbums = "genre_albums"
        case genreTracks = "genre_tracks"
        case searchResults = "search_results"
        case queue
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)

        if container.contains(.albums) {
            self = .albums(try container.decode(AlbumPayload.self, forKey: .albums).albums)
        } else if container.contains(.albumTracks) {
            self = .albumTracks(try container.decode(TrackPayload.self, forKey: .albumTracks).tracks)
        } else if container.contains(.artists) {
            self = .artists(try container.decode(ArtistPayload.self, forKey: .artists).artists)
        } else if container.contains(.artistAlbums) {
            self = .artistAlbums(try container.decode(AlbumPayload.self, forKey: .artistAlbums).albums)
        } else if container.contains(.artistTracks) {
            self = .artistTracks(try container.decode(TrackPayload.self, forKey: .artistTracks).tracks)
        } else if container.contains(.genres) {
            self = .genres(try container.decode(GenrePayload.self, forKey: .genres).genres)
        } else if container.contains(.genreAlbums) {
            self = .genreAlbums(try container.decode(AlbumPayload.self, forKey: .genreAlbums).albums)
        } else if container.contains(.genreTracks) {
            self = .genreTracks(try container.decode(TrackPayload.self, forKey: .genreTracks).tracks)
        } else if container.contains(.searchResults) {
            self = .searchResults(try container.decode(TrackPayload.self, forKey: .searchResults).tracks)
        } else if container.contains(.queue) {
            self = .queue(try container.decode(QueueSnapshot.self, forKey: .queue))
        } else {
            throw DecodingError.dataCorruptedError(
                forKey: .albums,
                in: container,
                debugDescription: "Unknown Kanade response variant"
            )
        }
    }

    private struct AlbumPayload: Decodable {
        let albums: [Album]
    }

    private struct TrackPayload: Decodable {
        let tracks: [Track]
    }

    private struct ArtistPayload: Decodable {
        let artists: [String]
    }

    private struct GenrePayload: Decodable {
        let genres: [String]
    }
}

public enum ServerMessage: Sendable, Equatable {
    case state(PlaybackState)
    case response(reqID: Int, data: KanadeResponse)
}

extension ServerMessage: Decodable {
    private enum CodingKeys: String, CodingKey {
        case type
        case state
        case reqID = "req_id"
        case data
    }

    private enum MessageType: String, Decodable {
        case state
        case response
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(MessageType.self, forKey: .type) {
        case .state:
            self = .state(try container.decode(PlaybackState.self, forKey: .state))
        case .response:
            self = .response(
                reqID: try container.decode(Int.self, forKey: .reqID),
                data: try container.decode(KanadeResponse.self, forKey: .data)
            )
        }
    }
}
