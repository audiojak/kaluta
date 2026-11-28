import AppKit
import SwiftUI

/// Settings › Agents (spec §9.2): which agent CLIs are installed, signed in
/// and new enough. Detection runs when this opens (cached per launch) and
/// again on Check Again.
struct AgentSettings: View {
    @Environment(AppModel.self) private var model
    @State private var providers: [AgentProviderInfo] = []
    @State private var checking = false
    /// A fix was started in Terminal; the footer says what happens next.
    @State private var waitingOnTerminal = false

    var body: some View {
        Form {
            Section {
                Picker("Default agent", selection: Binding(get: { model.agent.providerID },
                                                           set: { model.agent.providerID = $0 })) {
                    ForEach(providers, id: \.id) { provider in
                        Text(AgentStatusText(provider).isReady ? provider.name : "\(provider.name) (not ready)")
                            .tag(provider.id)
                    }
                }
                .disabled(providers.isEmpty)
                .hoverHelp("The agent that answers in the agent column, writing help and task suggestions")
            } footer: {
                Text("Used for new conversations, writing help and task suggestions. The agent column's ✦ menu switches it too.")
            }
            Section {
                if providers.isEmpty {
                    HStack {
                        ProgressView().controlSize(.small)
                        Text("Looking for Claude Code and Codex…").foregroundStyle(.secondary)
                    }
                }
                ForEach(providers, id: \.id) { provider in
                    AgentRow(provider: provider, isDefault: provider.id == model.agent.providerID) {
                        waitingOnTerminal = true
                    }
                }
            } header: {
                Text("Agents")
            } footer: {
                Text(waitingOnTerminal
                     ? "Finish in Terminal, then come back: Kaluta checks again when you return."
                     : "Kaluta uses the Claude Code or Codex command-line tool you already have, signed in with your own account. It never reads their credentials.")
                    .foregroundStyle(.secondary)
            }
            // The model Kaluta asks for, where the agent lists its account's.
            if let codex = providers.first(where: { $0.id == "codex" }), AgentStatusText(codex).isReady {
                AgentModelSection(provider: codex)
            }
            Section {
                Button(checking ? "Checking…" : "Check Again") { Task { await load(refresh: true) } }
                    .hoverHelp("Look again for the Claude Code and Codex command-line tools")
                    .disabled(checking)
            }
            Section {
                Button("Clear Suggestions History") { model.clearSuggestionHistory() }
                    .hoverHelp("Forget the prompts you've sent, which suggestions offer first")
            } footer: {
                Text("Suggestions over the prompt start with things you have asked before, remembered on this Mac for each account.")
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .task { await load(refresh: false) }
        // Back from signing in or installing in Terminal: look again.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            guard !checking, providers.contains(where: { !AgentStatusText($0).isReady }) else { return }
            Task { await load(refresh: true) }
        }
    }

    private func load(refresh: Bool) async {
        guard let core = model.core else { return }
        checking = true
        providers = await core.agentProviders(refresh: refresh)
        checking = false
        if providers.allSatisfy({ AgentStatusText($0).isReady }) { waitingOnTerminal = false }
    }
}

private struct AgentRow: View {
    let provider: AgentProviderInfo
    let isDefault: Bool
    /// The fix was started in Terminal.
    var started: () -> Void = {}
    @State private var failed: String?

    var body: some View {
        let status = AgentStatusText(provider)
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Image(systemName: status.symbol)
                .foregroundStyle(status.isReady ? .green : .secondary)
            VStack(alignment: .leading, spacing: Space.hair) {
                HStack(spacing: Space.s) {
                    Text(provider.name).font(TypeRole.heading)
                    if isDefault {
                        Text("Default")
                            .font(Font(TypeRole.chip))
                            .padding(.horizontal, Space.xs)
                            .padding(.vertical, Space.hair)
                            .background(Tone.highlight, in: .rect(cornerRadius: Radius.chip))
                            .accessibilityLabel("the default agent")
                    }
                }
                Text(status.detail).font(TypeRole.meta).foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                if let fix = status.fix {
                    HStack(spacing: Space.m) {
                        Text(fix.command).font(TypeRole.codeCaption).textSelection(.enabled)
                            .padding(.horizontal, Space.s)
                            .padding(.vertical, Space.xs)
                            .background(.quaternary, in: RoundedRectangle(cornerRadius: Radius.control))
                        Button(fix.title) {
                            failed = fix.runInTerminal() ? nil : "Terminal could not be opened. Copy the command and run it yourself."
                            if failed == nil { started() }
                        }
                        .hoverHelp("Open Terminal and run this command for you")
                        Button("Copy") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(fix.command, forType: .string)
                        }
                        .hoverHelp("Copy the command, to run it yourself")
                    }
                    .controlSize(.small)
                    .padding(.top, Space.xs)
                }
                if let failed {
                    Label(failed, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
                }
            }
        }
    }
}

/// The command that puts an agent right, and the button that runs it.
struct AgentFix: Equatable {
    /// "Sign In…", "Install…" or "Update…".
    let title: String
    let command: String

