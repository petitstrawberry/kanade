import Foundation
import Testing
@testable import KanadeSwift

@Test func encodesCommandWithSnakeCaseFields() throws {
    let track = Track(id: "track-1", filePath: "/music/track.flac", title: "Track")
    let message = ClientMessage.command(.replaceAndPlay(tracks: [track], index: 2))

    let data = try JSONEncoder().encode(message)
    let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])

    #expect(object["cmd"] as? String == "replace_and_play")
    #expect(object["index"] as? Int == 2)
    let tracks = try #require(object["tracks"] as? [[String: Any]])
    #expect(tracks.first?["file_path"] as? String == "/music/track.flac")
}

@Test func decodesStateBroadcast() throws {
    let data = Data(
        """
        {
          "type": "state",
          "state": {
            "nodes": [
              {
                "id": "default",
                "name": "Living Room",
                "connected": true,
                "status": "playing",
                "position_secs": 24.5,
                "volume": 55
              }
            ],
            "selected_node_id": "default",
            "queue": [
              {
                "id": "track-1",
                "file_path": "/music/track.flac",
                "title": "Track"
              }
            ],
            "current_index": 0,
            "shuffle": false,
            "repeat": "all"
          }
        }
        """.utf8
    )

    let message = try JSONDecoder().decode(ServerMessage.self, from: data)

    guard case let .state(state) = message else {
        Issue.record("Expected a state broadcast")
        return
    }

    #expect(state.selectedNodeID == "default")
    #expect(state.currentTrack?.title == "Track")
    #expect(state.repeatMode == .all)
}

@Test func decodesExternallyTaggedResponseVariant() throws {
    let data = Data(
        """
        {
          "type": "response",
          "req_id": 7,
          "data": {
            "queue": {
              "tracks": [
                {
                  "id": "track-1",
                  "file_path": "/music/track.flac"
                }
              ],
              "current_index": 0
            }
          }
        }
        """.utf8
    )

    let message = try JSONDecoder().decode(ServerMessage.self, from: data)

    guard case let .response(reqID, response) = message else {
        Issue.record("Expected a response message")
        return
    }

    #expect(reqID == 7)
    guard case let .queue(snapshot) = response else {
        Issue.record("Expected a queue response")
        return
    }

    #expect(snapshot.currentIndex == 0)
    #expect(snapshot.tracks.first?.id == "track-1")
}
