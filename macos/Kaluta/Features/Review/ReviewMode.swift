import SwiftUI

/// Review mode (spec §14.10, §14.11): deciding what waits for the user,
/// in the whole window. Entered on purpose from the Writing Guide's or
/// Facts' band; the sidebar and the reader are out of the way until Done.
/// One mode per page: the Guide's queue holds the learning run's
/// decisions, the daily review's proposed rules and the patterns still
/// collecting evidence; Facts' queue holds the proposed facts.
enum ReviewMode: Equatable {
    case rules
    case facts

    var title: String {
        switch self {
        case .rules: "Proposed Rules"
        case .facts: "Proposed Facts"
        }
    }
}

/// One decision in the queue, as it was when the mode opened. Decided
/// ones stay in place with their outcome until the user leaves, so the
/// run is visible and Undo has somewhere to land.
struct ReviewItem: Identifiable, Equatable {
    enum Outcome: Equatable {
        case accepted, rejected, replaced
    }

    let tag: String
    let statement: String
    let caption: String
    let symbol: String
    /// A pattern short of the threshold: shown last, not yet proposed.
    var watching = false
    var id: String { tag }
}

extension AppModel {
    /// Open the mode on what waits now; the first undecided item is current.
    func enterReview(_ mode: ReviewMode) {
        guidePrompt = nil
        selectedMailboxID = mode == .rules ? Self.guideMailboxID : Self.factsMailboxID
        reviewItems = reviewQueue(for: mode)
        reviewOutcomes = [:]
        reviewMode = mode
        analysis.reviewingRules = mode == .rules
        analysis.reviewingFacts = mode == .facts
        let first = reviewItems.first { !$0.watching }?.tag ?? reviewItems.first?.tag
        if mode == .rules { analysis.selection = first } else { facts.selection = first }
    }

    /// Done or Esc: back to the page, a category or fact chosen.
    func leaveReview() {
        reviewMode = nil
        reviewItems = []
        reviewOutcomes = [:]
        analysis.reviewingRules = false
        analysis.reviewingFacts = false
        if let chosen = facts.selection, AnalysisStore.isFactProposalTag(chosen) { facts.selection = nil }
        if guide.selectedCategory == nil { guide.selectedCategory = "A1" }
    }

    /// The queue for a page, in the order the user goes down it.
    func reviewQueue(for mode: ReviewMode) -> [ReviewItem] {
        switch mode {
        case .rules:
            guide.decisions.map { entry in
                ReviewItem(tag: AnalysisStore.tag(entry), statement: entry.statement,
                           caption: Self.reviewCaption(entry, in: self),
                           symbol: entry.contradictionOf == nil ? "plus.circle" : "arrow.triangle.2.circlepath")
            } + (analysis.proposals + analysis.watching).map { proposal in
                ReviewItem(tag: AnalysisStore.tag(proposal), statement: proposal.statement,
                           caption: Self.reviewCaption(proposal, in: self), symbol: proposal.symbol,
                           watching: proposal.watching)
            }
        case .facts:
            analysis.factProposals.map { proposal in
                ReviewItem(tag: AnalysisStore.tag(proposal), statement: proposal.headline, caption: proposal.subtitle,
                           symbol: proposal.symbol)
            }
        }
    }

    /// Whether an item still waits (it is in the stores), else its outcome.
    /// Undo puts an item back in the stores, so it waits again by itself.
    func reviewOutcome(of item: ReviewItem) -> ReviewItem.Outcome? {
        reviewIsWaiting(item.tag) ? nil : reviewOutcomes[item.tag] ?? .accepted
    }

    func reviewIsWaiting(_ tag: String) -> Bool {
        if AnalysisStore.isFactProposalTag(tag) { return analysis.factProposal(tagged: tag) != nil }
        return guide.decisions.contains { AnalysisStore.tag($0) == tag }
            || (analysis.proposals + analysis.watching).contains { AnalysisStore.tag($0) == tag }
    }

    /// The current item's tag, by mode.
    var reviewSelection: String? {
        get { reviewMode == .facts ? facts.selection : analysis.selection }
        set { if reviewMode == .facts { facts.selection = newValue } else { analysis.selection = newValue } }
    }

    /// "3 of 7": decided so far, out of what the mode opened on.
    var reviewProgress: String {
        let decided = reviewItems.filter { !$0.watching && reviewOutcome(of: $0) != nil }.count
        let total = reviewItems.filter { !$0.watching }.count
        return total == 0 ? "" : "\(decided) of \(total)"
    }

