import DebugTrace
import DebugTraceServer
import DebugTraceUI
import SwiftUI

@main
struct ApparitionApp: App {
    @State private var model: ProbeModel
    @State private var account: AccountModel

    init() {
        DebugTrace.configure(.init(subsystems: [Bundle.main.bundleIdentifier ?? "Apparition"]))
        DebugTraceServer.startIfRequested()
        let model = ProbeModel()
        _model = State(initialValue: model)
        ProbeEndpoints.register(model)
        let account = AccountModel()
        _account = State(initialValue: account)
        AccountEndpoints.register(account)
    }

    var body: some Scene {
        WindowGroup {
            ContentView(model: model, account: account)
                .debugApprovalPrompts()
        }
        #if os(visionOS)
        DebugApprovalWindow()
        #endif
    }
}
