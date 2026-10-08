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

/// One message: what the AI drafted beside what the user sent, the
/// user's edits marked (Review mode, spec §14.10).
struct AnalysisPairCard: View {
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
