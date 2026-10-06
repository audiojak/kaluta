import Foundation
import Observation

/// The writing guide as the Writing Guide section shows it (spec §14.9):
/// the categories with their coverage, the entries, the audience groups,
/// and the learning run's progress.
@MainActor
@Observable
final class GuideStore {
    private(set) var categories: [GuideCategoryInfo] = []
    private(set) var entries: [GuideEntry] = []
    private(set) var groups: [AudienceGroup] = []
    /// Proposals ready to decide (finished runs only).
    private(set) var decisions: [GuideEntry] = []
    private(set) var loaded = false
    var selectedCategory: String? = "A1"
    private(set) var error: String?

    @ObservationIgnored private let core: CoreClient?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var applied = 0

    init(core: CoreClient?) {
        self.core = core
    }

    func load() async {
        guard let core else { return }
        generation += 1
        let mine = generation
        do {
            let categories = try await core.guideCategories()
            let entries = try await core.guideEntries([.accepted])
            let groups = try await core.audienceGroups()
            let decisions = try await core.guideDecisions()
            guard mine > applied else { return }
            applied = mine
            self.categories = categories
            self.entries = entries
            self.groups = groups
            self.decisions = decisions
            error = nil
        } catch let error as CoreClientError {
            self.error = error.message
        } catch {
            self.error = error.localizedDescription
        }
        loaded = true
    }

    /// The groups' names in order, A to H, with their categories.
    var sections: [(group: String, name: String, categories: [GuideCategoryInfo])] {
        var out: [(String, String, [GuideCategoryInfo])] = []
        for category in categories {
            if out.last?.0 == category.group {
                out[out.count - 1].2.append(category)
            } else {
                out.append((category.group, category.groupName, [category]))
            }
        }
        return out.map { (group: $0.0, name: $0.1, categories: $0.2) }
    }

    var selected: GuideCategoryInfo? { categories.first { $0.id == selectedCategory } }

    func entries(in category: String) -> [GuideEntry] {
        entries.filter { $0.category == category }
    }

    /// Accepted entries across the guide.
    var acceptedCount: Int { entries.count }
    /// Categories with nothing accepted yet.
    var emptyCategories: Int { categories.filter { $0.accepted == 0 }.count }
}

extension GuideKind {
    var title: String {
        switch self {
        case .rule: "Rule"
        case .guideline: "Guideline"
        case .fact: "Fact"
        }
    }
}

extension GuideScope {
    static let always = GuideScope(groups: [], people: [], messageTypes: [], languages: [])

    var isAlways: Bool { groups.isEmpty && people.isEmpty && messageTypes.isEmpty && languages.isEmpty }

    /// "for Customers; in replies".
    var text: String {
        var parts: [String] = []
        if !groups.isEmpty { parts.append("for \(groups.joined(separator: ", "))") }
        if !people.isEmpty { parts.append("to \(people.joined(separator: ", "))") }
        if !messageTypes.isEmpty {
            parts.append("in " + messageTypes.map { $0 == "new" ? "new messages" : $0 == "reply" ? "replies" : "forwards" }
                .joined(separator: ", "))
        }
        if !languages.isEmpty { parts.append("in \(languages.joined(separator: ", "))") }
        return parts.joined(separator: "; ")
    }
}

extension GuideEntry {
    var fields: GuideEntryFields {
        GuideEntryFields(category: category, kind: kind, statement: statement, scope: scope, check: check)
    }
}
