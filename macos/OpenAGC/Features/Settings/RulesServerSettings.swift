import SwiftUI

/// An agent mailbox's *Rules server* row in Settings › Accounts (spec
/// §10.6): *Publish to a Rules Server…*, then the status line ("Version 12,
/// published 3 minutes ago", or why the last push failed), *Publish Now*
/// and *Stop Publishing…*; while it publishes, its cloud agents.
struct RulesServerRow: View {
    @Environment(AppModel.self) private var model
    let account: AccountSummary
    @State private var status: RulesPublication?
    @State private var asking = false
    @State private var confirmingStop = false
    @State private var working = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            LabeledContent("Rules server") {
                HStack(spacing: Space.m) {
                    if working { ProgressView().controlSize(.mini) }
                    if let status, status.enabled {
                        Button("Publish Now") { Task { await publishNow() } }
                            .hoverHelp("Send the writing guide and shared facts to the server again now")
                            .disabled(working)
                        Button("Stop Publishing…") { confirmingStop = true }
                            .hoverHelp("Stop sending changes to the server, and remove the mailbox from it if you like")
                            .disabled(working)
                    } else {
                        Button("Publish to a Rules Server…") { asking = true }
                            .hoverHelp("Let cloud agents read this mailbox's writing guide and the facts you share, from a server you run")
                    }
                }
            }
            if let status {
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    Text(Self.statusLine(status, now: context.date))
                        .font(TypeRole.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
            if let message = error ?? status?.error {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.caption).foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // Agents reach what it publishes (spec §10.6).
            if let status, status.enabled {
                CloudAgentsSection(account: account)
            }
        }
        .task(id: "\(account.id) \(model.rulesRevision)") { status = model.core?.rulesPublishStatus(account.id) }
        .sheet(isPresented: $asking) {
            PublishRulesSheet(accountID: account.id, name: account.displayName ?? account.email,
                              lastServer: status?.serverUrl) { published in
                status = published
                error = nil
            }
        }
        .confirmationDialog("Stop publishing \(account.email)?", isPresented: $confirmingStop) {
            Button("Stop Publishing") { Task { await stop(remove: false) } } // no-help: confirmation dialog button
            Button("Stop and Remove from Server", role: .destructive) { // no-help: confirmation dialog button
                Task { await stop(remove: true) }
            }
        } message: {
            Text(Self.stopMessage)
        }
    }

    static let stopMessage = "Cloud agents keep reading the last version until it is removed from the server. Removing it also ends the tokens agents use there."

    /// "https://rules.example.com · Version 12, published 3 minutes ago".
    static func statusLine(_ status: RulesPublication, now: Date = .now) -> String {
        let published = status.version.flatMap { version in
            status.publishedAt.map { at in
                "Version \(version), published \(DateStyle.relative(Date(timeIntervalSince1970: TimeInterval(at) / 1000), to: now))"
            }
        }
        let state: String
        switch (status.enabled, published) {
        case (false, let published?): state = "Stopped; agents read the last version · \(published)"
        case (false, nil): state = "Stopped"
        case (true, let published?): state = status.pending ? "\(published) · a change waits to go" : published
        case (true, nil): state = status.error == nil ? "Publishing…" : "Not published yet"
        }
        let sealed = status.encrypted && status.version != nil ? " · encrypted" : ""
        return "\(status.serverUrl) · \(state)\(sealed)"
    }

