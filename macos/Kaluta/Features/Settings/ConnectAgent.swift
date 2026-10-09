import AppKit
import SwiftUI

/// Connect an Agent… (spec §10.1): add an agent mailbox to Claude Code's or
/// Codex's MCP servers, through the bundled `kaluta-mcp --mailbox`. The
/// sheet shows exactly what is written and where before anything is, backs
/// the file up first, and offers the command to run instead.
struct ConnectAgentSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let accountID: String
    let address: String

    @State private var client: AgentClient = .claudeCode
    @State private var plan: AgentConnection?
    @State private var error: String?
    @State private var written: String?

    static let width: CGFloat = 600

    var body: some View {
        Dialog(title: "Connect an Agent", message: Self.message(address), width: Self.width) {
            Picker("Agent", selection: $client) {
                Text("Claude Code").tag(AgentClient.claudeCode)
                Text("Codex").tag(AgentClient.codex)
            }
            .pickerStyle(.segmented)
            .hoverHelp("Which agent on this Mac to give the mailbox to")
            .disabled(written != nil)
            if let plan {
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text(Self.whereText(plan)).font(TypeRole.meta).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(plan.entry)
                        .font(TypeRole.codeCaption)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(Space.m)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: Radius.control))
                }
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text("Or run this in Terminal instead:").font(TypeRole.meta).foregroundStyle(.secondary)
                    HStack(alignment: .top, spacing: Space.m) {
                        Text(plan.paste).font(TypeRole.codeCaption).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button("Copy") { copy(plan.paste) }
                            .hoverHelp("Copy the command, to add the mailbox yourself")
                    }
                }
            }
            if let written {
                Label(written, systemImage: "checkmark.circle").foregroundStyle(Tone.approved)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } buttons: {
            CancelButton(title: written == nil ? "Cancel" : "Done",
                         help: written == nil ? "Close without changing any file (Esc)" : "Close (Esc)") { dismiss() }
            if written == nil {
                Button(plan?.fileExists == true ? "Back Up and Add" : "Add") { write() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(plan == nil)
                    .hoverHelp("Write the entry shown into the agent's settings file, keeping a copy of the file as it was (Return)")
            }
        }
        .task(id: client) { load() }
    }

    static func message(_ address: String) -> String {
        "Let Claude Code or Codex use \(address) — its writing guide, facts and mail, and sending as it — even when Kaluta is closed. Sends follow When Agents Send; with Kaluta closed they wait until it opens. Your own accounts stay out of reach."
    }

    static func whereText(_ plan: AgentConnection) -> String {
        let file = (plan.configPath as NSString).abbreviatingWithTildeInPath
        if let old = plan.replacesOldName {
            return "Replaces the \(old) entry OpenAGC wrote in \(file) with this one, after copying the file as it is:"
        }
        switch (plan.fileExists, plan.replaces) {
        case (true, true): return "Replaces the \(plan.serverName) entry in \(file), after copying the file as it is:"
        case (true, false): return "Adds this to \(file), after copying the file as it is:"
        default: return "Creates \(file) with:"
        }
    }

    static func doneText(_ plan: AgentConnection, backup: String?) -> String {
        let name = plan.client == .claudeCode ? "Claude Code" : "Codex"
        let restart = "Start a new \(name) session to use it."
        guard let backup else { return "Added to \(name). \(restart)" }
        return "Added to \(name); the file as it was is at \((backup as NSString).abbreviatingWithTildeInPath). \(restart)"
    }

    private func load() {
        error = nil
        do {
            plan = try model.core?.agentConnection(accountID, client: client)
        } catch {
            plan = nil
            self.error = error.message
        }
    }

    private func write() {
        guard let plan else { return }
        do {
            let backup = try model.core?.connectAgent(accountID, client: client)
            written = Self.doneText(plan, backup: backup ?? nil)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}
