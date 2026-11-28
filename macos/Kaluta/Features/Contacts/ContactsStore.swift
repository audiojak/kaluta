import Foundation
import Observation

/// Contacts as the Contacts page shows them (spec §14.16): the list with
/// its tag filter, and the chosen contact's page. Waits, tasks and recent
/// conversations are found by query each time; nothing is copied here.
@MainActor
@Observable
final class ContactsStore {
    /// A sheet over the Contacts page.
    enum Sheet: Identifiable, Equatable {
        case fact(ContactFactInfo?)
        case addTag
        case samePerson
        case dive
        case review([ContactFactInfo])

        var id: String {
            switch self {
            case let .fact(f): "fact\(f?.id ?? 0)"
            case .addTag: "addTag"
            case .samePerson: "samePerson"
            case .dive: "dive"
            case .review: "review"
            }
        }
    }

    private(set) var list: [ContactSummary] = []
    private(set) var tags: [ContactTagInfo] = []
    /// Items held for review, across contacts (the sidebar's count).
    private(set) var held = 0
    private(set) var learning: ContactLearning?
    private(set) var batch: ContactBatch?
    private(set) var loaded = false
    private(set) var error: String?

    /// Tags a listed contact must have, all of them.
    var filterTagIDs: [String] = []
    var search = ""
    /// The chosen row, by `ContactsStore.tag`.
    var selection: String?
    var sheet: Sheet?
    /// A sheet over Learning Settings, which has its own window or dialog.
    var settingsSheet: SettingsSheet?

    enum SettingsSheet: String, Identifiable {
        case pastMail, tags
        var id: String { rawValue }
    }

    /// The chosen contact's page.
    private(set) var contact: ContactInfo?
    private(set) var facts: [ContactFactInfo] = []
    private(set) var links: ContactLinks?
    private(set) var dive: ContactDive?

    @ObservationIgnored private let core: CoreClient?
    @ObservationIgnored private let reads = SerialReads()

    init(core: CoreClient?) {
        self.core = core
    }

    /// A row's list tag: its contact, or (a search result with no contact yet) its address.
    nonisolated static func tag(_ row: ContactSummary) -> String {
        row.personId.map { "p\($0)" } ?? "e\(row.email.lowercased())"
    }

    nonisolated static func tag(person id: Int64) -> String { "p\(id)" }

    func load() async {
        await reads.run { [weak self] in await self?.read() }
    }

    private func read() async {
        guard let core else { return }
        do {
            // Read together, so the list, its tags and the count agree.
            let list = try await core.contacts(tagIDs: filterTagIDs, search: search)
            let tags = try await core.contactTags()
            let held = try await core.heldContactFactCount()
            self.list = list
            self.tags = tags
            self.held = Int(held)
            learning = try? await core.contactLearning()
            batch = try? await core.contactBatch()
            error = nil
        } catch {
            self.error = error.message
        }
        loaded = true
        await readPage()
    }

    /// The person the selection names, making their contact if the row was
    /// only an address (opening the page is a first need).
    private func selectedPersonID() async -> Int64? {
        guard let selection, let core else { return nil }
        if selection.hasPrefix("p") { return Int64(selection.dropFirst()) }
        guard selection.hasPrefix("e"), let made = try? await core.contact(forAddress: String(selection.dropFirst()))
        else { return nil }
        self.selection = Self.tag(person: made.personId)
        return made.personId
    }

    private func readPage() async {
        guard let core, let id = await selectedPersonID() else {
            contact = nil
            facts = []
            links = nil
            dive = nil
            return
        }
        do {
            let contact = try await core.contact(id)
            let facts = try await core.contactFacts(id)
            let links = try await core.contactLinks(id)
            self.contact = contact
            self.facts = facts
            self.links = links
            dive = try? await core.contactDive(id)
        } catch {
            // The contact is gone (joined to another, or another account's).
            contact = nil
            facts = []
            links = nil
            dive = nil
            selection = nil
        }
    }

    /// Open the page of whoever has this address. `false`: not a person.
    func open(email: String) async -> String? {
        guard let core else { return "No account is open" }
        do {
            let made = try await core.contact(forAddress: email)
            selection = Self.tag(person: made.personId)
            await load()
            return nil
        } catch {
            return error.message
        }
    }

    /// Nothing of the last account's is shown in this one.
    func reset() {
        list = []
        tags = []
        held = 0
        learning = nil
        batch = nil
        loaded = false
        filterTagIDs = []
        search = ""
        selection = nil
        sheet = nil
        settingsSheet = nil
        contact = nil
        facts = []
        links = nil
        dive = nil
    }

