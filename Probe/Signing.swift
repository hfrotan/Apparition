import DebugTrace
import Foundation
import Observation

private let log = DebugLogger(subsystem: Bundle.main.bundleIdentifier ?? "Apparition", category: "Signing")

/// The user's own signing certificates and provisioning profiles (core/src/identity.rs), for
/// signing without an Apple ID: a paid developer account's .p12 and year-long profiles that
/// list this headset. Which one signs an app is the app's own choice, else the default here.
@MainActor @Observable
final class SigningStore {
    struct Identity: Codable, Sendable, Identifiable, Hashable {
        /// The certificate's SHA-1 fingerprint.
        let id: String
        let name: String
        let teamId: String?
        let teamName: String?
        /// development, distribution or other.
        let kind: String
        let serialNumber: String?
        let expires: String?
        let expired: Bool
    }

    struct Profile: Codable, Sendable, Identifiable, Hashable {
        var id: String { uuid }
        let uuid: String
        let name: String
        let teamId: String
        let teamName: String?
        /// `com.example.app`, `com.example.*` or `*`.
        let appIdPattern: String
        /// development, adHoc, enterprise or appStore.
        let kind: String
        /// get-task-allow: apps signed with it can be launched with JIT.
        let debuggable: Bool
        let platforms: [String]
        let deviceCount: Int
        let includesDevice: Bool?
        let expires: String?
        let expired: Bool
        /// The imported certificates this profile allows to sign.
        let identityIds: [String]
    }

    struct Failure: Error, CustomStringConvertible {
        let description: String
    }

    /// The signer value meaning "the signed-in Apple ID" (isideload makes the certificate
    /// and a 7-day profile).
    nonisolated static let appleID = "appleID"
    private nonisolated static let defaultSignerKey = "signWith"

    private(set) var identities: [Identity] = []
    private(set) var profiles: [Profile] = []

    /// What signs an app that hasn't chosen: `appleID` or an imported certificate's id.
    var defaultSigner: String = UserDefaults.standard.string(forKey: SigningStore.defaultSignerKey) ?? SigningStore.appleID {
        didSet { UserDefaults.standard.set(defaultSigner, forKey: Self.defaultSignerKey) }
    }

    /// Whether apps sign with an imported certificate by default, so setup needs no Apple ID.
    nonisolated static var signsWithOwnCertificate: Bool {
        (UserDefaults.standard.string(forKey: defaultSignerKey) ?? appleID) != appleID
    }

    private nonisolated static var dataDir: String { AccountModel.dataDir.path }
    private nonisolated static var udid: String? { UserDefaults.standard.string(forKey: "pairedUDID") }

    init() {
        load()
    }

    func load() {
        let reply = AccountModel.call(apparition_signing_list(Self.dataDir, Self.udid))
        struct List: Decodable { let identities: [Identity]; let profiles: [Profile] }
        guard let list = try? Self.decode(List.self, reply) else { return }
        identities = list.identities
        profiles = list.profiles
        if defaultSigner != Self.appleID, identity(defaultSigner) == nil {
            defaultSigner = Self.appleID
        }
    }

    func identity(_ id: String?) -> Identity? {
        identities.first { $0.id == id }
    }

    /// The imported profiles this certificate can sign with on this headset.
    func usableProfiles(for identity: Identity) -> [Profile] {
        profiles.filter {
            $0.identityIds.contains(identity.id) && !$0.expired && $0.kind != "appStore" && $0.includesDevice == true
        }
    }

    /// The signer for an app: its own choice while that certificate is still imported, else the default.
    func signer(for app: LibraryModel.InstalledApp) -> String {
        if let chosen = app.signWith, chosen == Self.appleID || identity(chosen) != nil { return chosen }
        return defaultSigner
    }

    func signerName(_ signer: String) -> String {
        signer == Self.appleID ? "Apple ID" : identity(signer)?.name ?? "Removed certificate"
    }

    // MARK: Import and delete