    /// What is left to decide (the items still in the stores).
    var reviewWaitingCount: Int {
        reviewItems.filter { !$0.watching && reviewOutcome(of: $0) == nil }.count
    }

    enum ReviewAction { case accept, reject, edit }

    /// The keys and buttons of the mode, on the current item. The next
    /// one waiting becomes current, as the existing decisions do.
    @discardableResult
    func reviewAct(_ action: ReviewAction, on tag: String? = nil) -> Bool {
        guard let tag = tag ?? reviewSelection else { return false }
        if let entry = guide.decisions.first(where: { AnalysisStore.tag($0) == tag }) {
            switch action {
            case .accept:
                reviewOutcomes[tag] = entry.contradictionOf == nil ? .accepted : .replaced
                Task { await acceptDecision(entry) }
            case .reject:
                reviewOutcomes[tag] = .rejected
                Task { await decideProposedRule(reject: entry) }
            case .edit: guideSheet = .edit(entry, category: entry.category)
            }
            return true
        }
        if let proposal = (analysis.proposals + analysis.watching).first(where: { AnalysisStore.tag($0) == tag }) {
            switch action {
            case .accept:
                reviewOutcomes[tag] = .accepted
                Task { await decideProposedRule(proposal, accept: true) }
            case .reject:
                reviewOutcomes[tag] = .rejected
                Task { await decideProposedRule(proposal, accept: false) }
            case .edit:
                guard proposal.op != .remove else { return false }
                guideSheet = .proposal(proposal)
            }
            return true
        }
        if let proposal = analysis.factProposal(tagged: tag) {
            switch action {
            case .accept:
                reviewOutcomes[tag] = .accepted
                Task { await decideProposedFacts([proposal], accept: true) }
            case .reject:
                reviewOutcomes[tag] = .rejected
                Task { await decideProposedFacts([proposal], accept: false) }
            case .edit: return false
            }
            return true
        }
        return false
    }

    /// Accept All in the mode: every item waiting that goes against none
    /// of the user's (rules), or every proposed fact.
    func reviewAcceptAll() async {
        switch reviewMode {
        case .rules:
            for item in reviewItems where reviewOutcome(of: item) == nil { reviewOutcomes[item.tag] = .accepted }
            await acceptAllProposedRules()
        case .facts:
            for item in reviewItems where reviewOutcome(of: item) == nil { reviewOutcomes[item.tag] = .accepted }
            await decideProposedFacts(analysis.factProposals, accept: true)
        case nil: break
        }
    }

    /// Move the current item up or down the queue; j and k, the arrows.
    func reviewMove(_ delta: Int) {
        let tags = reviewItems.map(\.tag)
        guard let at = reviewSelection.flatMap({ tags.firstIndex(of: $0) }) else {
            reviewSelection = delta > 0 ? tags.first : tags.last
            return
        }
        reviewSelection = tags[min(max(at + delta, 0), tags.count - 1)]
    }

    /// "A1 Overall voice · Guideline · 2 messages show this".
    static func reviewCaption(_ entry: GuideEntry, in model: AppModel) -> String {
        let name = model.guide.categories.first { $0.id == entry.category }?.name ?? ""
        var parts = ["\(entry.category) \(name)", entry.kind.title]
        if entry.contradictionOf != nil { parts.append("goes against yours") }
        parts.append("\(entry.support.formatted()) \(entry.support == 1 ? "message shows" : "messages show") this")
        return parts.joined(separator: " · ")
    }

    /// "Change · A1 Overall voice · Seen in 2 messages".
    static func reviewCaption(_ proposal: AnalysisProposalInfo, in model: AppModel) -> String {
        let name = model.guide.categories.first { $0.id == proposal.category }?.name ?? ""
        var parts = [proposal.title, "\(proposal.category) \(name)"]
        parts.append(proposal.watching ? "\(proposal.strength), waiting for more" : proposal.strength)
        return parts.joined(separator: " · ")
    }
}

// MARK: - The band on the page

/// Under the Writing Guide's or Facts' title: how many decisions wait,
/// large, and the one way into the mode (spec §14.10, §14.11).
struct ReviewBand: View {
    @Environment(AppModel.self) private var model
    let mode: ReviewMode

