import SwiftUI

/// The daily review over the Writing Guide's list (spec §14.10), only
/// while there is something to say now: its progress and Pause while it
/// runs, and why it waits or failed. When it last ran and what it found
/// is in Learning Settings; Run Review Now in the toolbar's menu.
struct ReviewStatus: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let running = model.analysisProgress?.run.flatMap { $0.status == .running || $0.status == .paused ? $0 : nil }
        if running != nil || model.analysisProgress?.waiting != nil || model.analysisError != nil {
            VStack(alignment: .leading, spacing: Space.s) {
                if let run = running {
                    AnalysisRunBar(run: run)
                }
                if let waiting = model.analysisProgress?.waiting {
                    HStack(spacing: Space.m) {
                        Label(waiting, systemImage: "exclamationmark.triangle")
                            .font(TypeRole.caption)
                            .foregroundStyle(Tone.caution)
                            .fixedSize(horizontal: false, vertical: true)
                        Button("Agent Settings…") { model.openAgentSettings?() }
                            .controlSize(.small)
                            .hoverHelp("Connect Claude Code or Codex")
                    }
                }
                if let error = model.analysisError {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(TypeRole.caption)
                        .foregroundStyle(Tone.caution)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
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

/// The chosen row of the Writing Guide's Waiting section, in the detail
/// (spec §14.10): a learning decision's card, or a proposed rule's with
/// the messages behind it. The list is the flow.
struct ProposedRuleDetail: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                if let entry = model.selectedDecision {
                    GuideDecisionCard(entry: entry, isCurrent: true,
                                      accepted: model.guide.entries.first { $0.id == entry.contradictionOf })
                    Text("Return accepts, ⌫ rejects, e edits; each can be undone. The next one waiting is chosen for you.")
                        .font(TypeRole.caption).foregroundStyle(.secondary)
                } else if let proposal = model.analysis.selectedProposal {
                    ReviewProposalCard(proposal: proposal, isCurrent: true)
                    Text(proposal.watching
                         ? "Not proposed yet: your edits have shown this, but not often enough. Accepting it now is fine too."
                         : "Return accepts, ⌫ rejects, e edits; each can be undone. The next one waiting is chosen for you.")
                        .font(TypeRole.caption).foregroundStyle(.secondary)
                } else {
                    ContentUnavailableView("Nothing to Decide", systemImage: "checkmark.circle",
                                           description: Text("Rules the daily review and learning propose wait here."))
                }
            }
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
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