    private func publishNow() async {
        guard let core = model.core else { return }
        working = true
        defer { working = false }
        do {
            status = try await core.rulesPublishNow(account.id)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func stop(remove: Bool) async {
        guard let core = model.core else { return }
        working = true
        defer { working = false }
        do {
            try await core.rulesPublishStop(account.id, removeFromServer: remove)
            status = core.rulesPublishStatus(account.id)
            error = nil
        } catch {
            self.error = error.message
        }
    }
}

/// *Publish to a Rules Server…* (spec §10.6): the server's address and,
/// before anything is sent, exactly what goes.
struct PublishRulesSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let accountID: String
    let name: String
    var lastServer: String?
    var onPublished: (RulesPublication) -> Void = { _ in }

    @State private var serverURL = ""
    @State private var registrationToken = ""
    /// Encryption at rest (spec §10.6): on unless the user turns it off,
    /// which the server must allow.
    @State private var encrypt = true
    @State private var encryption: RulesEncryption?
    @State private var preview: RulesPreview?
    @State private var error: String?
    @State private var working = false
    @FocusState private var focused: Bool

    static let width: CGFloat = 560
    static let howTo = URL(string: "https://github.com/audiojak/openagc/blob/main/docs/rules-server.md#run-it")!

    var body: some View {
        Dialog(title: "Publish to a Rules Server", message: Self.message(name), width: Self.width) {
            VStack(alignment: .leading, spacing: Space.s) {
                TextField("Server address, such as https://rules.example.com", text: $serverURL)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                    .onSubmit { if canPublish { Task { await publish() } } }
                SecureField("Registration token, if the server asks for one", text: $registrationToken)
                    .textFieldStyle(.roundedBorder)
                HStack(spacing: Space.xs) {
                    Text("You run the server yourself, behind HTTPS.").foregroundStyle(.secondary)
                    Link("How to run one", destination: Self.howTo)
                        .hoverHelp("Open the rules server's guide: running it, TLS with Caddy, backups")
                }
                .font(TypeRole.caption)
                if let note = Self.encryptionNote(encryption) {
                    Text(note).font(TypeRole.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // Only a server that leaves it to the publisher offers the choice.
                if encryption == .optional || encryption == .unsupported {
                    DisclosureGroup("Advanced") {
                        VStack(alignment: .leading, spacing: Space.xs) {
                            Toggle("Encrypt on the server", isOn: $encrypt)
                                .hoverHelp("Keep the guide, facts and agents' reports encrypted in the server's database; agents read them with their own token")
                            Text(Self.encryptCaption).font(TypeRole.caption).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .font(TypeRole.meta)
                }
            }
            if let preview {
                RulesPreviewList(preview: preview)
            } else {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text("Reading what would go…").foregroundStyle(.secondary)
                }
                .font(TypeRole.meta)
            }
            if working {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text("Publishing…").foregroundStyle(.secondary)
                }
                .font(TypeRole.meta)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } buttons: {
            CancelButton(help: "Close without publishing (Esc)") { dismiss() }
            Button("Publish") { Task { await publish() } }
                .keyboardShortcut(.defaultAction)
                .disabled(!canPublish)
                .hoverHelp("Register this mailbox on the server and send what is listed; changes follow as you make them (Return)")
        }
        .task {
            if serverURL.isEmpty, let lastServer { serverURL = lastServer }
            do throws(CoreClientError) {
                preview = try await model.core?.rulesPreview(accountID)
            } catch {
                self.error = error.message
            }
        }
        .onAppear { focused = true }
        .task(id: serverURL) { await readEncryption() }
    }

    static let encryptCaption = "Protects what the server stores, such as its backups. Whoever runs the server can still read it while answering an agent."

    /// What the sheet says about the server's encryption, if anything.
    static func encryptionNote(_ encryption: RulesEncryption?) -> String? {
        switch encryption {
        case .required: "This server keeps what you publish encrypted; agents read it with their own token."
        case .unsupported: "This server runs an older openagc-rules that cannot keep what you publish encrypted: turn encryption off under Advanced, or ask its operator to update it."
        case .optional, nil: nil
        }
    }

    /// Ask the server, a moment after the address stops changing.
    private func readEncryption() async {
        encryption = nil
        let address = serverURL.trimmingCharacters(in: .whitespaces)
        guard !address.isEmpty, let core = model.core else { return }
        try? await Task.sleep(for: .milliseconds(600))
        guard !Task.isCancelled else { return }
        let found = try? await core.rulesServerEncryption(address)
        guard !Task.isCancelled else { return }
        encryption = found
        if found == .required { encrypt = true }
    }

    static func message(_ name: String) -> String {
        "Cloud agents, such as a Claude routine or an agent on another machine, can then read \(name)'s writing guide and the facts you share, as of the last version published. Changes are published as you make them, while OpenAGC is open."
    }

    private var canPublish: Bool {
        !serverURL.trimmingCharacters(in: .whitespaces).isEmpty && preview != nil && !working
    }

    private func publish() async {
        guard let core = model.core else { return }
        working = true
        defer { working = false }
        do {
            let token = registrationToken.trimmingCharacters(in: .whitespaces)
            let status = try await core.rulesPublishStart(accountID, serverURL: serverURL,
                                                          registrationToken: token.isEmpty ? nil : token,
                                                          encrypt: encryption == .required || encrypt)
            onPublished(status)
            dismiss()
        } catch {
            self.error = error.message
        }
    }
}

/// What a push sends, listed: the sheet's "These go to the server".
struct RulesPreviewList: View {
    let preview: RulesPreview

    static let listHeight: CGFloat = 300

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text("These go to the server:").font(TypeRole.groupLabel)
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    group(Self.count(preview.entries.count, "rule or guideline", "rules and guidelines")) {
                        ForEach(Array(preview.entries.enumerated()), id: \.offset) { _, entry in
                            VStack(alignment: .leading, spacing: Space.hair) {
                                Text(entry.statement).lineLimit(2)
                                Text(Self.detail(entry)).font(TypeRole.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    group(Self.count(preview.facts.count, "fact", "facts")) {
                        ForEach(Array(preview.facts.enumerated()), id: \.offset) { _, fact in
                            Text("\(fact.category) › \(fact.label)\(fact.askBeforeUsing ? " · ask before using" : "")")
                        }
                    }
                    group(Self.count(preview.audiences.count, "audience group", "audience groups")) {
                        ForEach(Array(preview.audiences.enumerated()), id: \.offset) { _, audience in
                            Text("\(audience.name) · \(Self.count(Int(audience.members), "address", "addresses")) as hashes")
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: Self.listHeight)
            .card(.neutral)
            Text(Self.footnote(preview))
                .font(TypeRole.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private func group(_ title: String, @ViewBuilder rows: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(title).font(TypeRole.meta.weight(.semibold))
            rows()
        }
    }

    static func count(_ n: Int, _ one: String, _ many: String) -> String {
        n == 1 ? "1 \(one)" : "\(n) \(many)"
    }

    /// "Rule · for Customers · checked".
    static func detail(_ entry: RulesPreviewEntry) -> String {
        var parts = [entry.kind == .guideline ? "Guideline" : "Rule"]
        parts.append(entry.scope.isEmpty ? "always" : entry.scope)
        if entry.hasCheck { parts.append("checked") }
        return parts.joined(separator: " · ")
    }

    /// What else goes, what never does, and the facts kept back.
    static func footnote(_ preview: RulesPreview) -> String {
        let kept = preview.factsKept
        let facts = kept == 0 ? ""
            : " \(count(Int(kept), "fact stays", "facts stay")) on this Mac; turn on Share with Cloud Agents on a fact in Facts to send it."
        return "Also \(preview.address), the name it sends as and its service's limits. Never mail, quotes from your mail, or keys; addresses go only as salted hashes.\(facts)"
    }
}
