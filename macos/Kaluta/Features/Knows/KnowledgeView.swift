import SwiftUI

/// What Kaluta Knows (spec §14.11 amendment 2026-11-18): one list over the
/// facts and the writing guide's entries, so the user can see in one
/// place what AI drafts rely on. Nothing moves in storage: each row opens
/// where it lives (Facts, or the Writing Guide).
struct KnowledgeItem: Identifiable, Equatable {
    enum Source: Equatable { case fact(tag: String), guide(category: String) }
    enum Tab: String, CaseIterable { case all, preferences, people }

    let id: String
    let title: String
    let detail: String
    /// "Global", "This account", or the people it is for.
    let scope: String
    let source: Source
    let tabs: Set<Tab>

    /// The accepted facts and guide entries, as one list: facts first.
    static func items(facts: [FactInfo], guide: [GuideEntry]) -> [KnowledgeItem] {
        let fromFacts = facts.filter { $0.status == .accepted }.map { f in
            var tabs: Set<Tab> = [.all]
            if f.category == "preferences" { tabs.insert(.preferences) }
            if f.category == "people" { tabs.insert(.people) }
            return KnowledgeItem(id: "fact-\(FactsStore.tag(f))", title: f.label, detail: f.value,
                                 scope: f.scope == .global ? "Global" : "This account",
                                 source: .fact(tag: FactsStore.tag(f)), tabs: tabs)
        }
        let fromGuide = guide.filter { $0.status == .accepted }.map { e in
            var tabs: Set<Tab> = [.all, .preferences]
            if !e.scope.people.isEmpty { tabs.insert(.people) }
            let scope = e.scope.people.isEmpty ? (e.scope.groups.isEmpty ? "This account" : e.scope.groups.joined(separator: ", "))
                : e.scope.people.joined(separator: ", ")
            return KnowledgeItem(id: "guide-\(e.id)", title: e.statement, detail: "Writing guide · \(e.category)",
                                 scope: scope, source: .guide(category: e.category), tabs: tabs)
        }
        return fromFacts + fromGuide
    }
}

struct KnowledgeView: View {
    @Environment(AppModel.self) private var model
    @State private var tab: String? = KnowledgeItem.Tab.all.rawValue

    var body: some View {
        let all = KnowledgeItem.items(facts: model.facts.facts, guide: model.guide.entries)
        let chosen = KnowledgeItem.Tab(rawValue: tab ?? "") ?? .all
        let shown = all.filter { $0.tabs.contains(chosen) }
        VStack(spacing: 0) {
            ListHeaderBar {
                CapsuleTabs(tabs: KnowledgeItem.Tab.allCases.map { t in
                    CapsuleTabs.Tab(id: t.rawValue, title: Self.title(t), symbol: Self.symbol(t),
                                    count: all.filter { $0.tabs.contains(t) }.count)
                }, selection: $tab, countNoun: "known")
                .accessibilityLabel("Show")
            }
            if shown.isEmpty {
                ContentUnavailableView("Nothing Here Yet", systemImage: "brain",
                                       description: Text("Facts you accept and writing guide entries appear here."))
            } else {
                List(shown) { item in
                    Button { open(item) } label: { row(item) }
                        .buttonStyle(.plain)
                        .hoverHelp(Self.help(item))
                }
            }
        }
        .task {
            await model.facts.load()
            await model.guide.load()
        }
    }

    private func row(_ item: KnowledgeItem) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(item.title).lineLimit(2)
                Text(item.detail).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            Spacer(minLength: Space.m)
            Text(item.scope)
                .font(TypeRole.caption)
                .padding(.horizontal, Space.s)
                .padding(.vertical, Space.hair)
                .background(Tone.controlFill, in: RoundedRectangle(cornerRadius: Radius.chip))
                .lineLimit(1)
        }
        .padding(.vertical, Space.xs)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private func open(_ item: KnowledgeItem) {
        switch item.source {
        case let .fact(tag):
            model.openFacts()
            model.facts.selection = tag
        case let .guide(category):
            model.selectedMailboxID = AppModel.guideMailboxID
            model.showGuideCategory(category)
        }
    }

    static func title(_ t: KnowledgeItem.Tab) -> String {
        switch t {
        case .all: "All"
        case .preferences: "Preferences"
        case .people: "People"
        }
    }

    static func symbol(_ t: KnowledgeItem.Tab) -> String {
        switch t {
        case .all: "square.stack"
        case .preferences: "slider.horizontal.3"
        case .people: "person.2"
        }
    }

    static func help(_ item: KnowledgeItem) -> String {
        switch item.source {
        case .fact: "Open this fact in Facts"
        case .guide: "Open this entry in the Writing Guide"
        }
    }
}
