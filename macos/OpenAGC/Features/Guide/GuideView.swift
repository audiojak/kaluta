import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// What the Writing Guide section shows over the list and reader.
enum GuideSheet: Identifiable {
    case learn
    case edit(GuideEntry?, category: String)
    /// The interview; `only` asks one category's question again.
    case interview(only: String?)
    case change
    case merge
    /// A rule the daily review proposed, edited before it is accepted.
    case proposal(AnalysisProposalInfo)
    /// A fact of this account (spec §14.11): new (nil) or to edit.
    case fact(FactInfo?, category: String?)
    case newFactCategory
    case factCategories
    case analysisSettings

    var id: String {
        switch self {
        case let .fact(f, category): "fact-\(f.map { FactsStore.tag($0) } ?? "new")-\(category ?? "")"
        case .newFactCategory: "new-fact-category"
        case .factCategories: "fact-categories"
        case .analysisSettings: "analysis-settings"
        case let .proposal(p): "proposal-\(p.id)"
        case .learn: "learn"
        case let .interview(only): "interview-\(only ?? "all")"
        case .change: "change"
        case .merge: "merge"
        case let .edit(entry, category): "edit-\(entry?.id ?? 0)-\(category)"
        }
    }
}

/// The Writing Guide section's list column (spec §14.9): every category
/// by group with how much of the guide covers it. What waits for a
/// decision is counted on the band over it and decided in Review mode.
struct GuideView: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var guide = model.guide
        List(selection: Binding(get: { model.guide.selectedCategory }, set: { model.showGuideCategory($0) })) {
            ForEach(guide.sections, id: \.group) { section in
                Section(section.name) {
                    ForEach(section.categories, id: \.id) { category in
                        GuideCategoryRow(category: category).tag(category.id)
                            .background(FocusRegionProbe(cycle: model.focus, region: .list))
                    }
                }
            }
        }
        .listStyle(.inset)
        // Tab from the sidebar (the main window's loop, spec §14.3).
        .focused($focused)
        .onChange(of: model.threadListFocusRequests) { focused = true }
        .overlay {
            if model.guide.loaded, model.guide.categories.isEmpty {
                ContentUnavailableView("No Writing Guide", systemImage: "text.book.closed")
            }
        }
        .task { await model.guide.load() }
    }
}

/// Over the category list while something runs (spec §14.9, §14.10): the
/// learning run's progress bars, or the daily review's; nothing otherwise.
/// The section's actions are in the toolbar (`ListToolbar`), its
/// explanation a tip, and the last run's summary in Learning Settings.
struct GuideActivityStrip: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let progress = model.guideProgress, model.guideRunActive {
            GuideProgressBars(progress: progress)
                .padding(.horizontal, Space.l)
                .padding(.vertical, Space.m)
        } else if model.reviewsAvailable {
            ReviewStatus()
        }
    }
}