    var body: some View {
        let waiting = mode == .rules ? model.analysis.rulesWaiting : model.analysis.factProposals.count
        let watching = mode == .rules ? model.analysis.watching.count : 0
        if waiting > 0 || watching > 0 {
            HStack(alignment: .center, spacing: Space.l) {
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text(waiting > 0 ? "\(waiting.formatted())" : "\(watching.formatted())")
                        .font(TypeRole.display)
                        .monospacedDigit()
                    Text(Self.words(waiting: waiting, watching: watching, mode: mode))
                        .font(TypeRole.meta)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: Space.m)
                Button(waiting > 0 ? "Review" : "Look") { model.enterReview(mode) }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp(waiting > 0 ? "Go through them one by one; Return accepts, ⌫ rejects, every decision can be undone"
                        : "Patterns in your edits not yet seen often enough to propose")
            }
            .padding(.horizontal, Space.xl)
            .padding(.vertical, Space.l)
            .card(.attention)
            .padding(.horizontal, Space.l)
            .padding(.bottom, Space.m)
            .accessibilityElement(children: .combine)
        }
    }

    /// "proposed rules waiting for you", "pattern collecting evidence".
    static func words(waiting: Int, watching: Int, mode: ReviewMode) -> String {
        if waiting > 0 {
            let noun = mode == .rules ? (waiting == 1 ? "proposed rule" : "proposed rules")
                : (waiting == 1 ? "proposed fact" : "proposed facts")
            return "\(noun) waiting for you"
        }
        return watching == 1 ? "pattern collecting evidence" : "patterns collecting evidence"
    }
}

// MARK: - The mode

/// The whole window while deciding: the queue on the left, the current
/// decision on the right in large type, Done and Accept All in the
/// toolbar. Esc leaves.
struct ReviewModeView: View {
    @Environment(AppModel.self) private var model
    let mode: ReviewMode

    var body: some View {
        HStack(spacing: 0) {
            ReviewQueueView(mode: mode)
                .frame(width: Self.queueWidth)
            PaneDivider()
            ReviewDecisionView(mode: mode)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .navigationTitle(mode.title)
        .navigationSubtitle(model.reviewProgress)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button("Done", systemImage: "chevron.left") { model.leaveReview() }
                    .help(ToolbarHelp.text(for: "Done", model: model) ?? "") // toolbar
                    .keyboardShortcut(.cancelAction)
            }
            ToolbarItem(placement: .primaryAction) {
                Button("Accept All") { Task { await model.reviewAcceptAll() } }
                    .help(ToolbarHelp.text(for: "Accept All", model: model) ?? "") // toolbar
                    .disabled(model.reviewWaitingCount == 0)
            }
        }
        .onExitCommand { model.leaveReview() }
    }

    static let queueWidth: CGFloat = 340
}

/// The queue: every decision the mode opened on, the current one
/// highlighted, decided ones dimmed with their outcome. Return accepts,
/// ⌫ rejects, e edits, j and k move.
struct ReviewQueueView: View {
    @Environment(AppModel.self) private var model
    let mode: ReviewMode
    @FocusState private var focused: Bool

    var body: some View {
        let waiting = model.reviewItems.filter { !$0.watching }
        let watching = model.reviewItems.filter(\.watching)
        List(selection: Binding(get: { model.reviewSelection }, set: { model.reviewSelection = $0 })) {
            // No heading over the queue: the toolbar's title names it.
            ForEach(waiting) { item in
                ReviewQueueRow(item: item, outcome: model.reviewOutcome(of: item)).tag(item.tag)
            }
            if !watching.isEmpty {
                Section("Collecting evidence") {
                    ForEach(watching) { item in
                        ReviewQueueRow(item: item, outcome: model.reviewOutcome(of: item)).tag(item.tag)
                    }
                }
            }
        }
        .listStyle(.inset)
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.return) { model.reviewAct(.accept) ? .handled : .ignored }
        .onDeleteCommand { model.reviewAct(.reject) }
        .onKeyPress(characters: .init(charactersIn: "ejk"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            switch press.characters {
            case "e": return model.reviewAct(.edit) ? .handled : .ignored
            case "j": model.reviewMove(1)
            case "k": model.reviewMove(-1)
            default: return .ignored
            }
            return .handled
        }
        .accessibilityLabel("Decisions")
    }
}

