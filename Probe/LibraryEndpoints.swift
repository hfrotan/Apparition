import DebugTrace
import Foundation

/// Spike 5 endpoints: sources, installing from them and refreshing what was installed.
/// Install and refresh return at once; poll libraryState.
@MainActor
enum LibraryEndpoints {
    struct State: Encodable {
        let busy: Bool
        let status: String?
        let sources: [Sources.Source]
        let installed: [LibraryModel.InstalledApp]
        let refreshRuns: [LibraryModel.RefreshRun]
        let rememberPassword: Bool
        let hasStoredPassword: Bool
    }

    static func state(_ model: LibraryModel, _ account: AccountModel) -> State {
        State(busy: model.busy, status: model.status, sources: model.sources, installed: model.installed,
              refreshRuns: Array(model.refreshRuns.prefix(5)), rememberPassword: account.rememberPassword,
              hasStoredPassword: Keychain.hasPassword)
    }

    static func register(_ model: LibraryModel, account: AccountModel) {
        DebugSurface.shared.register([
            .query("libraryState", "Sources and their apps, installed apps with expiry, recent refresh runs") { _ in
                state(model, account)
            },
            .command("addSource", "Add and fetch a source: an AltStore/SideStore source URL, or github:owner/repo",
                     parameters: [
                        .string("url", "https://… source JSON, or github:owner/repo", required: true),
                        .string("assetPattern", "github: only, text the .ipa asset name must contain, e.g. visionOS"),
                        .boolean("prereleases", "github: only, also consider prereleases", default: false),
                     ], timeout: .seconds(60)) { args in
                await model.addSource(try args.requireString("url"), assetPattern: args.string("assetPattern"),
                                      prereleases: args.bool("prereleases") ?? false)
                return state(model, account)
            },
            .command("removeSource", "Forget a source (installed apps stay)",
                     parameters: [.string("url", "The source's id as listed", required: true)]) { args in
                model.removeSource(try args.requireString("url"))
                return state(model, account)
            },
            .command("refreshSources", "Fetch every source again", timeout: .seconds(120)) { _ in
                await model.refreshSources()
                return state(model, account)
            },
            .command("installFromSource", "Download, sign and install an app from a source through the LocalDevVPN tunnel. Returns at once; poll libraryState",
                     parameters: [
                        .string("sourceId", "The source's id", required: true),
                        .string("appId", "The app's id in that source (bundle id or github:owner/repo)", required: true),
                        .boolean("stripExtensions", "Drop app extensions, which a free account can't sign", default: true),
                     ]) { args in
                let source = try args.requireString("sourceId")
                let app = try args.requireString("appId")
                let strip = args.bool("stripExtensions") ?? true
                Task { await model.install(sourceId: source, appId: app, stripExtensions: strip) }
                try await Task.sleep(for: .seconds(1))
                return state(model, account)
            },
            .command("refreshApps", "Re-sign and reinstall installed apps that expire within 3 days (all with force). Returns at once; poll libraryState",
                     parameters: [.boolean("force", "Refresh every installed app", default: false)]) { args in
                let force = args.bool("force") ?? false
                Task { await model.refresh(force: force, trigger: "manual") }
                try await Task.sleep(for: .seconds(1))
                return state(model, account)
            },
            .query("backgroundTasks", "Background refresh requests the system still holds") { _ in
                await model.pendingBackgroundTasks()
            },
            .command("scheduleBackgroundRefresh", "Test hook: ask for both background runs after `minutes`; with force, the next one refreshes everything",
                     parameters: [
                        .integer("minutes", "Earliest start, from now", default: 15),
                        .boolean("force", "Refresh every app on the next background run, due or not", default: true),
                     ]) { args in
                UserDefaults.standard.set(args.bool("force") ?? true, forKey: LibraryModel.forceNextBackgroundKey)
                model.scheduleRefresh(after: TimeInterval((args.int("minutes") ?? 15) * 60))
                return await model.pendingBackgroundTasks()
            },
        ])
    }
}
