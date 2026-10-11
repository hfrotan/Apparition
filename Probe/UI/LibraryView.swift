import DebugTrace
import SwiftUI

private let log = DebugLogger(subsystem: Bundle.main.bundleIdentifier ?? "Apparition", category: "UI")

/// The default tab: what Apparition installed and keeps signed, with when each runs out.
struct LibraryView: View {
    let model: ProbeModel
    let account: AccountModel
    let library: LibraryModel
    let onBrowse: () -> Void
    let onSetup: () -> Void

    /// The app whose JIT launch this screen started, to show its progress on the right card.
    @State private var jitAppId: String?
    @State private var pendingRemoval: LibraryModel.InstalledApp?

    private var apps: [LibraryModel.InstalledApp] {
        library.installed.sorted { (LibraryModel.expiryDate($0) ?? .distantPast) < (LibraryModel.expiryDate($1) ?? .distantPast) }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    if library.installed.isEmpty {
                        emptyState
                    } else {
                        summary
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 420, maximum: 640), spacing: 20)], spacing: 20) {
                            ForEach(apps) { app in
                                card(app)
                            }
                        }
                    }
                }
                .padding(32)
            }
            .navigationTitle("Library")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        Task { await library.refresh(force: true, trigger: "button") }
                    } label: {
                        Label("Refresh All", systemImage: "arrow.triangle.2.circlepath")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(library.busy || model.busy || library.installed.isEmpty)
                    .help("Re-sign and reinstall every app now, so each gets a fresh 7 days")
                }
            }
            .confirmationDialog("Remove \(pendingRemoval?.name ?? "app") from the library?",
                                isPresented: Binding(get: { pendingRemoval != nil }, set: { if !$0 { pendingRemoval = nil } }),
                                titleVisibility: .visible, presenting: pendingRemoval) { app in
                Button("Remove", role: .destructive) { library.removeInstalled(app.id) }
            } message: { _ in
                Text("Apparition stops refreshing it and deletes its kept copy. The app stays on this headset until its profile expires; delete it from the Home View to remove it now.")
            }
        }
    }

    // MARK: Summary and progress

    @ViewBuilder
    private var summary: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Text("^[\(library.installed.count) app](inflect: true)")
                    .font(.title2.weight(.semibold))
                if let next = apps.compactMap(LibraryModel.expiryDate).first {
                    Text("·").foregroundStyle(.tertiary)
                    Text(next < Date() ? "one has expired" : "next expiry \(next.relativeToNow)")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
            }
            if library.busy {
                HStack(spacing: 12) {
                    ProgressView()
                    Text(library.status ?? "Working…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            } else if let run = library.refreshRuns.first, let finished = Date.iso(run.finishedAt) {
                lastRun(run, finished: finished)
            }
            if model.relay == nil {
                Button(action: onSetup) {
                    Note(text: "No companion relay is set, so refreshes and JIT go through LocalDevVPN. Set up Apparition Companion in Setup.", kind: .warning)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func lastRun(_ run: LibraryModel.RefreshRun, finished: Date) -> some View {
        let failed = run.results.values.filter { $0.hasPrefix("error") }.count
        let ok = run.results.values.filter { $0.hasPrefix("ok") }.count
        let text: String
        if run.results["-"] == "nothing due" {
            text = "Last check \(finished.relativeToNow): nothing was due"
        } else if failed > 0 {
            text = "Last refresh \(finished.relativeToNow): \(ok) refreshed, \(failed) failed"
        } else {
            text = "Last refresh \(finished.relativeToNow): \(ok) refreshed"
        }
        return Note(text: text, kind: failed > 0 ? .warning : .info)
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label("No Apps Yet", systemImage: "square.stack.3d.up.slash")
        } description: {
            Text("Install apps from AltStore or SideStore sources, or from a GitHub repository's releases. Apparition signs them with your Apple ID and keeps them refreshed.")
        } actions: {
            Button("Browse Sources", action: onBrowse)
                .buttonStyle(.borderedProminent)
            Button("Finish Setup", action: onSetup)
        }
        .frame(maxWidth: .infinity, minHeight: 420)
    }

    // MARK: App card

    private func card(_ app: LibraryModel.InstalledApp) -> some View {
        let listing = library.sourceApp(for: app)
        return Card {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .top, spacing: 16) {
                    AppIconView(url: listing?.iconURL.flatMap(URL.init(string:)), name: app.name, size: 72)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(app.name)
                            .font(.title3.weight(.semibold))
                            .lineLimit(1)
                        Text("Version \(app.version)")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        TimelineView(.periodic(from: .now, by: 60)) { _ in
                            expiryPill(app)
                        }
                    }
                    Spacer(minLength: 0)
                    moreMenu(app, listing: listing)
                }
                result(app)
                if let jit = jitStatus(app) { jit }
                actions(app, listing: listing)
            }
        }
    }

    private func expiryPill(_ app: LibraryModel.InstalledApp) -> some View {
        guard let expires = LibraryModel.expiryDate(app) else {
            return StatusPill(text: "Not signed yet", systemImage: "clock.badge.questionmark", tint: .secondary)
        }
        let left = expires.timeIntervalSinceNow
        if left <= 0 {
            return StatusPill(text: "Expired \(expires.relativeToNow)", systemImage: "xmark.circle.fill", tint: .red)
        }
        let tint: Color = left < LibraryModel.refreshMargin ? .orange : .green
        return StatusPill(text: "Expires \(expires.relativeToNow)", systemImage: "clock", tint: tint)
    }

    @ViewBuilder
    private func result(_ app: LibraryModel.InstalledApp) -> some View {
        if let error = app.lastError {
            Note(text: error, kind: .error)
                .lineLimit(4)
            if DeveloperSlots.isSlotError(error) {
                ManageSlotsButton(account: account, library: library, onSignIn: onSetup)
            }
        } else if let signed = Date.iso(app.signedAt) {
            Note(text: "Refreshed \(signed.relativeToNow)", kind: .success)
        }
    }

    private func jitStatus(_ app: LibraryModel.InstalledApp) -> Note? {
        guard jitAppId == app.id, let attempt = model.latestAttempt("jit") else { return nil }
        switch attempt.outcome {
        case nil: return Note(text: "Launched under the debugger, serving JIT. Leave Apparition open until the app has started what needs JIT.", kind: .info)
        case "ok": return Note(text: "JIT session finished: \(attempt.detail ?? "ok")", kind: .success)
        case let outcome?: return Note(text: "JIT launch failed: \(outcome)", kind: .error)
        }
    }

    private func actions(_ app: LibraryModel.InstalledApp, listing: Sources.App?) -> some View {
        HStack(spacing: 12) {
            Button {
                Task { await library.refresh(appId: app.id) }
            } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            }
            .disabled(library.busy || model.busy)

            if let bundleId = app.installedBundleId {
                Button {
                    jitAppId = app.id
                    log.info("JIT launch from the library")
                    Task { await model.launchJIT(host: model.defaultTunnelHost, port: 49152, bundleId: bundleId, waitSeconds: 600) }
                } label: {
                    Label("Launch with JIT", systemImage: "bolt.fill")
                }
                .disabled(model.busy || library.busy)
                .help("Opens \(app.name) under a debugger so it can use JIT (UTM needs this to run VMs fast)")
            }

            if let listing, listing.version != app.version {
                Button {
                    Task { await library.install(sourceId: app.source, appId: app.id, stripExtensions: app.stripExtensions) }
                } label: {
                    Label("Update to \(listing.version)", systemImage: "arrow.down.circle")
                }
                .disabled(library.busy || model.busy)
            }
        }
        .buttonStyle(.bordered)
    }

    private func moreMenu(_ app: LibraryModel.InstalledApp, listing: Sources.App?) -> some View {
        Menu {
            if let bundleId = app.installedBundleId {
                Text(bundleId)
            }
            Text("From \(app.source)")
            Divider()
            Button("Remove from Library", systemImage: "trash", role: .destructive) { pendingRemoval = app }
                .disabled(library.busy)
        } label: {
            Image(systemName: "ellipsis")
                .frame(width: 32, height: 32)
        }
        .menuStyle(.button)
        .buttonBorderShape(.circle)
        .help("More")
    }
}