private struct ReviewQueueRow: View {
    let item: ReviewItem
    let outcome: ReviewItem.Outcome?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Image(systemName: outcome.map(Self.symbol) ?? item.symbol)
                .foregroundStyle(outcome == nil ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                .frame(width: Self.symbolWidth)
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(item.statement)
                    .font(TypeRole.heading)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                Text(outcome.map(Self.words) ?? item.caption)
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(.vertical, Space.s)
        .opacity(outcome == nil ? 1 : 0.55)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(item.statement). \(outcome.map(Self.words) ?? item.caption)")
    }

    static func symbol(_ outcome: ReviewItem.Outcome) -> String {
        switch outcome {
        case .accepted: "checkmark.circle.fill"
        case .rejected: "xmark.circle"
        case .replaced: "arrow.triangle.2.circlepath.circle.fill"
        }
    }

    static func words(_ outcome: ReviewItem.Outcome) -> String {
        switch outcome {
        case .accepted: "Accepted"
        case .rejected: "Left out"
        case .replaced: "Used instead of yours"
        }
    }

    private static let symbolWidth: CGFloat = 18
}

/// The current decision as a document, in large type, with the same cues
/// on both pages: a kind chip, a category chip, a source chip; a caution
/// block for a conflict; quotes as blockquotes. Its actions sit in a bar
/// at the bottom.
struct ReviewDecisionView: View {
    @Environment(AppModel.self) private var model
    let mode: ReviewMode

    var body: some View {
        let tag = model.reviewSelection
        let item = model.reviewItems.first { $0.tag == tag }
        Group {
            if let tag, let entry = model.guide.decisions.first(where: { AnalysisStore.tag($0) == tag }) {
                document { RuleDecisionBody(entry: entry) } actions: { ruleActions(entry) }
            } else if let tag, let proposal = (model.analysis.proposals + model.analysis.watching)
                .first(where: { AnalysisStore.tag($0) == tag }) {
                document { ProposalDecisionBody(proposal: proposal) } actions: { proposalActions(proposal) }
            } else if let proposal = model.analysis.factProposal(tagged: tag) {
                document { FactDecisionBody(proposal: proposal) } actions: { factActions(proposal) }
            } else if let item, let outcome = model.reviewOutcome(of: item) {
                DecidedView(item: item, outcome: outcome)
            } else {
                ReviewSummaryView(mode: mode)
            }
        }
        .task(id: tag) { await model.analysis.loadPairs() }
    }

    private func document(@ViewBuilder _ body: () -> some View, @ViewBuilder actions: () -> some View) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                body()
            }
            .padding(Space.page)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            HStack(spacing: Space.l) {
                actions()
                Spacer(minLength: 0)
                Text("Return accepts · ⌫ rejects · ⌘Z undoes")
                    .font(TypeRole.caption)
                    .foregroundStyle(.tertiary)
            }
            .controlSize(.large)
            .padding(.horizontal, Space.page)
            .padding(.vertical, Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.bar)
        }
    }

    @ViewBuilder private func ruleActions(_ entry: GuideEntry) -> some View {
        if entry.contradictionOf != nil {
            Button("Use This Instead") { model.reviewAct(.accept, on: AnalysisStore.tag(entry)) }
                .buttonStyle(.borderedProminent)
                .hoverHelp("Replace your entry with this one (Return); Undo takes it back")
            Button("Keep Mine") { model.reviewAct(.reject, on: AnalysisStore.tag(entry)) }
                .hoverHelp("Keep your entry as it is (⌫)")
        } else {
            Button("Accept") { model.reviewAct(.accept, on: AnalysisStore.tag(entry)) }
                .buttonStyle(.borderedProminent)
                .hoverHelp("Add it to your writing guide (Return); Undo takes it back")
            Button("Edit…") { model.reviewAct(.edit, on: AnalysisStore.tag(entry)) }
                .hoverHelp("Change it before adding it (e)")
            Button("Reject") { model.reviewAct(.reject, on: AnalysisStore.tag(entry)) } // undoable
                .hoverHelp("Leave it out; it will not be proposed again (⌫)")
        }
    }

    @ViewBuilder private func proposalActions(_ proposal: AnalysisProposalInfo) -> some View {
        Button(proposal.op == .remove ? "Remove from Guide" : "Accept") {
            model.reviewAct(.accept, on: AnalysisStore.tag(proposal))
        }
        .buttonStyle(.borderedProminent)
        .hoverHelp("Change your writing guide (Return); Undo takes it back")
        if proposal.op != .remove {
            Button("Edit…") { model.reviewAct(.edit, on: AnalysisStore.tag(proposal)) }
                .hoverHelp("Change it before accepting (e)")
        }
        Button("Reject") { model.reviewAct(.reject, on: AnalysisStore.tag(proposal)) } // undoable
            .hoverHelp("Leave the guide as it is; this will not be proposed again (⌫)")
    }

    @ViewBuilder private func factActions(_ proposal: AnalysisFactProposalInfo) -> some View {
        Button("Accept") { model.reviewAct(.accept, on: AnalysisStore.tag(proposal)) }
            .buttonStyle(.borderedProminent)
            .hoverHelp("Add it to your facts (Return); Undo takes it back")
        Button("Reject") { model.reviewAct(.reject, on: AnalysisStore.tag(proposal)) } // undoable
            .hoverHelp("Leave it out; it will not be proposed again (⌫)")
    }

    static let readingWidth: CGFloat = 680
}