    /// Run the command in a new Terminal window, in a login shell so the
    /// user's own PATH finds the tool. False when Terminal did not open.
    @MainActor func runInTerminal() -> Bool {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("Kaluta Agent Setup.command")
        let script = "#!/bin/zsh -l\n\(command)\necho\necho \"Finished. Go back to Kaluta: it checks again when you return.\"\n"
        do {
            try script.write(to: file, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: file.path)
        } catch {
            return false
        }
        return NSWorkspace.shared.open(file)
    }
}

/// Which model Kaluta asks an agent for: its own setting (what it uses in
/// Terminal) or one its account offers. When the one asked for was
/// refused this launch, says what was used instead.
struct AgentModelSection: View {
    @Environment(AppModel.self) private var model
    let provider: AgentProviderInfo
    @State private var models: [AgentModelInfo] = []
    @State private var loading = true
    @State private var chosen = ""
    @State private var fallback: AgentModelFallback?

    var body: some View {
        Section {
            Picker("Model", selection: $chosen) {
                Text("\(provider.name)'s own setting").tag("")
                ForEach(models, id: \.id) { m in
                    Text(m.isDefault ? "\(m.name) (your account's default)" : m.name).tag(m.id)
                }
                // One no longer offered stays shown, so the choice never goes blank.
                if !chosen.isEmpty && !models.contains(where: { $0.id == chosen }) {
                    Text(chosen).tag(chosen)
                }
            }
            .disabled(loading)
            .hoverHelp("The model Kaluta asks \(provider.name) for")
            .onChange(of: chosen) { _, new in
                guard !loading else { return }
                model.setAgentModel(new.isEmpty ? nil : new, for: provider.id)
                fallback = nil
            }
        } header: {
            Text("\(provider.name) model")
        } footer: {
            VStack(alignment: .leading, spacing: Space.xs) {
                if let fallback {
                    Label(Self.fallbackNote(fallback, agent: provider.name), systemImage: "exclamationmark.triangle")
                }
                Text("\(provider.name)'s own setting is the model it uses in Terminal. If the model asked for isn't available to your account, Kaluta uses your account's default instead.")
            }
            .foregroundStyle(.secondary)
        }
        .task {
            chosen = model.agentModel(for: provider.id) ?? ""
            fallback = model.core?.agentModelFallback(provider: provider.id)
            models = await model.core?.agentModels(provider: provider.id) ?? []
            loading = false
        }
    }

    /// "Codex's own setting, gpt-6.1-sol, isn't available to your account,
    /// so Kaluta is using gpt-5.5 instead."
    static func fallbackNote(_ f: AgentModelFallback, agent: String) -> String {
        f.requested == nil
            ? "\(agent)'s own setting, \(f.refused), isn't available to your account, so Kaluta is using \(f.used) instead."
            : "\(f.refused) isn't available to your account, so Kaluta is using \(f.used) instead."
    }
}

/// How an agent's status reads in the UI. Pure, so it is unit-tested.
struct AgentStatusText: Equatable {
    let symbol: String
    let detail: String
    let isReady: Bool
    /// What puts it right, when a command does.
    let fix: AgentFix?

    init(_ provider: AgentProviderInfo) {
        let codex = provider.id == "codex"
        let signIn = AgentFix(title: "Sign In…", command: codex ? "codex login" : "claude auth login")
        let package = codex ? "codex" : "--cask claude-code"
        switch provider.status {
        case let .ready(version):
            self.init("checkmark.circle.fill", "Ready — version \(version)", true, nil)
        case .notInstalled:
            self.init("arrow.down.circle", "Not installed. Install with: brew install \(package)", false,
                      AgentFix(title: "Install…", command: "brew install \(package)"))
        case let .notAuthenticated(version):
            self.init("person.crop.circle.badge.exclamationmark",
                      "Version \(version) is installed but not signed in. Sign in with: \(signIn.command)", false, signIn)
        case let .updateRequired(version, minimum):
            self.init("exclamationmark.arrow.circlepath",
                      "Version \(version) is too old; Kaluta needs \(minimum) or later.", false,
                      AgentFix(title: "Update…", command: "brew upgrade \(package)"))
        case let .error(message) where Self.isSignInFailure(message):
            self.init("person.crop.circle.badge.exclamationmark",
                      "Its sign-in has expired. Sign in again with: \(signIn.command)", false, signIn)
        case let .error(message):
            self.init("exclamationmark.triangle", "Couldn’t check: \(message)", false, nil)
        }
    }

    /// A check that failed because the tool's own sign-in lapsed, as the
    /// tools word it ("Failed to authenticate: OAuth session expired…").
    static func isSignInFailure(_ message: String) -> Bool {
        let m = message.lowercased()
        return ["authenticat", "oauth", "session expired", "token expired", "not logged in", "unauthorized", "401"]
            .contains { m.contains($0) }
    }

    private init(_ symbol: String, _ detail: String, _ isReady: Bool, _ fix: AgentFix?) {
        self.symbol = symbol
        self.detail = detail
        self.isReady = isReady
        self.fix = fix
    }
}
