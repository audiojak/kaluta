import AppKit
import KalutaCore

/// A bug report or a feature request (spec §17.1, ADR 0020): an ordinary
/// email the user reads and sends from their own account. The project's
/// handler tells the two apart by the address.
enum FeedbackKind: String, Sendable {
    case bug
    case idea

    var address: String {
        switch self {
        case .bug: "bug@support.kaluta.org"
        case .idea: "idea@support.kaluta.org"
        }
    }

    var sheetTitle: String {
        switch self {
        case .bug: "Report a Bug"
        case .idea: "Suggest a Feature"
        }
    }

    var titlePrompt: String {
        switch self {
        case .bug: "What went wrong, in a few words"
        case .idea: "What you would like, in a few words"
        }
    }

    var descriptionPrompt: String {
        switch self {
        case .bug: "What you did, what you expected and what happened"
        case .idea: "What you are trying to do, and how Kaluta could help"
        }
    }

    /// The notice once it has gone.
    var sentNotice: String {
        switch self {
        case .bug: "Bug report sent"
        case .idea: "Suggestion sent"
        }
    }
}

/// The message format the handler reads (spec §17.1): the description,
/// then two optional blocks of plain lines, each between its own marker
/// and `--- end ---`.
enum FeedbackReport {
    static let contextMarker = "--- Kaluta report ---"
    static let logMarker = "--- Kaluta log ---"
    static let endMarker = "--- end ---"
    /// The picture's name in the mail.
    static let pictureName = "screenshot.png"
    /// How many log lines a report offers.
    static let logLines: UInt32 = 40

    /// What the app adds about itself: nothing about the mailbox but its kind.
    static func context(app: String, build: String, core: String, macOS: String, account: String) -> [String] {
        ["App: \(app) (\(build))", "Core: \(core)", "macOS: \(macOS)", "Account: \(account)"]
    }

    static func body(description: String, context: [String], log: String) -> String {
        var parts = [description.trimmingCharacters(in: .whitespacesAndNewlines)]
        if !context.isEmpty {
            parts.append(([contextMarker] + context + [endMarker]).joined(separator: "\n"))
        }
        let lines = log.trimmingCharacters(in: .whitespacesAndNewlines)
        if !lines.isEmpty {
            parts.append([logMarker, lines, endMarker].joined(separator: "\n"))
        }
        return parts.filter { !$0.isEmpty }.joined(separator: "\n\n") + "\n"
    }

    static var appVersion: (version: String, build: String) {
        let info = Bundle.main.infoDictionary
        return (info?["CFBundleShortVersionString"] as? String ?? "?", info?["CFBundleVersion"] as? String ?? "?")
    }

    static var macOSVersion: String {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        return "\(v.majorVersion).\(v.minorVersion).\(v.patchVersion)"
    }

    static func accountLabel(_ kind: AccountKind?, demo: Bool) -> String {
        if demo { return "Demo" }
        return switch kind {
        case .gmail: "Gmail"
        case .agent: "Agent mailbox"
        case .archive: "Imported mailbox"
        case nil: "None"
        }
    }

    /// The picture with each cover painted over it: the pixels under a
    /// cover are gone from what is sent, not hidden by a layer. `covers`
    /// are fractions of the picture, measured from its top left.
    static func covered(_ image: CGImage, by covers: [CGRect]) -> CGImage? {
        guard !covers.isEmpty else { return image }
        let width = image.width, height = image.height
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(CGColor(gray: 0, alpha: 1))
        for cover in covers {
            let unit = cover.standardized.intersection(CGRect(x: 0, y: 0, width: 1, height: 1))
            guard !unit.isEmpty else { continue }
            // Core Graphics measures from the bottom left; whole pixels,
            // outwards, so no edge of the covered area shows through.
            let rect = CGRect(x: unit.minX * CGFloat(width), y: (1 - unit.maxY) * CGFloat(height),
                              width: unit.width * CGFloat(width), height: unit.height * CGFloat(height))
            context.fill(rect.integral)
        }
        return context.makeImage()
    }
}

/// The report being written in the sheet.
@MainActor
@Observable
final class FeedbackDraft: Identifiable {
    let kind: FeedbackKind
    /// The account it will be sent from; nil when none can send.
    let accountID: String?
    /// Its address, shown so the user knows who the mail is from.
    let sender: String?
    /// Why it cannot be sent from here (an imported mailbox, no account).
    let cannotSend: String?
    /// The lines about the app, as they will be sent.
    let context: [String]
    /// The newest warnings and errors, scrubbed, as they will be sent.
    let log: String

    var title = ""
    var details = ""
    /// The window as it was when the sheet was asked for, or a picture
    /// the user dropped in.
    var picture: CGImage?
    var includePicture: Bool
    /// Areas the user covered, as fractions of the picture from its top left.
    var covers: [CGRect] = []
    var includeLog = false
    var sending = false
    var error: String?

    init(kind: FeedbackKind, accountID: String?, sender: String?, cannotSend: String?, context: [String], log: String,
         picture: CGImage?) {
        self.kind = kind
        self.accountID = accountID
        self.sender = sender
        self.cannotSend = cannotSend
        self.context = context
        self.log = log
        self.picture = picture
        // A bug is usually something on screen; an idea usually is not.
        includePicture = kind == .bug && picture != nil
    }

    var canSend: Bool {
        cannotSend == nil && !sending && !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// The mail's text, exactly as it will be sent.
    var body: String {
        FeedbackReport.body(description: details, context: context, log: includeLog ? log : "")
    }

    /// The picture as it will be sent, or nil for none.
    var pictureToSend: CGImage? {
        guard includePicture, let picture else { return nil }
        return FeedbackReport.covered(picture, by: covers)
    }

    /// A picture file dropped on the sheet replaces the window's.
    func usePicture(at url: URL) -> Bool {
        guard let image = NSImage(contentsOf: url)?.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            return false
        }
        picture = WindowCapture.scaledToFit(image)
        covers = []
        includePicture = true
        return true
    }
}
