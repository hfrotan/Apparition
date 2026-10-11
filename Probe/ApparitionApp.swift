import DebugTrace
import DebugTraceServer
import DebugTraceUI
import SwiftUI

@main
struct ApparitionApp: App {
    @State private var model: ProbeModel
    @State private var account: AccountModel
    @State private var library: LibraryModel
    @State private var companions = CompanionFinder()

    init() {
        DebugTrace.configure(.init(subsystems: [Bundle.main.bundleIdentifier ?? "Apparition"]))
        DebugTraceServer.startIfRequested()
        let model = ProbeModel()
        _model = State(initialValue: model)
        ProbeEndpoints.register(model)
        let account = AccountModel()
        _account = State(initialValue: account)
        AccountEndpoints.register(account)
        let library = LibraryModel(account: account, probe: model)
        _library = State(initialValue: library)
        LibraryEndpoints.register(library, account: account)
        library.registerBackgroundTasks()
        CompanionEndpoints.register(_companions.wrappedValue, probe: model)
        DeveloperSlotsEndpoints.register()
        ShortcutActions.register(probe: model, account: account, library: library)
        IntentEndpoints.register(probe: model, account: account, library: library)
    }

    var body: some Scene {
        WindowGroup {
            ContentView(model: model, account: account, library: library, companions: companions)
                .debugApprovalPrompts()
                .task { await library.scheduleRefreshIfNeeded() }
        }
        #if os(visionOS)
        DebugApprovalWindow()
        #endif
    }
}
