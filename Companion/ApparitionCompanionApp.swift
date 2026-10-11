import DebugTrace
import SwiftUI

/// Apparition Companion: the optional helper on a Mac (later a PC) for Apparition on the headset.
/// It is the relay that replaces LocalDevVPN (see Relay.swift), and it can run a self-hosted
/// anisette server with the user's Docker (see Anisette.swift). On the same LAN, or with both
/// on Tailscale, Apparition can open its tunnel through it from anywhere.
@main
struct ApparitionCompanionApp: App {
    @State private var relay: Relay
    @State private var anisette: Anisette

    init() {
        DebugTrace.configure(.init(subsystems: ["com.illixion.apparition.companion"]))
        let relay = Relay()
        relay.start()
        _relay = State(initialValue: relay)
        let anisette = Anisette()
        anisette.onChange = { relay.anisettePort = $0 }
        anisette.startMonitoring()
        _anisette = State(initialValue: anisette)
    }

    var body: some Scene {
        MenuBarExtra("Apparition", systemImage: "arrow.triangle.swap") {
            Text("Relay: \(relay.status)")
            Text("\(relay.sessions.count) connections")
            Text("Anisette: \(anisette.state.text)")
            switch anisette.state {
            case .stopped, .failed: Button("Start anisette") { Task { await anisette.start() } }
            case .running: Button("Stop anisette") { Task { await anisette.stop() } }
            default: EmptyView()
            }
            Divider()
            OpenWindowButton()
            Button("Quit") { NSApplication.shared.terminate(nil) }
        }
        Window("Apparition Companion", id: "main") {
            CompanionView(relay: relay, anisette: anisette)
        }
    }
}

struct CompanionView: View {
    let relay: Relay
    let anisette: Anisette

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            AnisetteRow(anisette: anisette)
            Divider()
            Text("Relay: \(relay.status)").font(.headline)
            Text("In Apparition, set the relay to this Mac's address, port \(String(Relay.port)).")
                .font(.caption)
            Table(relay.sessions.reversed()) {
                TableColumn("Started") { Text($0.startedAt.formatted(date: .omitted, time: .standard)) }
                TableColumn("Device") { Text($0.peer) }
                TableColumn("Port") { Text(String($0.port)) }
                TableColumn("To device") { Text($0.bytesToDevice.formatted(.byteCount(style: .file))) }
                TableColumn("From device") { Text($0.bytesFromDevice.formatted(.byteCount(style: .file))) }
                TableColumn("Outcome") { Text($0.outcome ?? "open") }
            }
        }
        .padding()
        .frame(minWidth: 640, minHeight: 320)
    }
}

private struct OpenWindowButton: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("Show connections") { openWindow(id: "main") } }
}

private struct AnisetteRow: View {
    let anisette: Anisette

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Anisette: \(anisette.state.text)").font(.headline)
                Spacer()
                switch anisette.state {
                case .stopped, .failed:
                    Button("Start") { Task { await anisette.start() } }
                case .running:
                    Button("Stop") { Task { await anisette.stop() } }
                default:
                    EmptyView()
                }
            }
            Text(verbatim: """
                Apparition needs an anisette-v3 server to sign in to Apple and ships none. Start runs \
                Dadoum's anisette-v3-server in your Docker (pinned image, from its own registry), \
                and Apparition offers to use it (this Mac's address, port \(Anisette.port)). It sees \
                your headset's anisette identity and IP, never your Apple ID or password.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            if case .failed(let message) = anisette.state {
                Text(message).font(.caption.monospaced()).foregroundStyle(.red).textSelection(.enabled)
            }
        }
    }
}

extension Anisette.State {
    var text: String {
        switch self {
        case .checking: "checking"
        case .running: "running on port \(Anisette.port)"
        case .stopped: "not running"
        case .noDocker: "needs Docker (Docker Desktop, OrbStack or Colima)"
        case .working(let label): label
        case .failed: "failed"
        }
    }
}