// MARK: - Cues

/// The cues at the top of a decision: what kind of thing it is, where in
/// the guide it goes, and where it came from.
private struct CueRow: View {
    let kind: String
    var kindStrong = false
    let category: String?
    let source: String

    var body: some View {
        HStack(spacing: Space.s) {
            Cue(text: kind, fill: kindStrong ? AnyShapeStyle(Tone.highlight) : AnyShapeStyle(Tone.controlFill))
            if let category { Cue(text: category, fill: AnyShapeStyle(Tone.controlFill)) }
            Cue(text: source, fill: AnyShapeStyle(Tone.Intent.info.fill))
        }
        .accessibilityElement(children: .combine)
    }
}

private struct Cue: View {
    let text: String
    let fill: AnyShapeStyle

    var body: some View {
        Text(text)
            .font(TypeRole.groupLabel)
            .padding(.horizontal, Space.m)
            .padding(.vertical, Space.xs)
            .background(fill, in: .rect(cornerRadius: Radius.control))
    }
}

/// A quote from the user's mail, as a blockquote.
private struct QuoteBlock: View {
    let text: String
    var heading: String?

    var body: some View {
        HStack(alignment: .top, spacing: Space.l) {
            RoundedRectangle(cornerRadius: Radius.chip)
                .fill(.tint.opacity(0.5))
                .frame(width: Self.ruleWidth)
            VStack(alignment: .leading, spacing: Space.xs) {
                if let heading { Text(heading).font(TypeRole.caption).foregroundStyle(.secondary) }
                Text("“\(text)”").font(TypeRole.reading).italic().foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private static let ruleWidth: CGFloat = 3
}

/// A conflict: what in the guide this goes against.
private struct ConflictBlock: View {
    let statement: String
    let lead: String

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Label(lead, systemImage: "exclamationmark.triangle").font(TypeRole.groupLabel).foregroundStyle(Tone.caution)
            Text("“\(statement)”").font(TypeRole.reading).fixedSize(horizontal: false, vertical: true)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(.caution)
    }
}

private struct SectionLabel: View {
    let text: String
    var body: some View { Text(text).font(TypeRole.groupLabel).foregroundStyle(.secondary).textCase(.uppercase) }
}

// MARK: - Bodies

/// A learning decision: the proposed entry, the entry of the user's it
/// goes against, and the messages that show it.
private struct RuleDecisionBody: View {
    @Environment(AppModel.self) private var model
    let entry: GuideEntry

    var body: some View {
        CueRow(kind: entry.kind.title, kindStrong: entry.kind == .rule, category: categoryName,
               source: "From learning your sent mail")
        Text(entry.statement).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
        if !entry.scope.isAlways {
            Text("Applies \(entry.scope.text)").font(TypeRole.reading).foregroundStyle(.secondary)
        }
        if let check = entry.check {
            Label(check.summary, systemImage: "checkmark.shield").font(TypeRole.reading).foregroundStyle(.secondary)
        }
        if let against = model.guide.entries.first(where: { $0.id == entry.contradictionOf }) {
            ConflictBlock(statement: against.statement, lead: "Your mail disagrees with this entry of yours")
        }
        let quotes = entry.evidence.filter { !$0.contradicts }.prefix(3)
        if !quotes.isEmpty {
            VStack(alignment: .leading, spacing: Space.l) {
                SectionLabel(text: "\(entry.support.formatted()) \(entry.support == 1 ? "message shows" : "messages show") this")
                ForEach(Array(quotes.enumerated()), id: \.offset) { _, quote in QuoteBlock(text: quote.quote) }
            }
        }
    }

    private var categoryName: String {
        let name = model.guide.categories.first { $0.id == entry.category }?.name ?? ""
        return "\(entry.category) \(name)"
    }
}

/// A rule the daily review proposed: the change, its strength and the
/// messages behind it, the AI's draft beside what the user sent.
private struct ProposalDecisionBody: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisProposalInfo

    var body: some View {
        CueRow(kind: proposal.kind.title, kindStrong: proposal.kind == .rule, category: categoryName,
               source: proposal.watching ? "Collecting evidence" : "From the daily review")
        VStack(alignment: .leading, spacing: Space.s) {
            Text(proposal.title).font(TypeRole.groupLabel).foregroundStyle(.secondary)
            switch proposal.op {
            case .add:
                Text(proposal.statement).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
            case .edit:
                Text(proposal.beforeStatement ?? "").font(TypeRole.reading).strikethrough().foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(proposal.statement).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
            case .rescope:
                Text(proposal.statement).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
                Text("Applies \(scopeText(proposal.beforeScope)) → \(scopeText(proposal.scope))")
                    .font(TypeRole.reading).foregroundStyle(.secondary)
            case .remove:
                Text(proposal.statement).font(TypeRole.display).strikethrough()
                    .fixedSize(horizontal: false, vertical: true)
                Text("Your edits keep going against it.").font(TypeRole.reading).foregroundStyle(.secondary)
            }
            if !proposal.scope.isAlways, proposal.op == .add {
                Text(proposal.scope.text).font(TypeRole.reading).foregroundStyle(.secondary)
            }
        }
        if let against = proposal.contradicts {
            ConflictBlock(statement: against, lead: "Goes against an entry of yours")
        }
        let pairs = model.analysis.pairs
        let shown = model.analysis.showsAllPairs ? pairs : Array(pairs.prefix(Self.firstPairs))
        VStack(alignment: .leading, spacing: Space.l) {
            SectionLabel(text: proposal.watching ? "\(proposal.strength), waiting for more" : proposal.strength)
            if proposal.watching {
                Text("Not proposed yet: your edits have shown this, but not often enough. Accepting it now is fine too.")
                    .font(TypeRole.reading).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            ForEach(shown, id: \.compositionId) { pair in
                AnalysisPairCard(pair: pair)
            }
            if pairs.count > shown.count {
                Button("Show All \(pairs.count) Messages") { model.analysis.showsAllPairs = true }
                    .buttonStyle(.link)
                    .hoverHelp("Every message behind this proposal, with your edits marked")
            }
        }
    }

    private var categoryName: String {
        let name = model.guide.categories.first { $0.id == proposal.category }?.name ?? ""
        return "\(proposal.category) \(name)"
    }

    private func scopeText(_ scope: GuideScope?) -> String {
        guard let scope, !scope.isAlways else { return "always" }
        return scope.text
    }

    private static let firstPairs = 3
}

/// A proposed fact, category or starter set: what it would say, the
/// words in the user's mail, and how freely drafts may use it.
private struct FactDecisionBody: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo

    var body: some View {
        CueRow(kind: kindWords, category: proposal.kind == "fact" ? proposal.categoryName : nil, source: "From the daily review")
        VStack(alignment: .leading, spacing: Space.s) {
            switch proposal.kind {
            case "category":
                Text("New category").font(TypeRole.groupLabel).foregroundStyle(.secondary)
                Text(proposal.name).font(TypeRole.display)
                if !proposal.description.isEmpty {
                    Text(proposal.description).font(TypeRole.reading).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if proposal.factCount > 0 {
                    Text(proposal.factCount == 1 ? "1 fact would move here from Other" : "\(proposal.factCount) facts would move here from Other")
                        .font(TypeRole.reading).foregroundStyle(.secondary)
                }
            case "starter":
                Text("Starter set").font(TypeRole.groupLabel).foregroundStyle(.secondary)
                Text("Add the \(proposal.starterName) categories").font(TypeRole.display)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Your mail keeps showing facts that fit them.").font(TypeRole.reading).foregroundStyle(.secondary)
            default:
                Text(proposal.op == .remove ? "Remove" : proposal.label).font(TypeRole.groupLabel).foregroundStyle(.secondary)
                if proposal.op == .remove {
                    Text("\(proposal.label): \(proposal.value)").font(TypeRole.display).strikethrough()
                        .fixedSize(horizontal: false, vertical: true)
                    Text("Your mail no longer says so.").font(TypeRole.reading).foregroundStyle(.secondary)
                } else {
                    if let before = proposal.beforeValue {
                        Text(before).font(TypeRole.reading).strikethrough().foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(proposal.value).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        if !proposal.quote.isEmpty {
            VStack(alignment: .leading, spacing: Space.l) {
                SectionLabel(text: "In your mail")
                QuoteBlock(text: proposal.quote)
            }
        }
        if proposal.takesUse {
            VStack(alignment: .leading, spacing: Space.s) {
                SectionLabel(text: "Drafts may")
                FactUsePicker(proposal: proposal)
                Text(Self.useWords(model.analysis.use(of: proposal)))
                    .font(TypeRole.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var kindWords: String {
        switch proposal.kind {
        case "category": "Category"
        case "starter": "Starter set"
        default: proposal.op == .edit ? "Changed fact" : proposal.op == .remove ? "Fact to remove" : "Fact"
        }
    }

    /// What each choice means, in one line.
    static func useWords(_ use: FactUse) -> String {
        switch use {
        case .free: "AI drafts may use it whenever it fits, without asking."
        case .ask: "AI drafts ask you before using it."
        case .never: "AI drafts never see it; it is kept for you alone."
        }
    }
}

/// A decided item chosen again: what happened, and Undo while it is the
/// latest change.
private struct DecidedView: View {
    @Environment(AppModel.self) private var model
    let item: ReviewItem
    let outcome: ReviewItem.Outcome

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xxl) {
            Label(ReviewQueueRow.words(outcome), systemImage: ReviewQueueRow.symbol(outcome))
                .font(TypeRole.title).foregroundStyle(.secondary)
            Text(item.statement).font(TypeRole.display).fixedSize(horizontal: false, vertical: true)
            Text(item.caption).font(TypeRole.reading).foregroundStyle(.secondary)
            HStack(spacing: Space.l) {
                Button("Undo") { model.undo.undo(in: model.openAccountID) }
                    .controlSize(.large)
                    .hoverHelp("Take back the latest decision (⌘Z); it waits again")
                Text("Undo takes decisions back one at a time, latest first.")
                    .font(TypeRole.caption).foregroundStyle(.tertiary)
            }
        }
        .padding(Space.page)
        .frame(maxWidth: ReviewDecisionView.readingWidth, alignment: .leading)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

/// Nothing left to decide: the run's summary, until Done.
struct ReviewSummaryView: View {
    @Environment(AppModel.self) private var model
    let mode: ReviewMode

    var body: some View {
        let items = model.reviewItems.filter { !$0.watching }
        let accepted = items.filter { [.accepted, .replaced].contains(model.reviewOutcome(of: $0)) }.count
        let rejected = items.filter { model.reviewOutcome(of: $0) == .rejected }.count
        VStack(alignment: .leading, spacing: Space.xxl) {
            Label(items.isEmpty ? "Nothing to Decide" : "All Decided", systemImage: "checkmark.circle")
                .font(TypeRole.display)
            if !items.isEmpty {
                Text(Self.summary(accepted: accepted, rejected: rejected, mode: mode))
                    .font(TypeRole.reading).foregroundStyle(.secondary)
            } else {
                Text(mode == .rules ? "Rules the daily review and learning propose wait here."
                     : "Facts the daily review finds in your mail wait here.")
                    .font(TypeRole.reading).foregroundStyle(.secondary)
            }
            HStack(spacing: Space.l) {
                Button("Done") { model.leaveReview() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Back to the page (Return or Esc)")
                if accepted + rejected > 0 {
                    Button("Undo") { model.undo.undo(in: model.openAccountID) }
                        .controlSize(.large)
                        .hoverHelp("Take back the latest decision (⌘Z); it waits again")
                }
            }
        }
        .padding(Space.page)
        .frame(maxWidth: ReviewDecisionView.readingWidth, alignment: .leading)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// "5 added to your writing guide, 2 left out."
    static func summary(accepted: Int, rejected: Int, mode: ReviewMode) -> String {
        let home = mode == .rules ? "your writing guide" : "your facts"
        var parts: [String] = []
        if accepted > 0 { parts.append("\(accepted) added to \(home)") }
        if rejected > 0 { parts.append("\(rejected) left out") }
        return parts.joined(separator: ", ") + "."
    }
}
