import Foundation
import Testing
@testable import Kaluta

/// The first launch's takeover of what OpenAGC left, on scratch folders,
/// throwaway Keychain services and throwaway preference suites only.
@MainActor
struct MigrationTests {
    /// One scratch world: OpenAGC's folder, Kaluta's, both Keychain
    /// services and both preference suites, removed afterwards.
    final class World {
        let root = CoreClient.testScratch()
        let oldSecrets = KeychainSecretStore(service: "org.kaluta.Kaluta.tests.\(UUID().uuidString)")
        let newSecrets = KeychainSecretStore(service: "org.kaluta.Kaluta.tests.\(UUID().uuidString)")
        /// OpenAGC's preferences, as `persistentDomain(forName:)` gives them.
        var oldPreferences: [String: Any]?
        let newSuite = "kaluta-tests-\(UUID().uuidString)"
        var running = false

        var old: URL { root.appending(path: "OpenAGC", directoryHint: .isDirectory) }
        var new: URL { root.appending(path: "Kaluta", directoryHint: .isDirectory) }
        var newPreferences: UserDefaults { UserDefaults(suiteName: newSuite)! }

        func migration() -> OpenAGCMigration {
            OpenAGCMigration(
                oldDirectory: old, newDirectory: new, oldSecrets: oldSecrets, newSecrets: newSecrets,
                oldPreferences: oldPreferences,
                newPreferences: newPreferences, oldAppRunning: { [unowned self] in running },
                now: { Date(timeIntervalSince1970: 1_791_500_000) })
        }

        /// OpenAGC's folder as a real one looks: accounts, a database with
        /// its write-ahead log, and run/ with a session socket.
        func makeOld() throws {
            let fm = FileManager.default
            let account = old.appending(path: "accounts/acct-1", directoryHint: .isDirectory)
            try fm.createDirectory(at: account, withIntermediateDirectories: true)
            try Data("db".utf8).write(to: account.appending(path: "mail.sqlite"))
            try Data("wal".utf8).write(to: account.appending(path: "mail.sqlite-wal"))
            try fm.createDirectory(at: old.appending(path: "global"), withIntermediateDirectories: true)
            try Data("facts".utf8).write(to: old.appending(path: "global/facts.sqlite"))
            try fm.createDirectory(at: old.appending(path: "run"), withIntermediateDirectories: true)
            try Data().write(to: old.appending(path: "run/mcp-1-0.sock"))
            try oldSecrets.set("gmail.token.acct-1", "secret-1")
            try oldSecrets.set("mailbox.api_key.svc-1", "secret-2")
            oldPreferences = [
                "accountID": "acct-1",
                "notifyAnalysis": true,
                "NSWindow Frame main-AppWindow-1": "{{0, 0}, {1100, 700}}",
                "com_apple_SwiftUI_Settings_selectedTabIndex": 2,
            ]
        }

        /// Every file under `dir`, relative, with its contents.
        func files(_ dir: URL) -> [String: String] {
            var out: [String: String] = [:]
            let base = dir.standardizedFileURL.path
            guard let all = FileManager.default.enumerator(at: dir, includingPropertiesForKeys: nil) else { return [:] }
            for case let url as URL in all {
                var isDir: ObjCBool = false
                guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), !isDir.boolValue else { continue }
                let path = String(url.standardizedFileURL.path.dropFirst(base.count + 1))
                out[path] = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
            }
            return out
        }

