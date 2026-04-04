import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public actor KanadeClient {
    public typealias StateHandler = @Sendable (PlaybackState) async -> Void

    public nonisolated let webSocketURL: URL
    public nonisolated let mediaBaseURL: URL

    private let session: URLSession
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    private var socket: URLSessionWebSocketTask?
    private var receiveTask: Task<Void, Never>?
    private var nextRequestID = 0
    private var pendingRequests: [Int: CheckedContinuation<KanadeResponse, Error>] = [:]
    private var stateHandler: StateHandler?

    public init(
        webSocketURL: URL,
        mediaBaseURL: URL? = nil,
        session: URLSession = .shared,
        stateHandler: StateHandler? = nil
    ) {
        self.webSocketURL = webSocketURL
        self.mediaBaseURL = mediaBaseURL ?? KanadeClient.defaultMediaBaseURL(for: webSocketURL)
        self.session = session
        self.stateHandler = stateHandler

        self.encoder = JSONEncoder()
        self.decoder = JSONDecoder()
    }

    public func setStateHandler(_ handler: StateHandler?) {
        stateHandler = handler
    }

    public func connect() {
        guard socket == nil else {
            return
        }

        let socket = session.webSocketTask(with: webSocketURL)
        socket.resume()
        self.socket = socket
        receiveTask = Task { [weak self] in
            await self?.receiveLoop()
        }
    }

    public func disconnect() {
        receiveTask?.cancel()
        receiveTask = nil
        socket?.cancel(with: .normalClosure, reason: nil)
        socket = nil
        failPendingRequests(with: CancellationError())
    }

    public func send(_ command: KanadeCommand) async throws {
        try await send(ClientMessage.command(command))
    }

    public func request(_ request: KanadeRequest) async throws -> KanadeResponse {
        let requestID = nextRequestID
        nextRequestID += 1

        return try await withCheckedThrowingContinuation { continuation in
            pendingRequests[requestID] = continuation

            Task {
                do {
                    try await send(.request(id: requestID, request))
                } catch {
                    resolveRequest(id: requestID, result: .failure(error))
                }
            }
        }
    }

    public nonisolated func artworkURL(for albumID: String) -> URL {
        mediaBaseURL.appending(path: "media/art/\(albumID)")
    }

    public nonisolated func trackStreamURL(for trackID: String) -> URL {
        mediaBaseURL.appending(path: "media/tracks/\(trackID)")
    }

    public static func defaultMediaBaseURL(for webSocketURL: URL) -> URL {
        var components = URLComponents(url: webSocketURL, resolvingAgainstBaseURL: false)
        components?.scheme = webSocketURL.scheme == "wss" ? "https" : "http"
        components?.path = ""
        components?.query = nil
        components?.fragment = nil
        if webSocketURL.port == 8080 {
            components?.port = 8081
        }
        return components?.url ?? webSocketURL
    }

    private func send(_ message: ClientMessage) async throws {
        guard let socket else {
            throw KanadeClientError.notConnected
        }

        let data = try encoder.encode(message)
        guard let text = String(data: data, encoding: .utf8) else {
            throw KanadeClientError.invalidPayload
        }

        try await socket.send(.string(text))
    }

    private func receiveLoop() async {
        guard let socket else {
            return
        }

        do {
            while !Task.isCancelled {
                let payload = try await socket.receive()
                let data: Data
                switch payload {
                case let .string(text):
                    guard let messageData = text.data(using: .utf8) else {
                        throw KanadeClientError.invalidPayload
                    }
                    data = messageData
                case let .data(binary):
                    data = binary
                @unknown default:
                    throw KanadeClientError.unsupportedMessage
                }

                let message = try decoder.decode(ServerMessage.self, from: data)
                await handle(message)
            }
        } catch is CancellationError {
        } catch {
            failPendingRequests(with: error)
        }

        self.socket = nil
    }

    private func handle(_ message: ServerMessage) async {
        switch message {
        case let .state(state):
            await stateHandler?(state)
        case let .response(reqID, data):
            resolveRequest(id: reqID, result: .success(data))
        }
    }

    private func resolveRequest(id: Int, result: Result<KanadeResponse, Error>) {
        guard let continuation = pendingRequests.removeValue(forKey: id) else {
            return
        }

        switch result {
        case let .success(response):
            continuation.resume(returning: response)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }

    private func failPendingRequests(with error: Error) {
        let pending = pendingRequests
        pendingRequests.removeAll()
        for (_, continuation) in pending {
            continuation.resume(throwing: error)
        }
    }
}

public enum KanadeClientError: Error, LocalizedError {
    case notConnected
    case invalidPayload
    case unsupportedMessage

    public var errorDescription: String? {
        switch self {
        case .notConnected:
            return "The Kanade client is not connected."
        case .invalidPayload:
            return "The Kanade client received or generated an invalid payload."
        case .unsupportedMessage:
            return "The Kanade client received an unsupported WebSocket message."
        }
    }
}
