import SwiftUI

/// The Writing Guide section's detail column: the chosen category's
/// entries, read like a document, with their scope and the quotes from
/// the user's mail behind them. Proposed rules are decided in Review mode.
struct GuideDetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let category = model.guide.selected {
            GuideCategoryDetail(category: category)
        } else {
            ContentUnavailableView("No Category Selected", systemImage: "text.book.closed")
        }
    }
}

private struct GuideCategoryDetail: View {
    @Environment(AppModel.self) private var model
    let category: GuideCategoryInfo

    var body: some View {
        let entries = model.guide.entries(in: category.id)
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text("\(category.id) \(category.name)").font(TypeRole.title)
                    Text(category.looksFor.prefix(1).uppercased() + category.looksFor.dropFirst())
                        .foregroundStyle(.secondary)
                    Text(source).font(TypeRole.caption).foregroundStyle(.secondary)
                }
                if category.id == "D1" {
                    GuideAudiences()
                }
                if category.id == "F3" {
                    // Facts have a place of their own (spec §14.11).
                    HStack(spacing: Space.m) {
                        Text("Facts have a page of their own.").foregroundStyle(.secondary)
                        Button("Open Facts") { model.openFacts() }
                            .hoverHelp("See and change the facts AI drafts may use")
                    }
                    .card(.info)
                }
                if entries.isEmpty {
                    Text(category.learned ? "Nothing yet. Learn from your sent mail, or add an entry yourself."
                         : "Nothing yet. Your mail cannot show this: answer the questions, or add an entry yourself.")
                        .foregroundStyle(.secondary)
                }
                ForEach(entries, id: \.id) { entry in
                    GuideEntryCard(entry: entry)
                }
                HStack(spacing: Space.m) {
                    Button("Add Entry…") { model.guideSheet = .edit(nil, category: category.id) }
                        .hoverHelp("Write a rule, guideline or fact for \(category.name)")
                    if category.asked || entries.isEmpty {
                        Button("Answer the Question…") { model.guideSheet = .interview(only: category.id) }
                            .hoverHelp("A short question for \(category.name); no agent needed")
                    }
                    if category.learned, category.evidence < Self.fewMessages, !model.guideRunActive {
                        Button("Improve from Sent Mail") { Task { await model.improveGuide(category.id) } }
                            .hoverHelp("Analyse the sent messages most likely to show \(category.name)")
                    }
                }
            }
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var source: String {
        switch (category.learned, category.asked) {
        case (true, true): "Learned from your mail, and asked"
        case (true, false): "Learned from your mail"
        case (false, true): "Asked: mail cannot show it"
        case (false, false): "Set by you"
        }
    }

    /// Below this many supporting messages, a category offers to improve.
    private static let fewMessages: UInt32 = 5
    private static let readingWidth: CGFloat = 720
}

/// One entry as a card: its kind, statement, scope, check and evidence.
struct GuideEntryCard: View {
    /// "AI drafts that followed it: 4 sent as written, 1 changed against it".
    static func healthLine(_ h: GuideEntryHealth) -> String {
        var parts: [String] = []
        if h.unchanged > 0 { parts.append("\(h.unchanged) sent as written") }
        if h.overridden > 0 { parts.append("\(h.overridden) changed against it") }
        return "AI drafts that followed it: " + parts.joined(separator: ", ")
    }

    @Environment(AppModel.self) private var model
    let entry: GuideEntry
    @State private var showsAllQuotes = false

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.s) {
                GuideKindChip(kind: entry.kind)
                if !entry.scope.isAlways {
                    Text(entry.scope.text).font(TypeRole.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                if entry.source == .merged, let origin = entry.origin {
                    Text("from \(origin)").font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
            Text(entry.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if let check = entry.check {
                Label(check.summary, systemImage: "checkmark.shield")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
            if entry.support > 0 || entry.contradict > 0 {
                Text(evidenceLine).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            if let health = model.guide.health[entry.id], health.unchanged + health.overridden > 0 {
                Text(Self.healthLine(health))
                    .font(TypeRole.caption)
                    .foregroundStyle(health.overridden > health.unchanged ? Tone.caution : .secondary)
            }
            let quotes = entry.evidence.filter { !$0.contradicts }
            ForEach(Array(quotes.prefix(showsAllQuotes ? quotes.count : 2).enumerated()), id: \.offset) { _, quote in
                Text("“\(quote.quote)”")
                    .font(TypeRole.meta)
                    .italic()
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
            if quotes.count > 2 {
                Button(showsAllQuotes ? "Fewer Quotes" : "All \(quotes.count) Quotes") { showsAllQuotes.toggle() }
                    .buttonStyle(.link)
                    .font(TypeRole.caption)
                    .hoverHelp("Show every quote from your mail behind this entry")
            }
            HStack(spacing: Space.m) {
                Button("Edit…") { model.guideSheet = .edit(entry, category: entry.category) }
                    .hoverHelp("Change the statement, kind, scope or check")
                Button("Delete") { Task { await model.deleteGuideEntry(entry) } } // undoable
                    .hoverHelp("Remove it from the guide (Undo brings it back)")
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(.neutral)
        .accessibilityElement(children: .contain)
    }

    private var evidenceLine: String {
        var parts = ["\(entry.support.formatted()) \(entry.support == 1 ? "message" : "messages") show this"]
        if entry.contradict > 0 { parts.append("\(entry.contradict.formatted()) go against it") }
        return parts.joined(separator: " · ")
    }
}

/// "Rule", "Guideline" or "Fact", as a small chip.
struct GuideKindChip: View {
    let kind: GuideKind

    var body: some View {
        Text(kind.title)
            .font(Font(TypeRole.chip))
            .padding(.horizontal, Space.xs)
            .padding(.vertical, Space.hair)
            .background(kind == .rule ? Tone.highlight : Tone.controlFill, in: .rect(cornerRadius: Radius.chip))
    }
}

extension GuideCheck {
    var summary: String {
        switch kind {
        case .bannedPhrase: "Checked: drafts never say “\(value)”"
        case .requiredPhrase: "Checked: drafts include “\(value)”"
        case .maxWords: "Checked: drafts stay under \(value) words"
        }
    }
}
