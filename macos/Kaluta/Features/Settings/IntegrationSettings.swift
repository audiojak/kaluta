import AppKit
import SwiftUI

/// Settings › Integrations: other apps Kaluta can read from (spec §14.14).
/// Granola, optional: connected, Kaluta reads recent meetings, their notes
/// and transcripts (never writing to Granola) and offers follow-up emails
/// in the Follow-ups workflow.
struct IntegrationSettings: View {
    @Environment(AppModel.self) private var model
    @State private var status: GranolaStatus?
    @State private var working = false
    @State private var problem: String?

    var body: some View {
        Form {
            Section {
                LabeledContent("Granola") {
                    if status?.connected == true {
                        HStack(spacing: Space.m) {
                            Text(status?.email.map { "Connected as \($0)" } ?? "Connected")
                                .foregroundStyle(.secondary)
                            Button("Disconnect") { Task { await disconnect() } }
                                .hoverHelp("Stop reading Granola; the meetings and follow-ups Kaluta made from it go")
                        }
                    } else {
                        HStack(spacing: Space.m) {
                            if working { ProgressView().controlSize(.small) }
                            Button("Connect Granola…") { Task { await connect() } }
                                .disabled(working)
                                .hoverHelp("Sign in to Granola in your browser; Kaluta only reads your meetings")
                        }
                    }
                }
                if let problem {
                    Text(problem).foregroundStyle(Tone.failure).font(TypeRole.caption)
                }
            } header: {
                Text("Meeting notes")
            } footer: {
                Text("With Granola connected, Kaluta reads your meetings from the last two weeks, their notes and transcripts, and asks your agent which deserve a follow-up email. Those wait in Follow-ups on the Dashboard with drafts to edit and send. Nothing is sent or written to Granola on its own.")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .task { load() }
    }

    private func load() {
        status = try? model.core?.granolaStatus()
    }

    private func connect() async {
        working = true
        problem = nil
        defer { working = false }
        problem = await model.connectGranola()
        load()
    }

    private func disconnect() async {
        do {
            try await model.core?.disconnectGranola()
        } catch {
            problem = error.message
        }
        load()
        await model.refreshWorkflowOverview()
    }
}

extension AppModel {
    /// Connect Granola (spec §14.14): its sign-in opens in the browser. A
    /// problem to show, or nil.
    func connectGranola() async -> String? {
        guard let core else { return nil }
        do {
            let start = try await core.beginGranolaConnect()
            NSWorkspace.shared.open(start.authorizationURL)
            _ = try await core.completeGranolaConnect(start.sessionID)
            await refreshWorkflowOverview()
            return nil
        } catch {
            return error.kind == .cancelled ? nil : error.message
        }
    }
}
