import SwiftUI

/// What Kaluta Knows (spec §14.11 amendment 2026-11-18): one list over the
/// facts and the writing guide's entries, so the user can see in one
/// place what AI drafts rely on. Nothing moves in storage: each row opens
/// where it lives (Facts, or the Writing Guide), and can be edited or
/// deleted from here with the editors those pages use (amended
/// 2026-11-21). A filter field narrows the list by its words, so a
/// person's name or address finds what Kaluta knows about them.
struct KnowledgeItem: Identifiable, Equatable {
    enum Source: Equatable { case fact(FactInfo), guide(GuideEntry), contact(ContactFactInfo) }
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
                                 source: .fact(f), tabs: tabs)
        }
        let fromGuide = guide.filter { $0.status == .accepted }.map { e in
            var tabs: Set<Tab> = [.all, .preferences]
            if !e.scope.people.isEmpty { tabs.insert(.people) }
            let scope = e.scope.people.isEmpty ? (e.scope.groups.isEmpty ? "This account" : e.scope.groups.joined(separator: ", "))
                : e.scope.people.joined(separator: ", ")
            return KnowledgeItem(id: "guide-\(e.id)", title: e.statement, detail: "Writing guide · \(e.category)",
                                 scope: scope, source: .guide(e), tabs: tabs)
        }
        return fromFacts + fromGuide
    }

    /// What is known about contacts (spec §14.16), for the People tab: each
    /// accepted item with whom it is about.
    static func items(contactFacts: [ContactFactInfo], names: [Int64: String]) -> [KnowledgeItem] {
        contactFacts.filter { $0.status == .accepted }.map { f in
            let who = names[f.personId] ?? "A contact"
            return KnowledgeItem(id: "contact-\(f.id)", title: f.kind == .tag ? "Tag: \(f.value)" : f.label,
                                 detail: f.kind == .tag ? "Contacts · \(f.tag?.set.title ?? "Tag")" : "\(f.kindTitle) · \(f.value)",
                                 scope: who, source: .contact(f), tabs: [.all, .people])
        }
    }

    /// Whether every word of `filter` is in the row's title, detail or
    /// scope, whatever its case or accents. An empty filter matches all.
    func matches(_ filter: String) -> Bool {
        Self.matches(filter, in: "\(title) \(detail) \(scope)")
    }

    static func matches(_ filter: String, in text: String) -> Bool {
        filter.split(whereSeparator: \.isWhitespace)
            .allSatisfy { text.range(of: $0, options: [.caseInsensitive, .diacriticInsensitive]) != nil }
    }
}

struct KnowledgeView: View {
    @Environment(AppModel.self) private var model
    @State private var tab: String? = KnowledgeItem.Tab.all.rawValue
    @State private var filter = ""
    @State private var contactFacts: [ContactFactInfo] = []

    var body: some View {
        let names = Dictionary(model.contacts.list.compactMap { c in c.personId.map { ($0, c.name) } }, uniquingKeysWith: { a, _ in a })
        let all = KnowledgeItem.items(facts: model.facts.facts, guide: model.guide.entries)
            + KnowledgeItem.items(contactFacts: contactFacts, names: names)
        let chosen = KnowledgeItem.Tab(rawValue: tab ?? "") ?? .all
        let inTab = all.filter { $0.tabs.contains(chosen) }
        let shown = inTab.filter { $0.matches(filter) }
        VStack(spacing: 0) {
            ListHeaderBar {
                CapsuleTabs(tabs: KnowledgeItem.Tab.allCases.map { t in
                    CapsuleTabs.Tab(id: t.rawValue, title: Self.title(t), symbol: Self.symbol(t),
                                    count: all.filter { $0.tabs.contains(t) }.count)
                }, selection: $tab, countNoun: "known")
                .accessibilityLabel("Show")
            }
            if !inTab.isEmpty {
                HStack(spacing: Space.s) {
                    Image(systemName: "line.3.horizontal.decrease").foregroundStyle(.secondary).accessibilityHidden(true)
                    TextField("Filter by words or a person", text: $filter)
                        .textFieldStyle(.plain)
                        .accessibilityLabel("Filter what Kaluta knows")
                }
                .font(TypeRole.meta)
                .padding(.horizontal, Space.xl)
                .padding(.vertical, Space.s)
            }
            if inTab.isEmpty {
                ContentUnavailableView("Nothing Here Yet", systemImage: "brain",
                                       description: Text("Facts you accept and writing guide entries appear here."))
            } else if shown.isEmpty {
                ContentUnavailableView.search(text: filter)
            } else {
                List(shown) { item in
                    HStack(spacing: Space.s) {
                        Button { open(item) } label: { row(item) }
                            .buttonStyle(.plain)
                            .hoverHelp(Self.help(item))
                        Menu {
                            actions(item)
                        } label: {
                            Image(systemName: "ellipsis")
                        }
                        .menuStyle(.borderlessButton)
                        .menuIndicator(.hidden)
                        .fixedSize()
                        .accessibilityLabel("Change “\(item.title)”")
                        .hoverHelp("Edit or delete this")
                    }
                    .contextMenu { actions(item) }
                }
            }
        }
        .task(id: model.contacts.list.count + model.contacts.facts.count) {
            await model.facts.load()
            await model.guide.load()
            await model.contacts.load()
            contactFacts = (try? await model.core?.acceptedContactFacts()) ?? []
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
        case let .fact(fact):
            model.openFacts()
            model.facts.selection = FactsStore.tag(fact)
        case let .guide(entry):
            model.selectedMailboxID = AppModel.guideMailboxID
            model.showGuideCategory(entry.category)
        case let .contact(item):
            model.contacts.selection = ContactsStore.tag(person: item.personId)
            model.openContacts()
        }
    }

    /// Edit and delete where it is listed, with the page's own editor and
    /// the same undoable delete (UX review 2026-11, F14).
    @ViewBuilder private func actions(_ item: KnowledgeItem) -> some View {
        Button("Edit…") { // no-help: menu
            switch item.source {
            case let .fact(fact): model.guideSheet = .fact(fact, category: nil)
            case let .guide(entry): model.guideSheet = .edit(entry, category: entry.category)
            // A contact's item is edited on the contact's page.
            case .contact: open(item)
            }
        }
        Button("Delete") { // no-help: menu
            Task {
                switch item.source {
                case let .fact(fact): await model.deleteFact(fact)
                case let .guide(entry): await model.deleteGuideEntry(entry)
                case let .contact(item):
                    let edit: ContactEdit = item.kind == .tag && item.tag != nil
                        ? .removeTag(personId: item.personId, tagId: item.tag?.id ?? "") : .delete(id: item.id)
                    await model.applyContactEdits([edit], actionName: "Delete", notice: "Deleted “\(item.headline)”")
                    contactFacts = (try? await model.core?.acceptedContactFacts()) ?? []
                }
            }
        }
        Divider() // menu
        Button(Self.help(item)) { open(item) } // no-help: menu
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
        case .contact: "Open this contact's page"
        }
    }
}
