import SwiftUI

/// Analysis settings (spec §14.10): the daily review, where facts are
/// learned from, the daily cap, how long AI drafts are kept, and the
/// notification. In Settings › Analysis and from the Analysis header.
struct AnalysisSettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(NewMailNotifier.analysisKey, store: CoreClient.appDefaults()) private var notify = false
    @State private var settings: AnalysisSettings?
    @State private var error: String?

    var body: some View {
        Form {
            if let settings {
                Section {
                    Toggle("Review AI drafts each day", isOn: bind(\.dailyReview, settings))
                        .hoverHelp("Once a day, compare what AI drafted with what you sent, and propose changes to your writing guide")
                    Picker("Learn facts from", selection: bind(\.factsFrom, settings)) {
                        Text("Off").tag(FactsFrom.off)
                        Text("Mail written with AI").tag(FactsFrom.mailWrittenWithAi)
                        Text("All mail I send").tag(FactsFrom.allMailISend)
                    }
                    .pickerStyle(.segmented)
                    .hoverHelp("Mail you receive is never read for facts")
                    Picker("Drafts compared a day", selection: bind(\.pairsPerDay, settings)) {
                        ForEach([10, 25, 50, 100, 200] as [UInt32], id: \.self) { n in Text("\(n)").tag(n) }
                    }
                    .hoverHelp("At most this many a day; the rest wait for the next day")
                    Picker("Keep AI drafts for", selection: bind(\.keepDays, settings)) {
                        ForEach([7, 30, 90] as [UInt32], id: \.self) { d in Text("\(d) days").tag(d) }
                    }
                    .hoverHelp("After this long, only how much each draft changed is kept")
                } footer: {
                    Text("Reviews send the AI drafts you edited, and what you sent, to your own agent (Claude Code or Codex); with All mail I send, also the mail you sent that day. Nothing else leaves your Mac.")
                        .font(TypeRole.caption)
                        .foregroundStyle(.secondary)
                }
            } else if model.core != nil, model.openAccountID != nil {
                ProgressView().controlSize(.small)
            } else {
                Text("Open an account to change its Analysis settings.").foregroundStyle(.secondary)
            }
            Section {
                Toggle("Notify me of new proposals", isOn: $notify)
                    .hoverHelp("Once a day, when a review finds something and OpenAGC is not in front")
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        }
        .formStyle(.grouped)
        .task(id: model.openAccountID) {
            // Nothing from the last account is written into this one.
            settings = nil
            settings = try? await model.core?.analysisSettings()
        }
    }

    /// A binding that saves the whole settings when one changes.
    private func bind<T>(_ key: WritableKeyPath<AnalysisSettings, T>, _ current: AnalysisSettings) -> Binding<T> {
        Binding(get: { settings?[keyPath: key] ?? current[keyPath: key] }, set: { value in
            var next = settings ?? current
            next[keyPath: key] = value
            settings = next
            Task {
                guard let core = model.core else { return }
                do throws(CoreClientError) {
                    try await core.setAnalysisSettings(next)
                    model.analysisDaily = next.dailyReview
                    error = nil
                } catch {
                    self.error = error.message
                }
            }
        })
    }
}