    /// Copies the picked file out of its security scope and imports it. The password opens
    /// the .p12 in the core and is not kept anywhere.
    func importCertificate(from url: URL, password: String) async throws -> Identity {
        let path = try Self.copyIn(url)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let dataDir = Self.dataDir
        let reply = await Task.detached { AccountModel.call(apparition_identity_import(dataDir, path, password)) }.value
        struct Imported: Decodable { let identity: Identity }
        let identity = try Self.decode(Imported.self, reply).identity
        log.notice("imported a signing certificate (\(identity.kind, privacy: .public))")
        load()
        return identity
    }

    func importProfile(from url: URL) async throws -> Profile {
        let path = try Self.copyIn(url)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let (dataDir, udid) = (Self.dataDir, Self.udid)
        let reply = await Task.detached { AccountModel.call(apparition_profile_import(dataDir, path, udid)) }.value
        struct Imported: Decodable { let profile: Profile }
        let profile = try Self.decode(Imported.self, reply).profile
        log.notice("imported a \(profile.kind, privacy: .public) profile")
        load()
        return profile
    }

    func deleteIdentity(_ identity: Identity) throws {
        _ = try Self.decode(Empty.self, AccountModel.call(apparition_identity_delete(Self.dataDir, identity.id)))
        log.notice("deleted a signing certificate")
        load()
    }

    func deleteProfile(_ profile: Profile) throws {
        _ = try Self.decode(Empty.self, AccountModel.call(apparition_profile_delete(Self.dataDir, profile.uuid)))
        log.notice("deleted a profile")
        load()
    }

    // MARK: Signing

    /// Signs the IPA at `path` with an imported certificate; `{ok, signedPath, bundleId, ...}`.
    nonisolated static func signFile(_ path: String, identityId: String, udid: String,
                                     stripExtensions: Bool) async -> [String: Any] {
        let dataDir = Self.dataDir
        return await Task.detached {
            AccountModel.call(apparition_sign_offline(dataDir, path, identityId, nil, udid, stripExtensions))
        }.value
    }

    // MARK: Helpers

    private struct Empty: Decodable {}

    private static func decode<T: Decodable>(_ type: T.Type, _ reply: [String: Any]) throws -> T {
        guard reply["ok"] as? Bool == true else {
            throw Failure(description: reply["error"] as? String ?? "failed")
        }
        return try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: reply))
    }

    /// A picked file (Files, iCloud Drive) is only readable inside its security scope.
    private static func copyIn(_ url: URL) throws -> String {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let copy = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + "-" + url.lastPathComponent)
        try FileManager.default.copyItem(at: url, to: copy)
        return copy.path
    }
}

@MainActor
enum SigningEndpoints {
    static func register(_ store: SigningStore) {
        DebugSurface.shared.register([
            .query("signing", "Imported certificates and profiles (no keys), and the default signer") { _ in
                store.load()
                return SigningSummary(defaultSigner: store.defaultSigner, identities: store.identities, profiles: store.profiles)
            },
            .command("setDefaultSigner", "Sign apps with \"appleID\" or an imported certificate's id by default",
                     parameters: [.string("signer", "appleID or an identity id from signing", required: true)]) { args in
                let signer = try args.requireString("signer")
                guard signer == SigningStore.appleID || store.identity(signer) != nil else {
                    throw SigningStore.Failure(description: "no imported certificate \(signer)")
                }
                store.defaultSigner = signer
                return SigningSummary(defaultSigner: store.defaultSigner, identities: store.identities, profiles: store.profiles)
            },
            .command("importProfile", "Import a .mobileprovision already in Apparition's Documents folder",
                     parameters: [.string("file", "File name inside Documents", required: true)]) { args in
                let name = try args.requireString("file")
                guard !name.contains("/") else { throw SigningStore.Failure(description: "a file name, not a path") }
                let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                return try await store.importProfile(from: documents.appendingPathComponent(name))
            },
        ])
    }

    struct SigningSummary: Encodable {
        let defaultSigner: String
        let identities: [SigningStore.Identity]
        let profiles: [SigningStore.Profile]
    }
}
