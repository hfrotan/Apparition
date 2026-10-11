import DebugTrace
import DebugTraceServer
import DebugTraceUI
import SwiftUI

@main
struct ApparitionApp: App {
    @State private var model: ProbeModel

    init() {
        DebugTrace.configure(.init(subsystems: [Bundle.main.bundleIdentifier ?? "Apparition"]))
        DebugTraceServer.startIfRequested()
        let model = ProbeModel()
        _model = State(initialValue: model)
        ProbeEndpoints.register(model)
    }

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
                .debugApprovalPrompts()
        }
        #if os(visionOS)
        DebugApprovalWindow()
        #endif
    }
}
