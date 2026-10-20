import Foundation
import Observation

/// What the reader shows: the selected thread, its message bodies, and the
/// remote-image decision. Bodies are cached (LRU) so revisiting a thread
/// renders without touching the core (spec §13 rule 8).
@MainActor
@Observable
final class ReaderStore {
    static let cacheLimit = 200
    static let allowedSendersKey = "remoteImagesAllowedSenders"
    /// Remote images load by themselves from people the user has written
    /// to (Settings › Privacy); on unless turned off.
    static let knownSendersKey = "remoteImagesFromKnownSenders"

    static func loadsFromKnown(_ defaults: UserDefaults) -> Bool {
        defaults.object(forKey: knownSendersKey) as? Bool ?? true
    }

    /// Of these senders, those whose remote images load by themselves:
    /// always allowed, or written to (when that is on).
    static func imagesAllowed(for senders: [String], known: Set<String>, defaults: UserDefaults) -> Bool {
        let allowed = Set(defaults.stringArray(forKey: allowedSendersKey) ?? [])
            .union(loadsFromKnown(defaults) ? known : [])
        return !senders.isEmpty && senders.allSatisfy(allowed.contains)
    }

    /// Senders of the shown thread the user has written to.
    private(set) var knownSenders: Set<String> = []

    private(set) var threadID: String?
    private(set) var detail: ThreadDetail?
    private(set) var bodies: [String: RenderedBody] = [:]
    /// Inline (`cid:`) images for the thread, by content id.
    private(set) var inlineImages: [String: InlineImage] = [:]
    /// Remote images were requested for this thread in this session.
    private(set) var remoteImagesAllowedForThread = false

    @ObservationIgnored private let core: CoreClient?
    /// The user's addresses, so To lines say "Me" (set by the model).
    var ownAddresses: Set<String> = []
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private var cache: [String: RenderedBody] = [:]
    @ObservationIgnored private var cacheOrder: [String] = []
    @ObservationIgnored private var loadGeneration = 0

    init(core: CoreClient?, defaults: UserDefaults = CoreClient.appDefaults()) {
        self.core = core
        self.defaults = defaults
    }

    var hasRemoteImages: Bool { bodies.values.contains { $0.hasRemoteImages } }

    /// Remote images load if allowed for this thread or for every sender in it.
    var allowsRemoteImages: Bool {
        if remoteImagesAllowedForThread { return true }
        guard let detail else { return false }
        let senders = detail.messages.filter { !$0.isSentByMe }.compactMap { $0.from?.email.lowercased() }
        return Self.imagesAllowed(for: senders, known: knownSenders, defaults: defaults)
    }

    func show(threadID: String?) async {
        guard threadID != self.threadID else { return }
        self.threadID = threadID
        remoteImagesAllowedForThread = false
        inlineImages = [:]
        loadGeneration += 1
        let generation = loadGeneration
        guard let threadID, let core else {
            detail = nil
            bodies = [:]
            return
        }
        guard let loaded = try? await core.thread(threadID), generation == loadGeneration else { return }
        knownSenders = await core.knownSenders(Array(Set(loaded.messages.compactMap { $0.from?.email })))
        guard generation == loadGeneration else { return }
        // Show cached bodies immediately, then fill the rest.
        detail = loaded
        bodies = loaded.messages.reduce(into: [:]) { acc, m in
            if let cached = cache[m.id] { acc[m.id] = cached }
        }
        // Headers only so far (headers-first sync, tiered download):
        // download these bodies now; the thread refreshes when they land.
        let missing = loaded.messages.filter { !$0.hasBody }.map(\.id)
        if !missing.isEmpty { Task { await core.ensureBodies(missing) } }
        // Cleaned before the reader kept an email's own stylesheet: fetched
        // and cleaned again, once, then shown as it was sent.
        Task {
            guard await core.refreshStaleBodies(threadID) > 0, self.threadID == threadID else { return }
            for m in loaded.messages { cache[m.id] = nil }
            await reload()
        }
        for message in loaded.messages where bodies[message.id] == nil && message.hasBody {
            // `try?` flattens: nil means an error or no body yet.
            guard let body = try? await core.renderedBody(message.id), generation == loadGeneration else { continue }
            remember(body)
            bodies[message.id] = body
        }
        await loadInlineImages(of: loaded, generation: generation)
    }

    /// Load the shown thread again (its bodies arrived or it changed).
    func reload() async {
        guard let current = threadID else { return }
        threadID = nil
        await show(threadID: current)
    }

    /// Whether the shown thread is still waiting for a body.
    var isWaitingForBodies: Bool {
        detail?.messages.contains { !$0.hasBody } ?? false
    }

    /// Largest inline image fetched automatically.
    static let inlineImageLimit: UInt64 = 10_000_000

