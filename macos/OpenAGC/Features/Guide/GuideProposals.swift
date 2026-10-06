import SwiftUI

/// The daily review in the Writing Guide's header (spec §14.10): its
/// progress, when it last ran and what it examined, Run Now and Pause,
/// and how much AI drafts get changed.
struct ReviewStatus: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        review
    }

    @ViewBuilder private var review: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            if let run = model.analysisProgress?.run, run.status == .running || run.status == .paused {
                AnalysisRunBar(run: run)
            } else if let run = model.analysisProgress?.run {
                Text(Self.summary(run, daily: model.analysisDaily)).font(TypeRole.caption).foregroundStyle(.secondary)
            } else if !model.analysisDaily {
                Text("Daily reviews are off. Run Now reviews the AI drafts you sent since the last review.")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("The first review runs today, once mail has synced. It sends the AI drafts you edited, and what you sent, to your own agent (with All mail I send, also the day's sent mail).")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let waiting = model.analysisProgress?.waiting {
                Label(waiting, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.caption)
                    .foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error = model.analysisError {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.caption)
                    .foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let metrics = model.analysis.metrics, metrics.compared > 0 {
                Text(Self.metricsText(metrics)).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            HStack(spacing: Space.m) {
                Button("Run Now") { Task { await model.runAnalysisNow() } }
                    .disabled(model.analysisRunActive)
                    .hoverHelp(model.analysisRunActive ? "A review is in progress"
                        : "Compare the AI drafts you sent since the last review now")
                if model.analysisProgress?.waiting != nil {
                    Button("Agent Settings…") { model.openAgentSettings?() }
                        .hoverHelp("Connect Claude Code or Codex")
                }
                Spacer(minLength: 0)
                Button {
                    model.guideSheet = .analysisSettings
                } label: {
                    Label("Learning Settings", systemImage: "gearshape")
                }
                .labelStyle(.iconOnly)
                .hoverHelp("The daily review, where facts are learned from, and how long AI drafts are kept")
            }
            .controlSize(.small)
        }
    }

    /// "Last reviewed today: 6 drafts compared, 3 sent as written. Next review tomorrow."
    static func summary(_ run: AnalysisRunInfo, daily: Bool = true) -> String {
        let when = run.finishedAt.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) } ?? Date()
        let day = Calendar.current.isDateInToday(when) ? "today" : when.formatted(date: .abbreviated, time: .omitted)
        var parts = [run.total == 0 ? "no edited AI drafts to compare"
                     : "\(run.total) \(run.total == 1 ? "draft" : "drafts") compared"]
        if run.unchanged > 0 { parts.append("\(run.unchanged) sent as written") }
        if run.unmatched > 0 { parts.append("\(run.unmatched) not found in Sent") }
        let status = run.status == .cancelled ? "Last review stopped" : run.status == .failed ? "Last review failed" : "Last reviewed"
        return "\(status) \(day): \(parts.joined(separator: ", ")).\(daily ? " Next review tomorrow." : "")"
    }

    /// "AI drafts changed by 18% (median, this week) · 40% sent as written".
    static func metricsText(_ m: AnalysisMetrics) -> String {
        var parts: [String] = []
        if let latest = m.weeklyMedian.compactMap({ $0 }).last {
            parts.append("AI drafts changed by \(Int((latest * 100).rounded()))% (median)")
        }
        let written = Int((Double(m.sentAsWritten) / Double(max(m.compared, 1)) * 100).rounded())
        parts.append("\(written)% sent as written over four weeks")
        return parts.joined(separator: " · ")
    }
}

/// The review's progress while it runs (spec §14.10), as learning shows it.
private struct AnalysisRunBar: View {
    @Environment(AppModel.self) private var model
    let run: AnalysisRunInfo

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            HStack {
                Text(title).font(TypeRole.groupLabel)
                Spacer(minLength: Space.m)
                if run.total > 0 {
                    Text("\(run.done) of \(run.total) compared").font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
            if comparing || run.status == .paused {
                ProgressView(value: run.total == 0 ? 0 : Double(run.done) / Double(run.total)).progressViewStyle(.linear)
            } else {
                // The facts step has no count to show.
                ProgressView().progressViewStyle(.linear)
            }
            if let error = run.error, run.status == .paused {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.caption)
                    .foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: Space.m) {
                if run.status == .paused {
                    Button("Resume") { Task { await model.resumeAnalysis() } }
                        .hoverHelp("Carry on comparing where it stopped")
                } else {
                    Button("Pause") { Task { await model.pauseAnalysis() } }
                        .hoverHelp("Pause after the current batch; nothing compared is lost")
                }
            }
            .controlSize(.small)
        }
        .accessibilityElement(children: .contain)
    }

    /// Drafts are still being compared; after them comes the facts step.
    private var comparing: Bool { run.done < run.total }

    private var title: String {
        if run.status == .paused { return "Review paused" }
        return comparing ? "Reviewing AI drafts" : "Looking for facts in your sent mail"
    }
}

