import Foundation
import Observation

/// Facts as the Facts tab (or Settings › Facts, for the global ones) shows
/// them (spec §14.11): by category, with the categories to put them in.
@MainActor
@Observable
final class FactsStore {
    /// `account`: the open account's facts with the global ones merged in;
    /// `global`: the global store only (Settings).
    let scope: FactScope
    private(set) var facts: [FactInfo] = []
    private(set) var categories: [FactCategoryInfo] = []
    private(set) var loaded = false
    private(set) var error: String?
    /// The chosen fact, by `FactsStore.tag`.
    var selection: String?

    @ObservationIgnored private let core: CoreClient?

    init(core: CoreClient?, scope: FactScope = .account) {
        self.core = core
        self.scope = scope
    }

    /// A fact's list tag: its scope and id (global and account ids overlap).
    nonisolated static func tag(_ fact: FactInfo) -> String { "\(fact.scope == .global ? "g" : "a")\(fact.id)" }

    func load() async {
        guard let core else { return }
        do {
            if scope == .global {
                facts = try await core.globalFacts()
                categories = try await core.globalFactCategories()
            } else {
                facts = try await core.facts()
                categories = try await core.factCategories()
            }
            error = nil
        } catch {
            self.error = error.message
        }
        loaded = true
        if let selection, !facts.contains(where: { Self.tag($0) == selection }) { self.selection = nil }
    }

    var selected: FactInfo? { facts.first { Self.tag($0) == selection } }

    /// Categories with facts, in order (built-ins first, then the user's
    /// own), each with its facts by label. Hidden categories are left out.
    var sections: [(category: FactCategoryInfo, facts: [FactInfo])] {
        categories.filter { !$0.hidden }.compactMap { c in
            let mine = facts.filter { $0.category == c.key }
                .sorted { $0.label.localizedCaseInsensitiveCompare($1.label) == .orderedAscending }
            return mine.isEmpty ? nil : (c, mine)
        }
    }

    func category(_ key: String) -> FactCategoryInfo? { categories.first { $0.key == key } }

    func name(of key: String) -> String { category(key)?.name ?? key }

    /// Categories a new fact can go in: shown ones.
    var choosable: [FactCategoryInfo] { categories.filter { !$0.hidden } }
}

extension FactInfo {
    /// Unique across the account and global stores, whose ids overlap.
    var listTag: String { FactsStore.tag(self) }
}

extension FactUse {
    var title: String {
        switch self {
        case .free: "Use freely"
        case .ask: "Ask before using"
        case .never: "Never share"
        }
    }
}

extension FactSource {
    var title: String {
        switch self {
        case .you: "You"
        case .learned: "Learned from your mail"
        case .writingHelp: "Answered for writing help"
        }
    }
}