    // MARK: The page's sections

    /// Held for the user's review, oldest first.
    var needsReview: [ContactFactInfo] {
        facts.filter { $0.status == .proposed }.sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
    }

    private var accepted: [ContactFactInfo] { facts.filter { $0.status == .accepted } }

    var openTopics: [ContactFactInfo] { accepted.filter { $0.kind == .topic && $0.open == true } }

    func commitments(owedBy party: ContactParty) -> [ContactFactInfo] {
        accepted.filter { $0.kind == .commitment && $0.open == true && $0.owedBy == party }
    }

    /// Topics and commitments that are finished, newest first.
    var closed: [ContactFactInfo] { accepted.filter { $0.kind != .fact && $0.kind != .tag && $0.open == false } }

    /// Accepted facts by class, in the classes' order.
    var factsByClass: [(heading: ContactFactClass, facts: [ContactFactInfo])] {
        ContactFactClass.allCases.compactMap { c in
            let mine = accepted.filter { $0.kind == .fact && $0.class == c }
            return mine.isEmpty ? nil : (c, mine)
        }
    }

    /// The held item that would change this one, if there is one.
    func heldChange(to item: ContactFactInfo) -> ContactFactInfo? {
        facts.first { $0.status == .proposed && $0.changesId == item.id }
    }

    /// Whether the page has anything learned or written on it.
    var hasItems: Bool { !facts.isEmpty || !(contact?.tags.isEmpty ?? true) }

    // MARK: Tags

    /// The tag list by set and heading, in order, narrowed to names with
    /// a word starting with `typed` (any word: "eng" finds Engineering).
    nonisolated static func grouped(_ tags: [ContactTagInfo], typed: String = "") -> [(set: ContactTagSet, heading: String, tags: [ContactTagInfo])] {
        var out: [(set: ContactTagSet, heading: String, tags: [ContactTagInfo])] = []
        for tag in tags where matches(tag.name, typed: typed) {
            if let at = out.firstIndex(where: { $0.set == tag.set && $0.heading == tag.heading }) {
                out[at].tags.append(tag)
            } else {
                out.append((tag.set, tag.heading, [tag]))
            }
        }
        return out
    }

    /// Whether a tag's name answers what was typed: every typed word is
    /// the start of one of the name's words. Hyphens stand for spaces.
    nonisolated static func matches(_ name: String, typed: String) -> Bool {
        let words = { (s: String) in
            s.lowercased().replacingOccurrences(of: "-", with: " ").split(whereSeparator: \.isWhitespace).map(String.init)
        }
        let wanted = words(typed)
        if wanted.isEmpty { return true }
        let have = words(name)
        return wanted.allSatisfy { w in have.contains { $0.hasPrefix(w) } }
    }
}

extension ContactTagSet {
    var title: String {
        switch self {
        case .relationship: "Relationship"
        case .role: "Role"
        }
    }
}

extension ContactFactClass {
    /// Every class, in the order the page lists them.
    static var allCases: [ContactFactClass] {
        [.identity, .role, .contactDetail, .preference, .personal, .topic, .money, .routing, .agreement, .deadline, .other]
    }

    var title: String {
        switch self {
        case .identity: "Identity"
        case .role: "Role"
        case .contactDetail: "Contact detail"
        case .preference: "Preference"
        case .personal: "Personal"
        case .topic: "Topic"
        case .money: "Money"
        case .routing: "Where to send things"
        case .agreement: "Agreement"
        case .deadline: "Deadline"
        case .other: "Other"
        }
    }
}

extension ContactFactKind {
    var title: String {
        switch self {
        case .fact: "Fact"
        case .topic: "Topic"
        case .commitment: "Commitment"
        case .tag: "Tag"
        }
    }

    var symbol: String {
        switch self {
        case .fact: "person.text.rectangle"
        case .topic: "bubble.left.and.bubble.right"
        case .commitment: "checkmark.seal"
        case .tag: "tag"
        }
    }
}

extension SaidBy {
    var title: String {
        switch self {
        case .you: "You said"
        case .them: "They said"
        case .someoneElse: "Someone else said"
        }
    }
}

extension ContactFactInfo {
    /// What the item says in a line: a tag's name, else its label.
    var headline: String { kind == .tag ? value : label }

    /// "They owe you", "You owe them", or its kind.
    var kindTitle: String {
        guard kind == .commitment else { return kind.title }
        return owedBy == .you ? "You owe them" : "They owe you"
    }
}