    /// Fetch the images a body references by content id. They come from
    /// the account's own mail, so unlike remote images they are not gated.
    private func loadInlineImages(of detail: ThreadDetail, generation: Int) async {
        guard let core else { return }
        for message in detail.messages where bodies[message.id]?.html?.contains("kaluta-cid:") == true {
            for attachment in message.attachments {
                guard let cid = attachment.contentId.map(Self.normalizedContentID), !cid.isEmpty,
                      attachment.mimeType.hasPrefix("image/"), attachment.size <= Self.inlineImageLimit,
                      inlineImages[cid] == nil,
                      let file = try? await core.attachmentFile(attachment.id),
                      generation == loadGeneration,
                      let data = try? Data(contentsOf: file.url, options: .mappedIfSafe)
                else { continue }
                inlineImages[cid] = InlineImage(data: data, mimeType: attachment.mimeType)
            }
        }
    }

    /// A message's inline images (`cid:`), from its attachments: what
    /// workflows show too. Part of the email, so no remote fetch.
    static func inlineImages(of message: MessageInfo, core: CoreClient) async -> [String: InlineImage] {
        var out: [String: InlineImage] = [:]
        for attachment in message.attachments {
            guard let cid = attachment.contentId.map(normalizedContentID), !cid.isEmpty,
                  attachment.mimeType.hasPrefix("image/"), attachment.size <= inlineImageLimit,
                  let file = try? await core.attachmentFile(attachment.id),
                  let data = try? Data(contentsOf: file.url, options: .mappedIfSafe) else { continue }
            out[cid] = InlineImage(data: data, mimeType: attachment.mimeType)
        }
        return out
    }

    /// How many inline images a message has that `inlineImages` would load.
    static func inlineImageCount(of message: MessageInfo) -> Int {
        message.attachments.count { attachment in
            attachment.contentId.map(normalizedContentID).map { !$0.isEmpty } ?? false
                && attachment.mimeType.hasPrefix("image/") && attachment.size <= inlineImageLimit
        }
    }

    static func normalizedContentID(_ cid: String) -> String {
        cid.trimmingCharacters(in: CharacterSet(charactersIn: "<> "))
    }

    /// Files attached to the thread (not inline images), oldest first.
    var attachments: [AttachmentInfo] {
        (detail?.messages ?? []).flatMap { $0.attachments.filter { !$0.isInline } }
    }

    /// The thread's files by the email they came with, oldest first:
    /// "Jennifer Kennedy · Oct 14" and its files.
    var attachmentGroups: [AttachmentGroup] {
        (detail?.messages ?? []).compactMap { m in
            let files = m.attachments.filter { !$0.isInline }
            guard !files.isEmpty else { return nil }
            let who = m.isSentByMe ? "You" : m.from.map { $0.name?.isEmpty == false ? $0.name! : $0.email } ?? "Someone"
            let when = Date(timeIntervalSince1970: TimeInterval(m.date) / 1000)
            return AttachmentGroup(messageID: m.id, label: "\(who) · \(when.formatted(.dateTime.month(.abbreviated).day()))",
                                   attachments: files)
        }
    }

    func loadRemoteImagesForThread() {
        remoteImagesAllowedForThread = true
    }

    func alwaysLoadRemoteImagesFromSenders() {
        guard let detail else { return }
        var allowed = Set(defaults.stringArray(forKey: Self.allowedSendersKey) ?? [])
        for sender in detail.messages.compactMap({ $0.from?.email.lowercased() }) {
            allowed.insert(sender)
        }
        defaults.set(Array(allowed).sorted(), forKey: Self.allowedSendersKey)
        remoteImagesAllowedForThread = true
    }

    /// The thread as reader messages, ready for `EmailDocument`.
    var documentMessages: [EmailDocument.Message] {
        (detail?.messages ?? []).map { m in
            EmailDocument.Message(
                id: m.id,
                fromName: m.from.map { $0.name ?? $0.email } ?? "(unknown sender)",
                fromEmail: m.from?.email ?? "",
                recipients: (m.to + m.cc).map { ownAddresses.contains($0.email.lowercased()) ? "Me" : ($0.name ?? $0.email) },
                date: Date(timeIntervalSince1970: TimeInterval(m.date) / 1000),
                snippet: m.snippet,
                isRead: m.isRead,
                html: bodies[m.id]?.html,
                isDraft: m.isDraft,
                attachments: m.attachments.filter { !$0.isInline }
                    .map { EmailDocument.Attachment(id: $0.id, filename: $0.filename, size: $0.size) },
                offersReplyAll: Self.offersReplyAll(m, own: ownAddresses))
        }
    }

    /// Reply all reaches more than Reply: someone else is on a message
    /// from another person, or the user's own went to more than one.
    static func offersReplyAll(_ m: MessageInfo, own: Set<String>) -> Bool {
        let others = AppModel.others(on: m, own: own)
        return m.isSentByMe ? others.count > 1 : !others.isEmpty
    }

    private func remember(_ body: RenderedBody) {
        if cache[body.messageId] == nil {
            cacheOrder.append(body.messageId)
            if cacheOrder.count > Self.cacheLimit {
                cache.removeValue(forKey: cacheOrder.removeFirst())
            }
        }
        cache[body.messageId] = body
    }
}

/// One email's files in a thread, with who sent them and when.
struct AttachmentGroup: Identifiable {
    let messageID: String
    let label: String
    let attachments: [AttachmentInfo]
    var id: String { messageID }
}
