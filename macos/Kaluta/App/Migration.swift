import AppKit
import Darwin
import Foundation
import os

/// The first launch of Kaluta takes over what the app left when it was
/// named OpenAGC (ADR 0017): its data folder, its Keychain items and its
/// preferences. Nothing of OpenAGC's is changed or deleted: the folder is
/// cloned (APFS, so instant and without using space), items and settings
/// are copied. A marker in Kaluta's folder means it never runs again.
///
/// Never runs under tests, scratch runs or snapshots; everything it
/// touches is passed in, so tests run it on scratch folders, throwaway
/// Keychain services and throwaway preference suites.
struct OpenAGCMigration {
    /// What one run did, kept in the marker (names only, never a secret).
    struct Report: Codable, Equatable {
        enum Folder: String, Codable {
            /// OpenAGC's folder was cloned into Kaluta's.
            case copied
            /// There was no OpenAGC folder: nothing to take over.
            case none
            /// Kaluta's folder already had data; nothing was touched.
            case kept
        }

        var migratedAt: Date
        var folder: Folder
        var keychainCopied: [String] = []
        /// Items Kaluta could not read (the user said no) or write.
        var keychainSkipped: [String] = []
        var preferencesCopied: [String] = []
    }

    enum Outcome: Equatable {
        case done(Report)
        /// The marker is there: an earlier launch did it.
        case alreadyDone
        /// OpenAGC (or an agent using its mail) is running: nothing done.
        case oldAppRunning
        case failed(String)
    }

    static let oldBundleID = "ai.actual.openagc"
    static let markerName = "migrated-from-openagc.json"
    /// Sockets of running sessions; meaningless in a copy.
    static let skippedEntries: Set<String> = ["run"]

    var oldDirectory: URL
    var newDirectory: URL
    var oldSecrets: KeychainSecretStore
    var newSecrets: KeychainSecretStore
    /// OpenAGC's preferences, as `persistentDomain(forName:)` gives them.
    var oldPreferences: [String: Any]?
    var newPreferences: UserDefaults
    var oldAppRunning: () -> Bool
    var now: () -> Date = Date.init

    func run() -> Outcome {
        let fm = FileManager.default
        let marker = newDirectory.appending(path: Self.markerName)
        if fm.fileExists(atPath: marker.path) { return .alreadyDone }

        let folder: Report.Folder
        if Self.hasData(newDirectory) {
            folder = .kept
        } else if !fm.fileExists(atPath: oldDirectory.path) {
            folder = .none
        } else {
            if oldAppRunning() { return .oldAppRunning }
            do {
                try cloneOldDirectory()
            } catch {
                return .failed(String(describing: error))
            }
            folder = .copied
        }

        var report = Report(migratedAt: now(), folder: folder)
        if folder == .copied {
            copySecrets(into: &report)
            copyPreferences(into: &report)
        }
        do {
            try fm.createDirectory(at: newDirectory, withIntermediateDirectories: true)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            encoder.dateEncodingStrategy = .iso8601
            try encoder.encode(report).write(to: marker, options: .atomic)
        } catch {
            return .failed(String(describing: error))
        }
        return .done(report)
    }

    /// A folder with anything in it but Finder's `.DS_Store`.
    static func hasData(_ dir: URL) -> Bool {
        let entries = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        return entries.contains { $0 != ".DS_Store" }
    }

    /// Clone OpenAGC's folder beside Kaluta's, then move it into place, so
    /// Kaluta's folder is either whole or not there.
    private func cloneOldDirectory() throws {
        let fm = FileManager.default
        let parent = newDirectory.deletingLastPathComponent()
        let staging = parent.appending(path: ".\(newDirectory.lastPathComponent)-migrating-\(UUID().uuidString)",
                                       directoryHint: .isDirectory)
        try fm.createDirectory(at: staging, withIntermediateDirectories: true)
        do {
            for entry in try fm.contentsOfDirectory(atPath: oldDirectory.path) where !Self.skippedEntries.contains(entry) {
                let from = oldDirectory.appending(path: entry).path
                let to = staging.appending(path: entry).path
                let flags = copyfile_flags_t(COPYFILE_ALL | COPYFILE_RECURSIVE | COPYFILE_CLONE)
                guard copyfile(from, to, nil, flags) == 0 else {
                    throw CocoaError(.fileWriteUnknown, userInfo: [
                        NSFilePathErrorKey: from,
                        NSUnderlyingErrorKey: POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO),
                    ])
                }
            }
            // An empty Kaluta folder (made by an earlier launch that found
            // nothing) gives way; one with data never reaches here.
            if fm.fileExists(atPath: newDirectory.path) {
                let dsStore = newDirectory.appending(path: ".DS_Store")
                if fm.fileExists(atPath: dsStore.path) { try fm.removeItem(at: dsStore) }
                try fm.removeItem(at: newDirectory)
            }
            try fm.moveItem(at: staging, to: newDirectory)
        } catch {
            try? fm.removeItem(at: staging)
            throw error
        }
    }

    /// Copy each item Kaluta does not have yet. Reading OpenAGC's item may
    /// ask the user (a new app reads another's item); a refusal skips it.
    private func copySecrets(into report: inout Report) {
        for key in oldSecrets.keys() {
            if (try? newSecrets.get(key)) ?? nil != nil { continue }
            do {
                guard let value = try oldSecrets.get(key) else { continue }
                try newSecrets.set(key, value)
                report.keychainCopied.append(key)
            } catch {
                report.keychainSkipped.append(key)
            }
        }
    }

    /// The app's own settings; AppKit's window and panel state stays
    /// behind (it belongs to the old windows). Only reached when Kaluta's
    /// folder had no data, so its settings are nobody's yet.
    private func copyPreferences(into report: inout Report) {
        guard let old = oldPreferences else { return }
        for key in old.keys.sorted() where !Self.isWindowState(key) {
            newPreferences.set(old[key], forKey: key)
            report.preferencesCopied.append(key)
        }
    }

    static func isWindowState(_ key: String) -> Bool {
        key.hasPrefix("NS") || key.hasPrefix("com_apple_") || key.hasPrefix("com.apple.")
    }
}

