import DebugTrace
import Foundation
import Observation

private let log = DebugLogger(subsystem: "com.illixion.apparition.companion", category: "Anisette")

/// Runs the self-hosted anisette server (selfhost/anisette/compose.yaml, bundled) with the
/// user's own Docker, and notices one that is already running on its port however it was
/// started. Apparition ships no anisette server and never picks one: this only starts the
/// container when the user presses Start, and Apparition only uses it when the user says so.
@MainActor @Observable
final class Anisette {
    enum State: Equatable {
        case checking
        case running
        case stopped
        case noDocker
        case working(String)
        case failed(String)
    }

    nonisolated static let port: UInt16 = 6969

    private(set) var state = State.checking
    /// Called with the port while a server answers, nil otherwise.
    var onChange: ((UInt16?) -> Void)?

    private var composeFile: URL? { Bundle.main.url(forResource: "compose", withExtension: "yaml") }

    /// Docker Desktop, OrbStack, Colima and Homebrew put the CLI in different places, and an
    /// app launched from Finder gets none of the shell's PATH.
    private nonisolated static let searchPath = [
        "/opt/homebrew/bin", "/usr/local/bin",
        "\(NSHomeDirectory())/.docker/bin", "\(NSHomeDirectory())/.orbstack/bin",
        "/Applications/Docker.app/Contents/Resources/bin", "/usr/bin", "/bin",
    ]

    private nonisolated static var docker: String? {
        searchPath.map { "\($0)/docker" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    func startMonitoring() {
        Task {
            while true {
                await check()
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    func check() async {
        if case .working = state { return }
        let up = await Self.answers()
        if up {
            state = .running
        } else if Self.docker == nil {
            state = .noDocker
        } else if case .failed = state {
            // Keep the error on screen until the user tries again.
        } else {
            state = .stopped
        }
        onChange?(up ? Self.port : nil)
    }

    func start() async {
        await compose(["up", "-d"], label: "starting (the first start downloads the image)")
        // The server answers a few seconds after the container starts.
        for _ in 0..<30 where !(await Self.answers()) {
            try? await Task.sleep(for: .seconds(1))
        }
        await check()
    }

    func stop() async {
        await compose(["down"], label: "stopping")
        await check()
    }

    private func compose(_ arguments: [String], label: String) async {
        guard let docker = Self.docker, let composeFile else {
            state = .noDocker
            return
        }
        state = .working(label)
        let result = await Self.run(docker, ["compose", "-f", composeFile.path] + arguments)
        if result.status == 0 {
            state = .checking
        } else {
            let output = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            state = .failed(output.isEmpty ? "docker exited with \(result.status)" : output)
            log.error("docker compose \(arguments.joined(separator: " "), privacy: .public): \(output, privacy: .public)")
        }
    }

    /// True when the port serves anisette data (a v3 server's GET / returns the headers).
    nonisolated static func answers() async -> Bool {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:\(port)/")!)
        request.timeoutInterval = 3
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200 else { return false }
        return String(decoding: data, as: UTF8.self).contains("X-Apple-I-MD")
    }

    private nonisolated static func run(_ executable: String, _ arguments: [String]) async -> (status: Int32, output: String) {
        await withCheckedContinuation { continuation in
            let process = Process()
            process.executableURL = URL(fileURLWithPath: executable)
            process.arguments = arguments
            var environment = ProcessInfo.processInfo.environment
            // Compose and the credential helpers are found through PATH.
            environment["PATH"] = searchPath.joined(separator: ":")
            process.environment = environment
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = pipe
            process.terminationHandler = { process in
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                continuation.resume(returning: (process.terminationStatus, String(decoding: data, as: UTF8.self)))
            }
            do {
                try process.run()
            } catch {
                continuation.resume(returning: (-1, "\(error)"))
            }
        }
    }
}
