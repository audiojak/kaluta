import SwiftUI

extension AnalysisFactProposalInfo {
    var symbol: String {
        switch (kind, op) {
        case ("category", _): "folder.badge.plus"
        case ("starter", _): "square.stack.3d.up"
        case (_, .edit): "pencil.circle"
        case (_, .remove): "minus.circle"
        default: "plus.circle"
        }
    }

    /// "Occupation or role: CTO", "New category: Sailing".
    var headline: String {
        switch kind {
        case "category": "New category: \(name)"
        case "starter": "Add the \(starterName) categories"
        default: op == .remove ? "Remove \(label)" : "\(label): \(value)"
        }
    }

    var subtitle: String {
        switch kind {
        // "New category" when none of the facts in Other would move.
        case "category": factCount == 0 ? "New category" : factCount == 1 ? "1 fact from Other" : "\(factCount) facts from Other"
        case "starter": "Your mail keeps showing facts that fit them"
        default:
            switch op {
            case .edit: "\(categoryName) · was \(beforeValue ?? "")"
            case .remove: "\(categoryName) · your mail no longer says so"
            default: "New fact · \(categoryName)"
            }
        }
    }

    /// A fact to add or change, so the user says how freely drafts use it.
    var takesUse: Bool { kind == "fact" && op != .remove }

    var starterName: String {
        switch starter {
        case "business": "Business"
        case "freelance": "Freelance or consulting"
        case "household": "Household"
        default: "Job search"
        }
    }
}

/// A proposed fact as a card in the review flow: what it would say, the
/// words in the user's mail, how freely drafts may use it, and its actions.
struct ProposedFactCard: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo
    let isCurrent: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Label(proposal.subtitle, systemImage: proposal.symbol).font(TypeRole.caption).foregroundStyle(.secondary)
            Text(proposal.headline).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if proposal.kind == "category", !proposal.description.isEmpty {
                Text(proposal.description).foregroundStyle(.secondary)
            }
            if !proposal.quote.isEmpty {
                Text("“\(proposal.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary).lineLimit(3)
            }
            if proposal.takesUse {
                FactUsePicker(proposal: proposal)
            }
            HStack(spacing: Space.m) {
                Button("Accept") { Task { await model.decideProposedFacts([proposal], accept: true) } }
                    .defaultAction(isCurrent)
                    .hoverHelp("Add it to your facts (Return); Undo takes it back")
                Button("Reject") { Task { await model.decideProposedFacts([proposal], accept: false) } } // undoable
                    .hoverHelp("Leave it out; it will not be proposed again (⌫)")
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(isCurrent ? .attention : .neutral)
        .accessibilityElement(children: .contain)
    }
}

/// The review flow for facts (spec §14.11), in Facts' detail from its
/// header's *Review Proposed Facts*: every proposed fact as a card, one
/// current. Return accepts, ⌫ rejects, j and k move; each decision can be
/// undone, and the next card becomes current.
struct ProposedFactsView: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        let proposals = model.analysis.factProposals
        let current = currentTag
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                        VStack(alignment: .leading, spacing: Space.xs) {
                            Text("Proposed Facts").font(TypeRole.title)
                            Text(proposals.isEmpty ? "Nothing waiting"
                                 : "\(proposals.count) waiting. Return accepts, ⌫ rejects; each can be undone.")
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: Space.m)
                        if !proposals.isEmpty {
                            Button("Accept All") { Task { await model.decideProposedFacts(proposals, accept: true) } }
                                .controlSize(.small)
                                .hoverHelp("Add every proposed fact; one Undo takes them all back")
                        }
                    }
                    ForEach(proposals, id: \.id) { proposal in
                        ProposedFactCard(proposal: proposal, isCurrent: AnalysisStore.tag(proposal) == current)
                            .id(AnalysisStore.tag(proposal))
                            .onTapGesture { model.facts.selection = AnalysisStore.tag(proposal) }
                            .accessibilityAddTraits(.isButton)
                    }
                    if proposals.isEmpty {
                        ContentUnavailableView("Nothing to Decide", systemImage: "checkmark.circle",
                                               description: Text("Facts the daily review finds in your mail wait here."))
                    }
                }
                .padding(Space.xxl)
                .frame(maxWidth: Self.readingWidth, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: current) { _, tag in
                if let tag { withAnimation { scroller.scrollTo(tag, anchor: .top) } }
            }
        }
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onAppear {
            focused = true
            if model.facts.selection != current { model.facts.selection = current }
        }
        .onKeyPress(.return) { decide(accept: true) }
        .onKeyPress(.delete) { decide(accept: false) }
        .onKeyPress(characters: .init(charactersIn: "jk"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            let tags = model.analysis.factProposals.map(AnalysisStore.tag)
            guard let at = currentTag.flatMap({ tags.firstIndex(of: $0) }) else { return .ignored }
            model.facts.selection = tags[min(max(at + (press.characters == "j" ? 1 : -1), 0), tags.count - 1)]
            return .handled
        }
    }

    /// The chosen card while it is there; else the first.
    private var currentTag: String? {
        let tags = model.analysis.factProposals.map(AnalysisStore.tag)
        if let chosen = model.facts.selection, tags.contains(chosen) { return chosen }
        return tags.first
    }

    private func decide(accept: Bool) -> KeyPress.Result {
        guard let proposal = model.analysis.factProposal(tagged: currentTag) else { return .ignored }
        model.facts.selection = currentTag
        Task { await model.decideProposedFacts([proposal], accept: accept) }
        return .handled
    }

    private static let readingWidth: CGFloat = 720
}

