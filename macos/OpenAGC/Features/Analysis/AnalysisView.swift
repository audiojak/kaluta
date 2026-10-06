import SwiftUI

/// The Analysis section's list column (spec §14.10): the one queue of
/// proposed changes. The learning runs' decisions, the daily review's
/// proposals for the writing guide, and those still collecting evidence.
/// Return accepts, ⌫ rejects, e edits; each is one change that can be
/// undone.
struct AnalysisView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.analysis.showsFacts {
            FactsList(store: model.facts)
        } else {
            proposals
        }
    }

    private var proposals: some View {
        @Bindable var analysis = model.analysis
        return List(selection: $analysis.selection) {
            if analysis.learningDecisions > 0 {
                Section("From Learning") {
                    Label(analysis.learningDecisions == 1 ? "1 decision from learning"
                          : "\(analysis.learningDecisions) decisions from learning", systemImage: "text.book.closed")
                        .tag(AnalysisStore.learningTag)
                }
            }
            if !analysis.proposals.isEmpty {
                Section {
                    ForEach(analysis.proposals, id: \.id) { proposal in
                        AnalysisProposalRow(proposal: proposal).tag(AnalysisStore.tag(proposal))
                    }
                } header: {
                    HStack {
                        Text("Writing Guide")
                        Spacer(minLength: Space.m)
                        Button("Accept All") { Task { await model.decideAnalysis(analysis.proposals, accept: true) } }
                            .buttonStyle(.link)
                            .controlSize(.small)
                            .hoverHelp("Make every change in this group; one Undo takes them all back")
                    }
                }
            }
            if !analysis.factProposals.isEmpty {
                Section {
                    ForEach(analysis.factProposals, id: \.id) { proposal in
                        AnalysisFactRow(proposal: proposal).tag(AnalysisStore.tag(proposal))
                    }
                } header: {
                    HStack {
                        Text("Facts")
                        Spacer(minLength: Space.m)
                        Button("Accept All") { Task { await model.decideFactProposals(analysis.factProposals, accept: true) } }
                            .buttonStyle(.link)
                            .controlSize(.small)
                            .hoverHelp("Add every fact in this group; one Undo takes them all back")
                    }
                }
            }
            if !analysis.watching.isEmpty {
                Section("Watching") {
                    DisclosureGroup(isExpanded: $analysis.watchingExpanded) {
                        ForEach(analysis.watching, id: \.id) { proposal in
                            AnalysisProposalRow(proposal: proposal).tag(AnalysisStore.tag(proposal))
                        }
                    } label: {
                        Text(analysis.watching.count == 1 ? "1 pattern collecting evidence"
                             : "\(analysis.watching.count) patterns collecting evidence")
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .listStyle(.inset)
        .overlay {
            if model.analysis.loaded, model.analysis.waiting == 0, model.analysis.watching.isEmpty {
                ContentUnavailableView("Nothing to Decide", systemImage: "checkmark.circle",
                                       description: Text("Each day OpenAGC compares what AI drafted with what you sent, and proposes changes to your writing guide here."))
            }
        }
        .onKeyPress(.return) { act(.accept) }
        .onKeyPress(.delete) { act(.reject) }
        .onKeyPress(characters: .init(charactersIn: "e"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            return act(.edit)
        }
        .task { await model.analysisShown() }
    }

    private enum Action { case accept, reject, edit }

    private func act(_ action: Action) -> KeyPress.Result {
        if let fact = model.analysis.selectedFactProposal {
            switch action {
            case .accept: Task { await model.decideFactProposals([fact], accept: true) }
            case .reject: Task { await model.decideFactProposals([fact], accept: false) }
            case .edit: return .ignored
            }
            return .handled
        }
        guard let proposal = model.analysis.selectedProposal else { return .ignored }
        switch action {
        case .accept: Task { await model.decideAnalysis([proposal], accept: true) }
        case .reject: Task { await model.decideAnalysis([proposal], accept: false) }
        case .edit:
            guard proposal.op != .remove else { return .ignored }
            model.guideSheet = .proposal(proposal)
        }
        return .handled
    }
}

private struct AnalysisFactRow: View {
    let proposal: AnalysisFactProposalInfo

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Image(systemName: proposal.symbol).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(proposal.headline).lineLimit(2)
                Text(proposal.subtitle).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if proposal.unseen { NewDot() }
        }
        .accessibilityElement(children: .combine)
    }
}

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
        case "category": "\(factCount) facts from Other"
        case "starter": "Your mail keeps showing facts that fit them"
        default:
            switch op {
            case .edit: "\(categoryName) · was \(beforeValue ?? "")"
            case .remove: "\(categoryName) · your mail no longer says so"
            default: "New fact · \(categoryName)"
            }
        }
    }

    var starterName: String {
        switch starter {
        case "business": "Business"
        case "freelance": "Freelance or consulting"
        case "household": "Household"
        default: "Job search"
        }
    }
}

private struct AnalysisProposalRow: View {
    let proposal: AnalysisProposalInfo

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Image(systemName: proposal.symbol).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(proposal.statement).lineLimit(2)
                Text("\(proposal.title) · \(proposal.category) · \(proposal.strength.lowercased())")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if proposal.unseen { NewDot() }
        }
        .accessibilityElement(children: .combine)
    }
}

