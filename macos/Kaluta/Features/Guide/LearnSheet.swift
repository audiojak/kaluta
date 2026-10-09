import SwiftUI

/// Learn from Sent Mail… (spec §14.9): how many of the latest sent
/// messages to analyse (default 1,000), who to leave out, what that means,
/// and whose agent does it. The run then goes on in the background.
struct LearnSheet: View {
    @Environment(AppModel.self) private var model
    @State private var count = 1_000
    @State private var excluded = ""
    @State private var info: GuideSampleInfo?
    /// Which mail: the latest (first time), or after that newer, older or a
    /// re-check (further analysis).
    @State private var kind: GuideRunKind = .latest
    @State private var chosen: UInt32 = 0
    @State private var error: String?
    @State private var starting = false

    var body: some View {
        Dialog(title: (info?.analysedBefore ?? 0) > 0 ? "Learn More from Sent Mail" : "Learn from Sent Mail",
               message: "\(model.agent.providerName) reads your sent messages in batches and proposes rules and guidelines. You decide on them when it has finished.") {
            if let why = notReady {
                VStack(alignment: .leading, spacing: Space.s) {
                    Label(why, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(Tone.caution)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("Agent Settings…") { model.guideSheet = nil; model.openAgentSettings?() }
                        .hoverHelp("Connect Claude Code or Codex")
                }
                .font(TypeRole.meta)
            }
            if (info?.analysedBefore ?? 0) > 0 {
                Picker("Analyse", selection: $kind) {
                    Text("Mail sent since last time").tag(GuideRunKind.newer)
                    Text("Older mail, further back").tag(GuideRunKind.older)
                    Text("A fresh sample, to re-check the guide").tag(GuideRunKind.recheck)
                }
                .pickerStyle(.radioGroup)
                .hoverHelp("Which of your sent mail to analyse this time")
            }
            LabeledContent("Messages") {
                HStack(spacing: Space.s) {
                    TextField("1000", value: $count, format: .number)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: Self.countWidth)
                        .multilineTextAlignment(.trailing)
                    Text("of the latest you sent").foregroundStyle(.secondary)
                }
            }
            TextField("Leave out (addresses or @domains, optional)", text: $excluded)
                .textFieldStyle(.roundedBorder)
            if let info {
                Text(summary(info)).font(TypeRole.meta).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text("Your sent mail goes to \(model.agent.providerName) on this Mac, as when you ask it about mail. The guide stays in this account.")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without starting (Esc)") { model.guideSheet = nil }
            Button("Start") { Task { await start() } }
                .keyboardShortcut(.defaultAction)
                .disabled(notReady != nil || starting || chosen == 0)
                .hoverHelp("Start analysing in the background (Return)")
        }
        .task(id: "\(count)|\(excluded)|\(kind)") {
            try? await Task.sleep(for: .milliseconds(250))
            await refresh()
        }
        .task { if model.agent.providers.isEmpty { await model.agent.loadProviders() } }
    }

    /// Why the agent cannot do it, or nil when it can.
    private var notReady: String? {
        if model.agent.isProviderReady { return nil }
        let status = model.agent.provider.map { AgentStatusText($0).detail } ?? "No agent found."
        return "\(model.agent.providerName) is not ready: \(status) Connect Claude Code or Codex in Settings › Agents to learn from your mail."
    }

    private var filter: GuideSampleFilter {
        GuideSampleFilter(excludePeople: excluded.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
                              .filter { !$0.isEmpty },
                          excludeLabels: [])
    }

    private func refresh() async {
        guard count > 0, let core = model.core else { info = nil; return }
        info = try? await core.guideSampleInfo(count: UInt32(count), filter: filter)
        // Once some mail is analysed, the first-time choice becomes "newer".
        if kind == .latest, (info?.analysedBefore ?? 0) > 0 { kind = .newer }
        chosen = (try? await core.guideRunPreview(request)) ?? 0
    }

    private var request: GuideRunRequest {
        GuideRunRequest(kind: kind, count: UInt32(max(count, 1)), filter: filter, focus: nil, agent: model.agent.providerID)
    }

    private func summary(_ info: GuideSampleInfo) -> String {
        if chosen == 0 {
            if info.sent == 0 { return "This account has no sent mail yet." }
            return kind == .newer ? "Nothing sent since the last analysis." : "Every sent message has been analysed already."
        }
        let batches = (chosen + 19) / 20
        var s = "\(chosen.formatted()) messages in \(batches.formatted()) batches of 20"
        s += " (you have sent \(info.sent.formatted())"
        s += info.analysedBefore > 0 ? "; \(info.analysedBefore.formatted()) analysed before)." : ")."
        return s
    }

    private func start() async {
        guard let core = model.core else { return }
        starting = true
        defer { starting = false }
        do {
            let run = try await core.startGuideRun(request)
            model.guideProgress = GuideProgress(run: run, decisionsTotal: model.guideProgress?.decisionsTotal ?? 0,
                                                decisionsDone: model.guideProgress?.decisionsDone ?? 0)
            model.guideSheet = nil
        } catch let e as CoreClientError {
            error = e.message
        } catch {
            self.error = error.localizedDescription
        }
    }

    private static let countWidth: CGFloat = 90
}

extension AppModel {
    /// Improve one category from the sent messages most likely to show it
    /// (further analysis, spec §14.9).
    func improveGuide(_ category: String) async {
        guard let core else { return }
        do {
            _ = try await core.startGuideRun(GuideRunRequest(kind: .improve, count: 100,
                                                             filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                             focus: category, agent: agent.providerID))
            guideProgress = try? await core.guideProgress()
        } catch {
            guideError = error.message
        }
    }
}
