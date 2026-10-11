import BackgroundTasks
import DebugTrace
import Foundation
import Observation

private let log = DebugLogger(subsystem: Bundle.main.bundleIdentifier ?? "Apparition", category: "Library")

/// Spike 5: sources, the apps installed from them, and keeping those apps signed.
///
/// Installing downloads the IPA into Documents/apps (kept, so a refresh never needs the
/// network for it), signs it for this device and installs it through the LocalDevVPN tunnel.
/// A refresh re-signs the kept IPA and reinstalls it over the old copy, which keeps its data.
@MainActor @Observable
final class LibraryModel {
    struct InstalledApp: Codable, Sendable, Identifiable {
        /// The source app's id: its bundle identifier, or `github:owner/repo`.
        let id: String
        let source: String
        var name: String
        var version: String
        var ipaFile: String
        var stripExtensions: Bool
        /// After signing, e.g. com.utmapp.UTM.<team>: what launchJIT needs.
        var installedBundleId: String?
        var signedAt: String?
        var expiresAt: String?
        var lastError: String?
    }

    struct RefreshRun: Codable, Sendable {
        let startedAt: String
        let trigger: String
        var finishedAt: String?
        var results: [String: String] = [:]
    }

    nonisolated static let refreshTaskId = "com.illixion.apparition.refresh"
    /// Free-account profiles last 7 days; refresh once fewer than this many are left.
    nonisolated static let refreshMargin: TimeInterval = 3 * 24 * 3600

    private(set) var sources: [Sources.Source] = []
    private(set) var installed: [InstalledApp] = []
    private(set) var busy = false
    private(set) var status: String?
    private(set) var refreshRuns: [RefreshRun] = []

    private let account: AccountModel
    private let probe: ProbeModel

    private static var stateURL: URL { AccountModel.dataDir.appendingPathComponent("library.json") }

    private struct Stored: Codable {
        var sources: [Sources.Source]
        var installed: [InstalledApp]
        var refreshRuns: [RefreshRun]?
    }

    init(account: AccountModel, probe: ProbeModel) {
        self.account = account
        self.probe = probe
        if let data = try? Data(contentsOf: Self.stateURL),
           let stored = try? JSONDecoder().decode(Stored.self, from: data) {
            sources = stored.sources
            installed = stored.installed
            refreshRuns = stored.refreshRuns ?? []
        }
    }

    private func save() {
        let stored = Stored(sources: sources, installed: installed, refreshRuns: Array(refreshRuns.prefix(20)))
        guard let data = try? JSONEncoder().encode(stored) else { return }
        try? data.write(to: Self.stateURL, options: .atomic)
    }

    // MARK: Sources

    func addSource(_ id: String, assetPattern: String?, prereleases: Bool) async {
        let id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        if !sources.contains(where: { $0.id == id }) {
            sources.append(Sources.Source(id: id, assetPattern: assetPattern, prereleases: prereleases))
        }
        await refreshSource(id)
    }

    func removeSource(_ id: String) {
        sources.removeAll { $0.id == id }
        save()
    }

    func refreshSources() async {
        for source in sources { await refreshSource(source.id) }
    }

    private func refreshSource(_ id: String) async {
        guard let index = sources.firstIndex(where: { $0.id == id }) else { return }
        do {
            let (name, apps) = try await Sources.fetch(sources[index])
            sources[index].name = name
            sources[index].apps = apps
            sources[index].error = nil
        } catch {
            sources[index].error = "\(error)"
        }
        sources[index].fetchedAt = DebugTime.iso(Date())
        save()
    }

    // MARK: Install and refresh

    /// Downloads (if this version isn't kept yet), signs and installs `appId` from `sourceId`.
    func install(sourceId: String, appId: String, stripExtensions: Bool) async {
        guard let app = sources.first(where: { $0.id == sourceId })?.apps.first(where: { $0.id == appId }) else {
            status = "no app \(appId) in \(sourceId)"
            return
        }
        await exclusive("install \(app.name)") {
            let file = try await self.download(app)
            var record = self.installed.first { $0.id == app.id }
                ?? InstalledApp(id: app.id, source: sourceId, name: app.name, version: app.version,
                                ipaFile: file, stripExtensions: stripExtensions)
            record.version = app.version
            record.ipaFile = file
            record.stripExtensions = stripExtensions
            let outcome = await self.signAndInstall(&record)
            self.upsert(record)
            return outcome
        }
    }