extension OpenAGCMigration {
    /// The user's real OpenAGC and Kaluta, for the app's own launch.
    static func live() -> OpenAGCMigration {
        let support = URL.applicationSupportDirectory
        return OpenAGCMigration(
            oldDirectory: support.appending(path: "OpenAGC", directoryHint: .isDirectory),
            newDirectory: support.appending(path: "Kaluta", directoryHint: .isDirectory),
            oldSecrets: KeychainSecretStore(service: oldBundleID),
            newSecrets: KeychainSecretStore(service: "org.kaluta.Kaluta"),
            // On the app's own launch these are its real preferences.
            oldPreferences: CoreClient.appDefaults().persistentDomain(forName: oldBundleID),
            newPreferences: CoreClient.appDefaults(),
            oldAppRunning: { isOldAppRunning() })
    }

    /// OpenAGC itself, or its MCP server serving an outside agent.
    static func isOldAppRunning() -> Bool {
        if !NSRunningApplication.runningApplications(withBundleIdentifier: oldBundleID).isEmpty { return true }
        let pgrep = Process()
        pgrep.executableURL = URL(filePath: "/usr/bin/pgrep")
        pgrep.arguments = ["-x", "openagc-mcp"]
        pgrep.standardOutput = FileHandle.nullDevice
        pgrep.standardError = FileHandle.nullDevice
        guard (try? pgrep.run()) != nil else { return false }
        pgrep.waitUntilExit()
        return pgrep.terminationStatus == 0
    }

    /// Run before the core opens the real data folder. Returns false when
    /// the user chose to quit.
    @MainActor
    static func runAtLaunch() -> Bool {
        precondition(!KalutaApp.isolated, "the migration never runs under tests or on a scratch directory")
        let logger = Logger(subsystem: "org.kaluta.Kaluta", category: "migration")
        let migration = live()
        var askedToQuit = false
        while true {
            switch migration.run() {
            case .alreadyDone:
                return true
            case .done(let report):
                logger.notice("""
                    took over OpenAGC: folder \(report.folder.rawValue, privacy: .public), \
                    \(report.keychainCopied.count) Keychain items copied, \
                    \(report.keychainSkipped.count) skipped, \
                    \(report.preferencesCopied.count) settings copied
                    """)
                return true
            case .oldAppRunning:
                if askedToQuit {
                    alert("OpenAGC is still open",
                          "Quit OpenAGC, and end any agent session that uses its mail, then open Kaluta again.",
                          buttons: ["Quit Kaluta"])
                    return false
                }
                let choice = alert(
                    "Quit OpenAGC to continue",
                    "Kaluta is OpenAGC's new name. The first time it opens, it copies OpenAGC's mail, accounts "
                        + "and settings, and OpenAGC must be closed while it does. OpenAGC's own copy is left as it is.",
                    buttons: ["Quit OpenAGC and Continue", "Quit Kaluta"])
                guard choice == .alertFirstButtonReturn else { return false }
                askedToQuit = true
                quitOldApp()
            case .failed(let message):
                logger.error("taking over OpenAGC failed: \(message, privacy: .public)")
                let choice = alert(
                    "Kaluta couldn't copy OpenAGC's mail and settings",
                    "Nothing of OpenAGC's was changed. You can quit and try again, or continue without them. "
                        + "(\(message))",
                    buttons: ["Quit Kaluta", "Continue Without Them"])
                return choice == .alertSecondButtonReturn
            }
        }
    }

    @MainActor
    private static func quitOldApp() {
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: oldBundleID)
        apps.forEach { $0.terminate() }
        let deadline = Date.now.addingTimeInterval(10)
        while apps.contains(where: { !$0.isTerminated }) && Date.now < deadline {
            RunLoop.current.run(until: .now.addingTimeInterval(0.2))
        }
    }

    @MainActor
    @discardableResult
    private static func alert(_ title: String, _ message: String, buttons: [String]) -> NSApplication.ModalResponse {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        buttons.forEach { alert.addButton(withTitle: $0) }
        // The app may not have finished launching: no NSApp yet.
        NSApplication.shared.activate()
        return alert.runModal()
    }
}
