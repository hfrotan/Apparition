import DebugTrace
import SwiftUI

/// Apparition Companion: the optional helper on a Mac (later a PC) for Apparition on the headset.
/// For now it is the relay that replaces LocalDevVPN (see Relay.swift). On the same LAN,
/// or with both on Tailscale, Apparition can open its tunnel through it from anywhere.
@main
struct ApparitionCompanionApp: App {
    @State private var relay: Relay

    init() {
        DebugTrace.configure(.init(subsystems: ["com.illixion.apparition.companion"]))
        let relay = Relay()
        relay.start()
        _relay = State(initialValue: relay)
    }

    var body: some Scene {
        MenuBarExtra("Apparition", systemImage: "arrow.triangle.swap") {
            Text("Relay: \(relay.status)")
            Text("\(relay.sessions.count) connections")
            Divider()
            OpenWindowButton()
            Button("Quit") { NSApplication.shared.terminate(nil) }
        }
        Window("Apparition Companion", id: "main") {
            CompanionView(relay: relay)
        }
    }
}

struct CompanionView: View {
    let relay: Relay

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
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