    /// Re-signs and reinstalls every kept app that expires within `refreshMargin` (all of them
    /// with `force`). Called by the background task and by hand.
    @discardableResult
    func refresh(force: Bool, trigger: String) async -> RefreshRun {
        var run = RefreshRun(startedAt: DebugTime.iso(Date()), trigger: trigger)
        let due = installed.filter { force || Self.isDue($0) }
        if due.isEmpty {
            run.results["-"] = "nothing due"
        } else if busy {
            run.results["-"] = "busy"
        } else {
            busy = true
            for var record in due {
                status = "refreshing \(record.name)"
                run.results[record.id] = await signAndInstall(&record)
                upsert(record)
            }
            busy = false
            status = nil
        }
        run.finishedAt = DebugTime.iso(Date())
        refreshRuns.insert(run, at: 0)
        save()
        scheduleRefresh()
        log.info("refresh (\(trigger, privacy: .public)): \(run.results.count, privacy: .public) results")
        return run
    }

    nonisolated static func isDue(_ app: InstalledApp, now: Date = Date()) -> Bool {
        guard let text = app.expiresAt,
              let expires = try? Date(text, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true))
        else { return true }
        return expires.timeIntervalSince(now) < refreshMargin
    }

    private func signAndInstall(_ record: inout InstalledApp) async -> String {
        if UserDefaults.standard.string(forKey: "pairedUDID") == nil {
            await probe.identify(host: probe.defaultTunnelHost, port: 49152)
        }
        guard let udid = UserDefaults.standard.string(forKey: "pairedUDID") else {
            return fail(&record, "can't read this device's UDID (paired, and LocalDevVPN or the companion up?)")
        }
        let name = UserDefaults.standard.string(forKey: "pairedName") ?? "Apple Vision Pro"
        guard await account.signInSilently() else {
            return fail(&record, "not signed in (sign in, or turn on Remember password for unattended refreshes)")
        }
        let path = AccountModel.appsDir.appendingPathComponent(record.ipaFile).path
        let signed = await AccountModel.signFile(path, udid: udid, deviceName: name, stripExtensions: record.stripExtensions)
        guard signed["ok"] as? Bool == true, let signedPath = signed["signedPath"] as? String else {
            return fail(&record, "sign failed: \(signed["error"] as? String ?? "unknown")")
        }
        let main = AccountModel.inspect(URL(fileURLWithPath: signedPath), udid: udid).first
        let outcome = await probe.install(host: probe.defaultTunnelHost, port: 49152, appPath: signedPath)
        try? FileManager.default.removeItem(atPath: signedPath)
        guard outcome == "ok" else {
            return fail(&record, "install failed (LocalDevVPN connected, or the companion running?): \(outcome)")
        }
        record.installedBundleId = main?.bundleId
        record.signedAt = DebugTime.iso(Date())
        record.expiresAt = main?.profileExpires
        record.lastError = nil
        return "ok, expires \(record.expiresAt ?? "?")"
    }

    private func fail(_ record: inout InstalledApp, _ message: String) -> String {
        record.lastError = message
        log.error("\(record.name, privacy: .public): \(message, privacy: .public)")
        return "error: \(message)"
    }

    private func download(_ app: Sources.App) async throws -> String {
        let safe = app.id.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
        let file = "\(safe)-\(app.version).ipa"
        let destination = AccountModel.appsDir.appendingPathComponent(file)
        if FileManager.default.fileExists(atPath: destination.path) { return file }
        guard let url = URL(string: app.downloadURL) else { throw IDevice.ProbeError("bad download URL") }
        status = "downloading \(app.name) \(app.version)"
        let (temporary, response) = try await URLSession.shared.download(from: url)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw IDevice.ProbeError("HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0) downloading \(app.name)")
        }
        try FileManager.default.moveItem(at: temporary, to: destination)
        // Keep one version per app.
        for other in (try? FileManager.default.contentsOfDirectory(atPath: AccountModel.appsDir.path)) ?? []
        where other.hasPrefix("\(safe)-") && other != file {
            try? FileManager.default.removeItem(at: AccountModel.appsDir.appendingPathComponent(other))
        }
        account.refreshDownloads()
        return file
    }

    private func upsert(_ record: InstalledApp) {
        if let index = installed.firstIndex(where: { $0.id == record.id }) {
            installed[index] = record
        } else {
            installed.append(record)
        }
        save()
    }

    private func exclusive(_ label: String, _ body: () async throws -> String) async {
        guard !busy else { status = "busy"; return }
        busy = true
        status = label
        defer { busy = false }
        do {
            status = "\(label): \(try await body())"
        } catch {
            status = "\(label): error: \(error)"
        }
        log.info("\(self.status ?? "", privacy: .public)")
    }

    // MARK: Background refresh

    /// Asks for a background run about every 12 hours. It only succeeds when LocalDevVPN is
    /// connected and the password is remembered; otherwise the run records why.
    func scheduleRefresh() {
        let request = BGAppRefreshTaskRequest(identifier: Self.refreshTaskId)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 12 * 3600)
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            log.error("refresh not scheduled: \(String(describing: error), privacy: .public)")
        }
    }
}
