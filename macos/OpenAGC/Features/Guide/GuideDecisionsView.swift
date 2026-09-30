import SwiftUI

/// Decisions (spec §14.9): what a finished analysis proposed, by category,
/// each with the quotes from the user's mail behind it. Accept (↩), edit
/// (e) or reject (⌫); every decision is saved as it is made and can be
/// undone, so the user can leave and come back. A proposal that mail
/// contradicts an accepted entry offers to use it instead, keep the entry,
/// or edit the entry.
struct GuideDecisionsView: View {
    @Environment(AppModel.self) private var model
    @State private var current: Int64?
    @FocusState private var focused: Bool

    var body: some View {
        let decisions = model.guide.decisions
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    header(decisions.count)
                    ForEach(decisions, id: \.id) { entry in
                        GuideDecisionCard(entry: entry, isCurrent: entry.id == currentID(decisions),
                                          accepted: model.guide.entries.first { $0.id == entry.contradictionOf })
                            .id(entry.id)
                            .onTapGesture { current = entry.id }
                            .accessibilityAddTraits(.isButton)
                    }
                    if decisions.isEmpty {
                        ContentUnavailableView("No Decisions Waiting", systemImage: "checkmark.circle",
                                               description: Text("Everything the analysis proposed is decided"))
                    }
                }
                .padding(Space.xxl)
                .frame(maxWidth: Self.readingWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: currentID(decisions)) { _, id in
                if let id { withAnimation { scroller.scrollTo(id, anchor: .center) } }
            }
        }
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onAppear { focused = true }
        .onKeyPress(.return) { act(.accept) }
        .onKeyPress(.delete) { act(.reject) }
        .onKeyPress(characters: .init(charactersIn: "ejk"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            switch press.characters {
            case "e": return act(.edit)
            case "j": move(1)
            case "k": move(-1)
            default: return .ignored
            }
            return .handled
        }
    }

    private func header(_ waiting: Int) -> some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            HStack {
                Text("Decisions").font(TypeRole.title)
                Spacer()
                Button("Done") { model.guide.showsDecisions = false }
                    .hoverHelp("Back to the guide; the rest wait for you")
            }
            Text(waiting == 0 ? "Nothing waiting"
                 : "\(waiting) waiting. Return accepts, ⌫ rejects, e edits; each can be undone.")
                .foregroundStyle(.secondary)
        }
    }

    private func currentID(_ decisions: [GuideEntry]) -> Int64? {
        if let current, decisions.contains(where: { $0.id == current }) { return current }
        return decisions.first?.id
    }

    private enum Action { case accept, reject, edit }

    private func act(_ action: Action) -> KeyPress.Result {
        guard let id = currentID(model.guide.decisions),
              let entry = model.guide.decisions.first(where: { $0.id == id }) else { return .ignored }
        switch action {
        case .accept: Task { await model.decideGuide(entry, accept: true) }
        case .reject: Task { await model.decideGuide(entry, accept: false) }
        case .edit: model.guideSheet = .edit(entry, category: entry.category)
        }
        return .handled
    }

    private func move(_ delta: Int) {
        let ids = model.guide.decisions.map(\.id)
        guard let at = currentID(model.guide.decisions).flatMap({ ids.firstIndex(of: $0) }) else { return }
        current = ids[min(max(at + delta, 0), ids.count - 1)]
    }

    private static let readingWidth: CGFloat = 720
}

private struct GuideDecisionCard: View {
    @Environment(AppModel.self) private var model
    let entry: GuideEntry
    let isCurrent: Bool
    /// The accepted entry this proposal's mail goes against.
    let accepted: GuideEntry?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.s) {
                Text(categoryName).font(TypeRole.caption).foregroundStyle(.secondary)
                GuideKindChip(kind: entry.kind)
                if !entry.scope.isAlways {
                    Text(entry.scope.text).font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
            if let accepted {
                Text("Your mail disagrees with: “\(accepted.statement)”")
                    .font(TypeRole.meta)
                    .foregroundStyle(Tone.caution)
            }
            Text(entry.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if let check = entry.check {
                Label(check.summary, systemImage: "checkmark.shield").font(TypeRole.caption).foregroundStyle(.secondary)
            }
            Text("\(entry.support.formatted()) \(entry.support == 1 ? "message shows" : "messages show") this")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
            ForEach(Array(entry.evidence.filter { !$0.contradicts }.prefix(3).enumerated()), id: \.offset) { _, q in
                Text("“\(q.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary).lineLimit(3)
            }
            HStack(spacing: Space.m) {
                if let accepted {
                    Button("Use This Instead") { Task { await model.replaceGuideEntry(accepted, with: entry) } }
                        .hoverHelp("Replace your entry with this one (Return)")
                    Button("Keep Mine") { Task { await model.decideGuide(entry, accept: false) } }
                        .hoverHelp("Keep your entry as it is (⌫)")
                    Button("Edit Mine…") { model.guideSheet = .edit(accepted, category: accepted.category) }
                        .hoverHelp("Narrow or change your entry")
                } else {
                    Button("Accept") { Task { await model.decideGuide(entry, accept: true) } }
                        .hoverHelp("Add it to your guide (Return)")
                    Button("Edit…") { model.guideSheet = .edit(entry, category: entry.category) }
                        .hoverHelp("Change it before adding it (e)")
                    Button("Reject") { Task { await model.decideGuide(entry, accept: false) } } // undoable
                        .hoverHelp("Leave it out; it will not be proposed again (⌫)")
                }
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(isCurrent ? .attention : .neutral)
        .accessibilityElement(children: .contain)
    }

    private var categoryName: String {
        let name = model.guide.categories.first { $0.id == entry.category }?.name ?? ""
        return "\(entry.category) \(name)"
    }
}

extension AppModel {
    /// Accept or reject a proposal; undoable.
    func decideGuide(_ entry: GuideEntry, accept: Bool) async {
        await applyGuideEdits([.decide(id: entry.id, status: accept ? .accepted : .rejected)],
                              reason: accept ? "accept" : "reject", actionName: accept ? "Accept Entry" : "Reject Entry",
                              notice: accept ? "Added to your writing guide" : "Left out of your writing guide")
    }

    /// Use a proposal in place of the accepted entry it contradicts: one
    /// change, undoable.
    func replaceGuideEntry(_ old: GuideEntry, with new: GuideEntry) async {
        await applyGuideEdits([.decide(id: new.id, status: .accepted), .decide(id: old.id, status: .rejected)],
                              reason: "replace", actionName: "Replace Entry", notice: "Replaced “\(old.statement)”")
    }
}