/// The two progress bars (spec §14.9): analysis, then decisions.
struct GuideProgressBars: View {
    @Environment(AppModel.self) private var model
    let progress: GuideProgress

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            if let run = progress.run, run.status == .running || run.status == .paused {
                bar(title: run.status == .paused ? "Learning paused" : "Learning",
                    detail: "\(run.done.formatted()) of \(run.total.formatted()) messages, batch \(min(run.batchesDone + 1, run.batches)) of \(run.batches)",
                    value: run.total == 0 ? 0 : Double(run.done) / Double(run.total))
                if let left = Self.timeLeft(run) {
                    Text(left)
                        .font(TypeRole.caption)
                        .foregroundStyle(.secondary)
                }
                if let error = run.error, run.status == .paused {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(TypeRole.caption)
                        .foregroundStyle(Tone.caution)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: Space.m) {
                    if run.status == .paused {
                        Button("Resume") { Task { await model.resumeGuideRun() } }
                            .hoverHelp("Carry on analysing where it stopped")
                        if run.error != nil {
                            Button("Agent Settings…") { model.openAgentSettings?() }
                                .hoverHelp("Connect Claude Code or Codex")
                        }
                    } else {
                        Button("Pause") { Task { try? await model.core?.pauseGuideRun() } }
                            .hoverHelp("Pause after the current batch; nothing analysed is lost")
                    }
                    Button("Stop") { Task { await model.cancelGuideRun() } }
                        .hoverHelp("Stop for good; what was analysed so far becomes decisions")
                }
                .controlSize(.small)
                bar(title: "Decisions", detail: "waiting for analysis", value: nil)
            } else {
                bar(title: "Decisions",
                    detail: "\(progress.decisionsDone.formatted()) of \(progress.decisionsTotal.formatted()) decided",
                    value: progress.decisionsTotal == 0 ? 0 : Double(progress.decisionsDone) / Double(progress.decisionsTotal))
            }
        }
    }

    /// "About 12 minutes left", from the batches timed so far; while the
    /// first batch runs there is nothing to go on yet.
    static func timeLeft(_ run: GuideRunInfo) -> String? {
        guard run.status == .running || run.status == .paused else { return nil }
        guard let seconds = run.secondsLeft else {
            return run.status == .running ? "Estimating the time left after the first batch…" : nil
        }
        let minutes = Int((Double(seconds) / 60).rounded())
        let paused = run.status == .paused ? " once resumed" : ""
        switch minutes {
        case ..<1: return "Less than a minute left\(paused)"
        case 1: return "About a minute left\(paused)"
        case ..<90: return "About \(minutes) minutes left\(paused)"
        default:
            let hours = Double(minutes) / 60
            return "About \(hours.formatted(.number.precision(.fractionLength(0...1)))) hours left\(paused)"
        }
    }

    private func bar(title: String, detail: String, value: Double?) -> some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            HStack {
                Text(title).font(TypeRole.groupLabel)
                Spacer(minLength: Space.m)
                Text(detail).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            if let value {
                ProgressView(value: value).progressViewStyle(.linear)
            } else {
                ProgressView(value: 0).progressViewStyle(.linear).opacity(0.4)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct GuideCategoryRow: View {
    let category: GuideCategoryInfo

    var body: some View {
        HStack(spacing: Space.m) {
            Text(category.id)
                .font(Font(TypeRole.rowSecondary).monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: Self.idWidth, alignment: .leading)
            Text(category.name).lineLimit(1)
            Spacer(minLength: Space.m)
            if category.accepted > 0 {
                Text("\(category.accepted)")
                    .font(Font(TypeRole.rowSecondary).monospacedDigit())
                    .foregroundStyle(.secondary)
            } else {
                Text(category.asked && !category.learned ? "ask" : "nothing yet")
                    .font(TypeRole.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(category.id) \(category.name), \(category.accepted == 0 ? "nothing yet" : "\(category.accepted) entries")")
    }

    private static let idWidth: CGFloat = 30
}

/// The sidebar footer's line while a learning run is going (spec §14.9).
struct GuideRunFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let run = model.guideProgress?.run, run.status == .running || run.status == .paused {
            VStack(spacing: Space.xs) {
                ProgressView(value: run.total == 0 ? 0 : Double(run.done) / Double(run.total))
                    .progressViewStyle(.linear)
                    .controlSize(.mini)
                    .frame(maxWidth: Self.barWidth)
                Text(run.status == .paused ? "Writing guide paused" : "Learning your writing style")
                    .font(TypeRole.caption.weight(.medium))
                Text("\(run.done.formatted()) of \(run.total.formatted()) messages")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                if let seconds = run.secondsLeft, run.status == .running {
                    Text(seconds < 60 ? "under a minute left" : "about \(Int((Double(seconds) / 60).rounded())) min left")
                        .font(TypeRole.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .lineLimit(1)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
            .contentShape(.rect)
            .onTapGesture { model.selectedMailboxID = AppModel.guideMailboxID }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .hoverHelp("Show the writing guide")
        }
    }

    private static let barWidth: CGFloat = 150
}

extension AppModel {
    /// A category chosen: the review flow gives way to it.
    func showGuideCategory(_ id: String?) {
        analysis.reviewingRules = false
        guide.selectedCategory = id
    }

    func resumeGuideRun() async {
        guard let core else { return }
        do {
            try await core.resumeGuideRun()
        } catch {
            guideProgress = try? await core.guideProgress()
        }
    }

    func cancelGuideRun() async {
        try? await core?.cancelGuideRun()
        await guide.load()
    }

    /// Save the guide as Markdown (to read or share) or JSON (to merge
    /// into another account).
    func exportGuide(json: Bool) {
        Task {
            guard let core, let text = try? await core.exportGuide(json: json) else { return }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = json ? "Writing Guide.json" : "Writing Guide.md"
            panel.allowedContentTypes = json ? [.json] : [UTType(filenameExtension: "md") ?? .plainText]
            guard panel.runModal() == .OK, let url = panel.url else { return }
            try? text.write(to: url, atomically: true, encoding: .utf8)
        }
    }
}
