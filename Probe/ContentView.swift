import SwiftUI

struct ContentView: View {
    let model: ProbeModel
    @State private var pin = ""

    var body: some View {
        NavigationStack {
            List {
                Section("Steps") {
                    Text("1. Open Settings > General > Remote Devices in a window beside this one.")
                    Text("2. Pair against the manual-pairing service on the device marked \"this device\". Type the code Settings shows.")
                    Text("3. Then open a tunnel against the paired service (usually port 49152).")
                }
                .font(.footnote)

                if model.pinRequested {
                    Section("Code from Settings") {
                        TextField("6-digit code", text: $pin)
                            .keyboardType(.numberPad)
                        HStack {
                            Button("Submit") { model.submitPin(pin); pin = "" }
                            Button("Cancel", role: .cancel) { model.cancelPin() }
                        }
                    }
                }

                Section("Advertised services") {
                    if model.advertised.isEmpty { Text("Nothing yet").foregroundStyle(.secondary) }
                    ForEach(model.advertised.values.sorted { $0.id < $1.id }) { service in
                        serviceRow(service)
                    }
                }

                Section("Reachability") {
                    Button("Scan all candidates") { Task { await model.scan() } }
                        .disabled(model.busy)
                    ForEach(Array(model.reachability.enumerated()), id: \.offset) { _, row in
                        Text("\(row.host):\(row.port)  \(row.result)  \(row.elapsedMs) ms")
                            .font(.caption.monospaced())
                    }
                }

                Section("Attempts") {
                    ForEach(Array(model.attempts.enumerated()), id: \.offset) { _, attempt in
                        VStack(alignment: .leading, spacing: 4) {
                            Text("\(attempt.action) \(attempt.host):\(attempt.port)").font(.headline)
                            Text(attempt.outcome ?? "running…").font(.caption.monospaced())
                            if let peer = attempt.peer { Text("peer: \(peer.name) \(peer.model)").font(.caption) }
                            if let services = attempt.services {
                                Text("\(services.count) RSD services").font(.caption)
                                ForEach(services, id: \.name) { service in
                                    Text("\(service.name) :\(service.port)").font(.caption2.monospaced())
                                }
                            }
                        }
                    }
                }

                Section("This device") {
                    Text(model.hasPairingFile ? "Pairing file saved" : "No pairing file")
                    if model.hasPairingFile {
                        Button("Forget pairing", role: .destructive) { model.forgetPairing() }
                    }
                    ForEach(model.interfaces, id: \.self) { Text($0).font(.caption.monospaced()) }
                }
            }
            .navigationTitle("Apparition")
            .toolbar {
                Button("Browse") { model.startBrowsing() }
            }
        }
        .onAppear { model.startBrowsing() }
    }

    @ViewBuilder
    private func serviceRow(_ service: ProbeModel.Advertised) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(service.name)\(service.isSelf ? "  (this device)" : "")").font(.headline)
            Text("\(service.type)  port \(service.port.map(String.init) ?? "?")").font(.caption.monospaced())
            if let port = service.port, port > 0 {
                ForEach(model.candidateHosts(for: service), id: \.self) { host in
                    HStack {
                        Text(host).font(.caption.monospaced())
                        Spacer()
                        if service.type == ProbeModel.manualPairingType {
                            Button("Pair") { Task { await model.pair(host: host, port: port) } }
                        } else {
                            Button("Tunnel") { Task { await model.tunnel(host: host, port: port) } }
                        }
                    }
                    .buttonStyle(.bordered)
                    .disabled(model.busy)
                }
            }
        }
    }
}