/// Over the list: the review's progress, when it last ran and what it
/// examined, Run Now and Pause, and how much AI drafts get changed.
struct AnalysisHeader: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            CapsuleTabs(tabs: [
                CapsuleTabs.Tab(id: "proposals", title: "Proposals", symbol: "sparkle.magnifyingglass",
                                count: model.analysis.waiting),
                CapsuleTabs.Tab(id: "facts", title: "Facts", symbol: "person.text.rectangle"),
            ], selection: Binding(get: { model.analysis.showsFacts ? "facts" : "proposals" },
                                  set: { model.analysis.showsFacts = $0 == "facts" }), countNoun: "waiting")
            .accessibilityLabel("Show")
            if model.analysis.showsFacts {
                factsActions
            } else {
                review
            }
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
    }

    private var factsActions: some View {
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
        }
        .controlSize(.small)
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
                    Label("Analysis Settings", systemImage: "gearshape")
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
        var parts = ["\(run.total) \(run.total == 1 ? "draft" : "drafts") compared"]
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
                Text(run.status == .paused ? "Review paused" : "Reviewing AI drafts").font(TypeRole.groupLabel)
                Spacer(minLength: Space.m)
                Text("\(run.done) of \(run.total) compared").font(TypeRole.caption).foregroundStyle(.secondary)
            }
            ProgressView(value: run.total == 0 ? 0 : Double(run.done) / Double(run.total)).progressViewStyle(.linear)
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
}

/// The detail column: the learning runs' decisions, or the chosen proposal
/// with the messages behind it, side by side.
struct AnalysisDetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.analysis.showsFacts {
            if let fact = model.facts.selected {
                FactDetail(fact: fact, store: model.facts) { model.guideSheet = .fact($0, category: nil) }
            } else {
                ContentUnavailableView("No Fact Selected", systemImage: "person.text.rectangle")
            }
        } else if model.analysis.selection == AnalysisStore.learningTag {
            GuideDecisionsView()
        } else if let fact = model.analysis.selectedFactProposal {
            AnalysisFactDetail(proposal: fact)
        } else if let proposal = model.analysis.selectedProposal {
            AnalysisProposalDetail(proposal: proposal)
        } else {
            ContentUnavailableView("Nothing Selected", systemImage: "sparkle.magnifyingglass")
        }
    }
}

private struct AnalysisProposalDetail: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisProposalInfo

    var body: some View {
        let pairs = model.analysis.pairs
        let shown = model.analysis.showsAllPairs ? pairs : Array(pairs.prefix(Self.firstPairs))
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                change
                actions
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
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
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
                Text(proposal.statement).font(TypeRole.title).fixedSize(horizontal: false, vertical: true)
            case .edit:
                Text(proposal.beforeStatement ?? "").strikethrough().foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(proposal.statement).font(TypeRole.title).fixedSize(horizontal: false, vertical: true)
            case .rescope:
                Text(proposal.statement).font(TypeRole.title).fixedSize(horizontal: false, vertical: true)
                Text("Applies \(scopeText(proposal.beforeScope)) → \(scopeText(proposal.scope))")
                    .foregroundStyle(.secondary)
            case .remove:
                Text(proposal.statement).font(TypeRole.title).strikethrough()
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
                Task { await model.decideAnalysis([proposal], accept: true) }
            }
            .buttonStyle(.borderedProminent)
            .hoverHelp("Change your writing guide (Return); Undo takes it back")
            if proposal.op != .remove {
                Button("Edit…") { model.guideSheet = .proposal(proposal) }
                    .hoverHelp("Change it before accepting (e)")
            }
            Button("Reject") { Task { await model.decideAnalysis([proposal], accept: false) } } // undoable
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
    private static let readingWidth: CGFloat = 760
}

/// A proposed fact: what it would say, and the words in the user's mail.
private struct AnalysisFactDetail: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                Label(proposal.subtitle, systemImage: proposal.symbol).font(TypeRole.caption).foregroundStyle(.secondary)
                Text(proposal.headline).font(TypeRole.title).fixedSize(horizontal: false, vertical: true)
                if proposal.kind == "category", !proposal.description.isEmpty {
                    Text(proposal.description).foregroundStyle(.secondary)
                }
                if !proposal.quote.isEmpty {
                    VStack(alignment: .leading, spacing: Space.xs) {
                        Text("From your mail").font(TypeRole.groupLabel)
                        Text("“\(proposal.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary)
                    }
                    .card()
                }
                HStack(spacing: Space.m) {
                    Button("Accept") { Task { await model.decideFactProposals([proposal], accept: true) } }
                        .buttonStyle(.borderedProminent)
                        .hoverHelp("Add it to your facts (Return); Undo takes it back")
                    Button("Reject") { Task { await model.decideFactProposals([proposal], accept: false) } } // undoable
                        .hoverHelp("Leave it out; it will not be proposed again (⌫)")
                }
                .controlSize(.small)
            }
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private static let readingWidth: CGFloat = 720
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
                Text("The texts are no longer kept (Settings › Analysis › Keep AI drafts for).")
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