        deinit {
            try? FileManager.default.removeItem(at: root)
            try? oldSecrets.deleteAll()
            try? newSecrets.deleteAll()
            UserDefaults(suiteName: newSuite)?.removePersistentDomain(forName: newSuite)
        }
    }

    @Test func theFirstLaunchTakesOverOpenAGCsFolderItemsAndSettingsAndChangesNothingOfIts() throws {
        let w = World()
        try w.makeOld()
        let before = w.files(w.old)

        guard case .done(let report) = w.migration().run() else { Issue.record("not done"); return }
        #expect(report.folder == .copied)
        #expect(report.keychainCopied == ["gmail.token.acct-1", "mailbox.api_key.svc-1"])
        #expect(report.keychainSkipped.isEmpty)
        #expect(report.preferencesCopied == ["accountID", "notifyAnalysis"], "window state stays behind")

        var copied = w.files(w.new)
        #expect(copied.removeValue(forKey: OpenAGCMigration.markerName) != nil)
        #expect(copied == before.filter { !$0.key.hasPrefix("run/") }, "everything but the sockets")
        #expect(try w.newSecrets.get("gmail.token.acct-1") == "secret-1")
        #expect(try w.newSecrets.get("mailbox.api_key.svc-1") == "secret-2")
        #expect(w.newPreferences.string(forKey: "accountID") == "acct-1")
        #expect(w.newPreferences.object(forKey: "NSWindow Frame main-AppWindow-1") == nil)

        // OpenAGC's own copy is as it was.
        #expect(w.files(w.old) == before)
        #expect(try w.oldSecrets.get("gmail.token.acct-1") == "secret-1")
        // Nothing left beside the two folders.
        let siblings = try FileManager.default.contentsOfDirectory(atPath: w.root.path).sorted()
        #expect(siblings == ["Kaluta", "OpenAGC"])

        // The marker says what happened, and the next launch does nothing.
        let marker = try String(contentsOf: w.new.appending(path: OpenAGCMigration.markerName), encoding: .utf8)
        #expect(marker.contains("\"folder\" : \"copied\""))
        #expect(!marker.contains("secret-"), "names only")
        try w.oldSecrets.set("gmail.token.acct-2", "later")
        #expect(w.migration().run() == .alreadyDone)
        #expect(try w.newSecrets.get("gmail.token.acct-2") == nil)
    }

    @Test func anEmptyKalutaFolderGivesWayButOneWithDataIsNeverTouched() throws {
        let empty = World()
        try empty.makeOld()
        try FileManager.default.createDirectory(at: empty.new, withIntermediateDirectories: true)
        try Data().write(to: empty.new.appending(path: ".DS_Store"))
        guard case .done(let report) = empty.migration().run() else { Issue.record("not done"); return }
        #expect(report.folder == .copied)
        #expect(empty.files(empty.new)["accounts/acct-1/mail.sqlite"] == "db")

        let used = World()
        try used.makeOld()
        try FileManager.default.createDirectory(at: used.new.appending(path: "accounts"), withIntermediateDirectories: true)
        try Data("mine".utf8).write(to: used.new.appending(path: "accounts/own"))
        guard case .done(let kept) = used.migration().run() else { Issue.record("not done"); return }
        #expect(kept.folder == .kept)
        #expect(kept.keychainCopied.isEmpty && kept.preferencesCopied.isEmpty)
        #expect(used.files(used.new)["accounts/own"] == "mine")
        #expect(used.files(used.new)["accounts/acct-1/mail.sqlite"] == nil)
        #expect(try used.newSecrets.get("gmail.token.acct-1") == nil)
    }

    @Test func whileOpenAGCRunsNothingHappens() throws {
        let w = World()
        try w.makeOld()
        w.running = true
        #expect(w.migration().run() == .oldAppRunning)
        #expect(!FileManager.default.fileExists(atPath: w.new.path))
        #expect(try w.newSecrets.get("gmail.token.acct-1") == nil)
        w.running = false
        guard case .done = w.migration().run() else { Issue.record("not done once it quit"); return }
    }

    @Test func aFreshMacHasNothingToTakeOver() throws {
        let w = World()
        guard case .done(let report) = w.migration().run() else { Issue.record("not done"); return }
        #expect(report.folder == .none)
        #expect(FileManager.default.fileExists(atPath: w.new.appending(path: OpenAGCMigration.markerName).path))
    }

    @Test func anItemKalutaAlreadyHasIsKept() throws {
        let w = World()
        try w.makeOld()
        try w.newSecrets.set("gmail.token.acct-1", "newer")
        guard case .done(let report) = w.migration().run() else { Issue.record("not done"); return }
        #expect(report.keychainCopied == ["mailbox.api_key.svc-1"])
        #expect(try w.newSecrets.get("gmail.token.acct-1") == "newer")
    }

    @Test func theTestHostNeverTakesOverTheRealOpenAGC() {
        #expect(KalutaApp.isolated)
        #expect(KeychainSecretStore.realServices.contains(OpenAGCMigration.oldBundleID))
    }
}
