import DebugTrace
import DebugTraceServer
import DebugTraceUI
import SwiftUI

@main
struct ApparitionApp: App {
    @State private var model: ProbeModel
    @State private var account: AccountModel
    @State private var library: LibraryModel

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
    }

    var body: some Scene {
        WindowGroup {
            ContentView(model: model, account: account, library: library)
                .debugApprovalPrompts()
                // Not in init: submitting before SwiftUI registers the handler below throws.
                .task { library.scheduleRefresh() }
        }
        .backgroundTask(.appRefresh(LibraryModel.refreshTaskId)) {
            await library.refresh(force: false, trigger: "background")
        }
        #if os(visionOS)
        DebugApprovalWindow()
        #endif
    }
}