/// How freely drafts may use a proposed fact once it is accepted (spec
/// §14.11), preset to its category's default.
struct FactUsePicker: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo
    /// A pop-up menu for a row; segments in the detail.
    var compact = false

    var body: some View {
        let picker = Picker("Drafts", selection: Binding(get: { model.analysis.use(of: proposal) },
                                                         set: { model.analysis.factUses[proposal.id] = $0 })) {
            ForEach([FactUse.free, .ask, .never], id: \.self) { use in Text(use.title).tag(use) }
        }
        .fixedSize()
        .hoverHelp("Once accepted: whether AI drafts may use it, ask you first, or never see it")
        if compact {
            picker.labelsHidden().pickerStyle(.menu)
        } else {
            picker.pickerStyle(.segmented)
        }
    }
}

/// Over the Facts list: add a fact, categories, and where facts are learned
/// from (spec §14.11).
struct FactsHeader: View {
    @Environment(AppModel.self) private var model
    @State private var from: FactsFrom?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.m) {
                Button("Add Fact…") { model.guideSheet = .fact(nil, category: nil) }
                    .hoverHelp("Write a fact AI drafts may use")
                Menu {
                    Button("Add Category…") { model.guideSheet = .newFactCategory } // no-help: menu
                    Menu("Add Categories From a Starter Set") { // no-help: menu
                        ForEach(model.core?.factStarterSets() ?? [], id: \.name) { set in
                            Button(set.name) { Task { await model.addFactStarterSet(set) } } // no-help: menu
                        }
                    }
                    Button("Categories…") { model.guideSheet = .factCategories } // no-help: menu
                    Divider() // menu
                    Button("Export as Markdown…") { model.exportFacts(json: false) } // no-help: menu
                    Button("Export for Another Account…") { model.exportFacts(json: true) } // no-help: menu
                    Button("Merge Facts from a File…") { model.mergeFactsFromFile() } // no-help: menu
                } label: {
                    Label("Categories", systemImage: "folder")
                }
                .fixedSize()
                .hoverHelp("Categories, starter sets, export and merge")
                Spacer(minLength: 0)
                if model.reviewsAvailable {
                    Button {
                        model.guideSheet = .analysisSettings
                    } label: {
                        Label("Learning Settings", systemImage: "gearshape")
                    }
                    .labelStyle(.iconOnly)
                    .hoverHelp("Where facts are learned from, and the daily review")
                }
            }
            .controlSize(.small)
            let waiting = model.analysis.factProposals.count
            if waiting > 0 {
                Button {
                    model.openFacts(proposed: true)
                } label: {
                    Label(waiting == 1 ? "Review 1 Proposed Fact" : "Review \(waiting) Proposed Facts", systemImage: "checklist")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .hoverHelp("Go through the facts found in your mail: Return accepts, ⌫ rejects")
            }
            if model.reviewsAvailable, let from {
                Text(Self.learning(from, run: model.analysisProgress?.run))
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
        .task(id: model.guideSheet == nil) { from = try? await model.core?.analysisFactsFrom() }
    }

    /// "Each day's review looks for facts in mail written with AI. Last looked today."
    static func learning(_ from: FactsFrom, run: AnalysisRunInfo?) -> String {
        let source = switch from {
        case .off: "Facts are not learned from your mail."
        case .mailWrittenWithAi: "Each day's review looks for facts in the mail you send that AI helped write."
        case .allMailISend: "Each day's review looks for facts in all the mail you send."
        }
        guard from != .off, let at = run?.finishedAt, run?.status == .done else { return source }
        let when = Date(timeIntervalSince1970: TimeInterval(at) / 1000)
        let day = Calendar.current.isDateInToday(when) ? "today" : when.formatted(date: .abbreviated, time: .omitted)
        return "\(source) Last looked \(day)."
    }
}

/// The Facts page's detail: the review flow of proposed facts, or the
/// chosen fact.
struct FactsDetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.analysis.reviewingFacts {
            ProposedFactsView()
        } else if let fact = model.facts.selected {
            FactDetail(fact: fact, store: model.facts) { model.guideSheet = .fact($0, category: nil) }
        } else {
            ContentUnavailableView("No Fact Selected", systemImage: "person.text.rectangle")
        }
    }
}
