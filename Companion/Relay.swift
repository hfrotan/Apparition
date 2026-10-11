import DebugTrace
import Foundation
import Network
import Observation

private let log = DebugLogger(subsystem: "com.illixion.apparition.companion", category: "Relay")

/// Lets Apparition reach the headset it runs on. The headset resets connections it makes to its
/// own RemotePairing service, so Apparition asks the companion to connect back to it instead:
///
///     Apparition -> companion:49160   "APPARITION-RELAY 1 <port>\n"
///     companion -> <Apparition's own address>:<port>, then "OK\n" and bytes both ways
///
/// The companion only ever connects back to the address a request came from, so it can't be
/// used to reach anything else, and everything it carries is end-to-end encrypted between
/// Apparition and the headset (pair-verify, then TLS-PSK). It holds no keys and no Apple ID.
@MainActor @Observable
final class Relay {
    struct Session: Identifiable {
        let id = UUID()
        let peer: String
        let port: UInt16
        let startedAt = Date()
        var bytesToDevice = 0
        var bytesFromDevice = 0
        var outcome: String?
    }

    nonisolated static let port: UInt16 = 49160
    nonisolated static let serviceType = "_apparition-companion._tcp"

    private(set) var status = "stopped"
    private(set) var sessions: [Session] = []

    private var listener: NWListener?
    private let queue = DispatchQueue(label: "relay")

    func start() {
        do {
            let listener = try NWListener(using: .tcp, on: NWEndpoint.Port(rawValue: Self.port)!)
            listener.service = NWListener.Service(type: Self.serviceType)
            listener.stateUpdateHandler = { [weak self] state in
                Task { @MainActor in self?.status = Self.describe(state) }
            }
            listener.newConnectionHandler = { [weak self] connection in
                Task { @MainActor in self?.accept(connection) }
            }
            listener.start(queue: queue)
            self.listener = listener
        } catch {
            status = "can't listen on \(Self.port): \(error)"
            log.error("listen failed: \(String(describing: error), privacy: .public)")
        }
    }

    private static func describe(_ state: NWListener.State) -> String {
        switch state {
        case .ready: "listening on port \(port)"
        case .failed(let error): "failed: \(error)"
        case .waiting(let error): "waiting: \(error)"
        case .cancelled: "stopped"
        default: "starting"
        }
    }

    private func accept(_ client: NWConnection) {
        guard case var .hostPort(host, _) = client.endpoint else {
            client.cancel()
            return
        }
        // The listener is dual-stack, so IPv4 peers arrive as ::ffff:a.b.c.d.
        if case let .ipv6(address) = host, let v4 = address.asIPv4 {
            host = .ipv4(v4)
        }
        client.start(queue: queue)
        readRequest(client, buffer: Data()) { [weak self] request in
            Task { @MainActor in self?.connect(client, back: host, request: request) }
        }
    }

    /// Reads up to the first newline (at most 256 bytes) and hands over the line.
    private nonisolated func readRequest(_ client: NWConnection, buffer: Data, done: @escaping @Sendable (String?) -> Void) {
        client.receive(minimumIncompleteLength: 1, maximumLength: 256) { [weak self] data, _, isComplete, error in
            var buffer = buffer
            if let data { buffer.append(data) }
            if let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                done(String(data: buffer[..<newline], encoding: .utf8))
            } else if error != nil || isComplete || buffer.count > 256 {
                done(nil)
            } else {
                self?.readRequest(client, buffer: buffer, done: done)
            }
        }
    }

    private func connect(_ client: NWConnection, back host: NWEndpoint.Host, request: String?) {
        let parts = request?.split(separator: " ") ?? []
        guard parts.count == 3, parts[0] == "APPARITION-RELAY", parts[1] == "1",
              let port = UInt16(parts[2]), port >= 1024 else {
            refuse(client, "bad request")
            return
        }
        let index = sessions.count
        sessions.append(Session(peer: "\(host)", port: port))
        log.info("relay to :\(port, privacy: .public)")

        let device = NWConnection(host: host, port: NWEndpoint.Port(rawValue: port)!, using: .tcp)
        device.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                client.send(content: Data("OK\n".utf8), completion: .contentProcessed { _ in
                    self?.pipe(from: client, to: device, session: index, toDevice: true)
                    self?.pipe(from: device, to: client, session: index, toDevice: false)
                })
            case .failed(let error), .waiting(let error):
                self?.refuse(client, "can't connect to :\(port): \(error)")
                device.cancel()
                Task { @MainActor in self?.finish(index, "failed: \(error)") }
            default:
                break
            }
        }
        device.start(queue: queue)
    }

    private nonisolated func refuse(_ client: NWConnection, _ reason: String) {
        client.send(content: Data("ERR \(reason)\n".utf8), completion: .contentProcessed { _ in client.cancel() })
    }

    private nonisolated func pipe(from source: NWConnection, to sink: NWConnection, session: Int, toDevice: Bool) {
        source.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            if let data, !data.isEmpty {
                let count = data.count
                Task { @MainActor in self?.count(session, count, toDevice: toDevice) }
                sink.send(content: data, completion: .contentProcessed { sendError in
                    if sendError == nil { self?.pipe(from: source, to: sink, session: session, toDevice: toDevice) }
                })
            }
            if isComplete || error != nil {
                // One side closed: close both, the tunnel is gone either way.
                source.cancel()
                sink.cancel()
                Task { @MainActor in self?.finish(session, error.map { "closed: \($0)" } ?? "closed") }
            } else if data == nil || data!.isEmpty {
                self?.pipe(from: source, to: sink, session: session, toDevice: toDevice)
            }
        }
    }

    private func count(_ session: Int, _ bytes: Int, toDevice: Bool) {
        guard sessions.indices.contains(session) else { return }
        if toDevice { sessions[session].bytesToDevice += bytes } else { sessions[session].bytesFromDevice += bytes }
    }

    private func finish(_ session: Int, _ outcome: String) {
        guard sessions.indices.contains(session), sessions[session].outcome == nil else { return }
        sessions[session].outcome = outcome
    }
}