/// A rule the daily review proposed, as a card in the review flow: the
/// change, its strength and actions; the current card also shows the
/// messages behind it, the AI's draft beside what the user sent.
struct ReviewProposalCard: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisProposalInfo
    let isCurrent: Bool

    var body: some View {
        let pairs = model.analysis.pairs
        let shown = model.analysis.showsAllPairs ? pairs : Array(pairs.prefix(Self.firstPairs))
        VStack(alignment: .leading, spacing: Space.l) {
            change
            actions
            if isCurrent, !pairs.isEmpty {
                VStack(alignment: .leading, spacing: Space.l) {
                    Text("What you changed").font(TypeRole.groupLabel)
                    ForEach(shown, id: \.compositionId) { pair in
                        AnalysisPairCard(pair: pair)
                    }
                    if pairs.count > shown.count {
                        Button("Why? Show All \(pairs.count) Messages") { model.analysis.showsAllPairs = true }
                            .buttonStyle(.link)
                            .hoverHelp("Every message behind this proposal, with your edits marked")
                    }
                }
            }
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(isCurrent ? .attention : .neutral)
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var change: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.s) {
                Label(proposal.title, systemImage: proposal.symbol).font(TypeRole.caption).foregroundStyle(.secondary)
                Text(categoryName).font(TypeRole.caption).foregroundStyle(.secondary)
                GuideKindChip(kind: proposal.kind)
            }
            switch proposal.op {
            case .add:
                Text(proposal.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            case .edit:
                Text(proposal.beforeStatement ?? "").strikethrough().foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(proposal.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            case .rescope:
                Text(proposal.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
                Text("Applies \(scopeText(proposal.beforeScope)) → \(scopeText(proposal.scope))")
                    .foregroundStyle(.secondary)
            case .remove:
                Text(proposal.statement).font(TypeRole.heading).strikethrough()
                    .fixedSize(horizontal: false, vertical: true)
                Text("Your edits keep going against it.").foregroundStyle(.secondary)
            }
            if let against = proposal.contradicts {
                Text("Goes against: “\(against)”").font(TypeRole.meta).foregroundStyle(Tone.caution)
            }
            if !proposal.scope.isAlways, proposal.op == .add {
                Text(proposal.scope.text).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            Text(proposal.watching ? "\(proposal.strength); waiting for more before it is proposed"
                 : proposal.strength)
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var actions: some View {
        HStack(spacing: Space.m) {
            Button(proposal.op == .remove ? "Remove from Guide" : "Accept") {
                Task { await model.decideProposedRule(proposal, accept: true) }
            }
            .defaultAction(isCurrent)
            .hoverHelp("Change your writing guide (Return); Undo takes it back")
            if proposal.op != .remove {
                Button("Edit…") { model.guideSheet = .proposal(proposal) }
                    .hoverHelp("Change it before accepting (e)")
            }
            Button("Reject") { Task { await model.decideProposedRule(proposal, accept: false) } } // undoable
                .hoverHelp("Leave the guide as it is; this will not be proposed again (⌫)")
        }
        .controlSize(.small)
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

/// The review flow (spec §14.9, §14.10), in the Writing Guide's detail
/// from its header's *Review Proposed Rules*: every proposed rule as a
/// card, the learning runs' and the daily review's, one current. Return
/// accepts, ⌫ rejects, e edits, j and k move; each decision is one change
/// that can be undone, and the next card becomes current. Those still
/// collecting evidence come last, folded.
struct ProposedRulesView: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var analysis = model.analysis
        let decisions = model.guide.decisions
        let proposals = model.analysis.proposals
        let current = currentTag
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    header
                    ForEach(decisions, id: \.id) { entry in
                        GuideDecisionCard(entry: entry, isCurrent: AnalysisStore.tag(entry) == current,
                                          accepted: model.guide.entries.first { $0.id == entry.contradictionOf })
                            .id(AnalysisStore.tag(entry))
                            .onTapGesture { model.analysis.selection = AnalysisStore.tag(entry) }
                            .accessibilityAddTraits(.isButton)
                    }
                    ForEach(proposals, id: \.id) { proposal in
                        card(proposal, current: current)
                    }
                    if decisions.isEmpty, proposals.isEmpty {
                        ContentUnavailableView("Nothing to Decide", systemImage: "checkmark.circle",
                                               description: Text("Proposed rules from learning and from the daily review wait here."))
                    }
                    if !model.analysis.watching.isEmpty {
                        DisclosureGroup(isExpanded: $analysis.watchingExpanded) {
                            VStack(alignment: .leading, spacing: Space.l) {
                                ForEach(model.analysis.watching, id: \.id) { proposal in
                                    card(proposal, current: current)
                                }
                            }
                            .padding(.top, Space.m)
                        } label: {
                            Text(model.analysis.watching.count == 1 ? "1 pattern collecting evidence"
                                 : "\(model.analysis.watching.count) patterns collecting evidence")
                                .font(TypeRole.groupLabel)
                        }
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
            if model.analysis.selection != current { model.analysis.selection = current }
        }
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

    private func card(_ proposal: AnalysisProposalInfo, current: String?) -> some View {
        ReviewProposalCard(proposal: proposal, isCurrent: AnalysisStore.tag(proposal) == current)
            .id(AnalysisStore.tag(proposal))
            .onTapGesture { model.analysis.selection = AnalysisStore.tag(proposal) }
            .accessibilityAddTraits(.isButton)
    }

    private var header: some View {
        let waiting = model.analysis.rulesWaiting
        return HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text("Proposed Rules").font(TypeRole.title)
                Text(waiting == 0 ? "Nothing waiting"
                     : "\(waiting) waiting. Return accepts, ⌫ rejects, e edits; each can be undone.")
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: Space.m)
            if waiting > 0 {
                Button("Accept All") { Task { await model.acceptAllProposedRules() } }
                    .controlSize(.small)
                    .hoverHelp("Accept every proposed rule that goes against none of yours")
            }
        }
    }

    /// Every card in order, the folded ones only while shown.
    private var tags: [String] {
        model.proposedRuleTags + (model.analysis.watchingExpanded ? model.analysis.watching.map(AnalysisStore.tag) : [])
    }

    /// The chosen card while it is there; else the first.
    private var currentTag: String? {
        let all = model.proposedRuleTags + model.analysis.watching.map(AnalysisStore.tag)
        if let chosen = model.analysis.selection, all.contains(chosen) { return chosen }
        return tags.first
    }

    private enum Action { case accept, reject, edit }

    private func act(_ action: Action) -> KeyPress.Result {
        if let entry = model.guide.decisions.first(where: { AnalysisStore.tag($0) == currentTag }) {
            switch action {
            case .accept: Task { await model.acceptDecision(entry) }
            case .reject: Task { await model.decideProposedRule(reject: entry) }
            case .edit: model.guideSheet = .edit(entry, category: entry.category)
            }
            return .handled
        }
        guard let proposal = (model.analysis.proposals + model.analysis.watching)
            .first(where: { AnalysisStore.tag($0) == currentTag }) else { return .ignored }
        switch action {
        case .accept: Task { await model.decideProposedRule(proposal, accept: true) }
        case .reject: Task { await model.decideProposedRule(proposal, accept: false) }
        case .edit:
            guard proposal.op != .remove else { return .ignored }
            model.guideSheet = .proposal(proposal)
        }
        return .handled
    }

    private func move(_ delta: Int) {
        let all = tags
        guard let at = currentTag.flatMap({ all.firstIndex(of: $0) }) else { return }
        model.analysis.selection = all[min(max(at + delta, 0), all.count - 1)]
    }

    private static let readingWidth: CGFloat = 760
}

/// One message: what the AI drafted beside what the user sent, the
/// user's edits marked.
private struct AnalysisPairCard: View {
    @Environment(AppModel.self) private var model
    let pair: AnalysisPairInfo

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack {
                Text(pair.subject.isEmpty ? "(no subject)" : pair.subject).font(TypeRole.heading).lineLimit(1)
                Spacer(minLength: Space.m)
                Text(Date(timeIntervalSince1970: TimeInterval(pair.createdAt) / 1000).formatted(date: .abbreviated, time: .omitted))
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
            if let ai = pair.aiText, let sent = pair.sentText {
                let marked = WordDiff.attributed(ai: ai, sent: sent)
                HStack(alignment: .top, spacing: Space.l) {
                    side("AI drafted", marked.ai)
                    side("You sent", marked.sent)
                }
            } else {
                Text("The texts are no longer kept (Settings › Learning › Keep AI drafts for).")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
            HStack {
                if !pair.instruction.isEmpty {
                    Text("Asked: \(pair.instruction)").font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: Space.m)
                Button("Ignore Edits to This Message") { Task { await model.ignoreAnalysisPair(pair) } }
                    .buttonStyle(.link)
                    .controlSize(.small)
                    .hoverHelp("You changed it for another reason; it no longer counts for any proposal")
            }
        }
        .card()
        .accessibilityElement(children: .contain)
    }

    private func side(_ title: String, _ text: AttributedString) -> some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(title).font(TypeRole.caption).foregroundStyle(.secondary)
            Text(text).font(TypeRole.meta).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
