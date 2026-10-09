import AppKit
import SwiftUI
import UniformTypeIdentifiers

extension AppModel {
    /// Put an agent mailbox on one of the user's own domains (spec §7.9).
    func beginAgentDomain(_ accountID: String) {
        agentMailboxSheet = .domain(accountID: accountID)
    }

    /// A subdomain of the user's own address for agents: `agents.example.com`.
    var suggestedAgentDomain: String {
        guard let email = codeAccounts.first?.email, let host = email.split(separator: "@").last,
              !Self.sharedMailHosts.contains(String(host).lowercased()) else { return "" }
        return "agents.\(host)"
    }

    /// Hosts nobody can add records to.
    static let sharedMailHosts: Set<String> = ["gmail.com", "googlemail.com", "outlook.com", "hotmail.com",
                                               "icloud.com", "me.com", "yahoo.com"]

    /// The agent's address on `domain`: its name, made an address.
    func suggestedAgentAddress(_ accountID: String, on domain: String) -> String {
        let name = accounts.first { $0.id == accountID }?.displayName ?? "agent"
        let local = name.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : "-" }.joined()
            .split(separator: "-").joined(separator: "-")
        return "\(local.isEmpty ? "agent" : local)@\(domain)"
    }

    /// One line on what a record is for.
    static func recordPurpose(_ purpose: String) -> String {
        switch purpose {
        case "inbound_mx": "Receives mail"
        case "ownership_verification": "Proves the domain is yours"
        case "spf": "Allows sending (SPF)"
        case "dkim": "Signs sent mail (DKIM)"
        case "dmarc": "Sending policy (DMARC)"
        case "tls_reporting": "Delivery reports (TLS-RPT)"
        default: purpose
        }
    }
}

/// Add a domain, create its records at the DNS host, wait until the
/// service sees them, then give the agent an address on it.
struct AgentDomainForm: View {
    @Environment(AppModel.self) private var model
    let accountID: String

    @State private var domainText = ""
    @State private var domain: AgentDomain?
    @State private var address = ""
    @State private var busy = false
    @State private var checking = false
    @State private var error: String?
    @State private var loaded = false
    @State private var lastChecked: Date?

    var body: some View {
        // Wide only for the records table.
        Dialog(title: title, message: message,
               width: domain.map { $0.verified } == false ? Self.wideWidth : DialogMetrics.width) {
            if let domain {
                if domain.verified {
                    TextField("Address", text: $address, prompt: Text("scout@\(domain.domain)"))
                        .onSubmit { Task { await useAddress() } }
                } else {
                    records(domain)
                    HStack(spacing: Space.s) {
                        if checking { ProgressView().controlSize(.small) }
                        Text(checkLine).font(TypeRole.meta).foregroundStyle(.secondary)
                    }
                }
            } else if loaded {
                TextField("Domain", text: $domainText, prompt: Text("agents.example.com"))
                    .onSubmit { Task { await add() } }
                Text("Use a subdomain: the agent receives all mail sent to it, and your own mail on the main domain is left alone.")
                    .font(TypeRole.meta).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                ProgressView().controlSize(.small)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } leading: {
            if let domain, !domain.verified {
                Button("Save Zone File…") { Task { await saveZoneFile(domain) } }
                    .hoverHelp("Save the records as a zone file, for DNS hosts that can import one")
            }
        } buttons: {
            if busy { ProgressView().controlSize(.small) }
            CancelButton(title: domain?.verified == true ? "Not Now" : "Close",
                         help: "Close; the domain keeps its place in the mailbox's settings (Esc)") {
                model.agentMailboxSheet = nil
            }
            if let domain {
                if domain.verified {
                    Button("Use This Address") { Task { await useAddress() } }
                        .keyboardShortcut(.defaultAction)
                        .disabled(busy || !address.contains("@"))
                        .hoverHelp("The agent sends and receives as this address from now on (Return)")
                } else {
                    Button("Check Now") { Task { await check() } }
                        .keyboardShortcut(.defaultAction)
                        .disabled(checking)
                        .hoverHelp("Ask Primitive to look for the records now (Return)")
                }
            } else {
                Button("Add Domain") { Task { await add() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || !domainText.contains("."))
                    .hoverHelp("Add the domain to the mailbox's Primitive account (Return)")
            }
        }
        .task { await load() }
        .task(id: domain?.id) { await keepChecking() }
    }

    static let wideWidth: CGFloat = 720

    private var title: String {
        guard let domain else { return "Use Your Own Domain" }
        return domain.verified ? "\(domain.domain) Is Ready" : "Add These Records for \(domain.domain)"
    }

    private var message: String? {
        guard let domain else {
            return "Give the agent an address on a domain you own. You add a few DNS records where the domain is managed; Kaluta checks them for you."
        }
        if domain.verified { return "Choose the agent's address on it. Mail to any address there comes to this mailbox." }
        return "Create each record at your DNS host (where the domain is managed). Changes can take a few minutes to an hour to be seen."
    }

    private var checkLine: String {
        let found = domain?.records.filter { $0.status == "found" }.count ?? 0
        let total = domain?.records.count ?? 0
        let when = lastChecked.map { " · checked \($0.formatted(date: .omitted, time: .shortened))" } ?? ""
        return "\(found) of \(total) records found\(when). This keeps checking while it is open."
    }

    @ViewBuilder private func records(_ domain: AgentDomain) -> some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: Space.xl, verticalSpacing: Space.xs) {
            GridRow {
                Text("Type"); Text("Name"); Text("Value"); Text("")
            }
            .font(TypeRole.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            ForEach(Array(domain.records.enumerated()), id: \.offset) { _, record in
                GridRow {
                    Text(record.kind + (record.priority.map { " \($0)" } ?? ""))
                        .font(TypeRole.meta.monospaced())
                    VStack(alignment: .leading, spacing: Space.hair) {
                        Text(record.fqdn).font(TypeRole.meta.monospaced()).textSelection(.enabled)
                        Text(AppModel.recordPurpose(record.purpose)).font(TypeRole.caption).foregroundStyle(.secondary)
                    }
                    Text(record.value)
                        .font(TypeRole.meta.monospaced())
                        .lineLimit(2)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                        .hoverHelp(record.message ?? record.value)
                    HStack(spacing: Space.s) {
                        statusIcon(record.status)
                        Button("Copy") { copy(record.value) }
                            .controlSize(.small)
                            .hoverHelp("Copy the record's value")
                    }
                }
            }
        }
    }

    private func statusIcon(_ status: String) -> some View {
        let (symbol, tone, label): (String, Color, String) = switch status {
        case "found": ("checkmark.circle.fill", .secondary, "Found")
        case "incorrect": ("exclamationmark.triangle.fill", Tone.failure, "Found, but not right")
        default: ("circle.dashed", .secondary, "Not found yet")
        }
        return Image(systemName: symbol).foregroundStyle(tone).hoverHelp(label).accessibilityLabel(label)
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func load() async {
        guard let core = model.core else { return }
        if domainText.isEmpty { domainText = model.suggestedAgentDomain }
        let existing = (try? await core.agentDomains(accountID)) ?? []
        // Pick up where the user left off: the newest domain not in use yet.
        let current = model.accounts.first { $0.id == accountID }?.email.split(separator: "@").last.map(String.init)
        if let pending = existing.last(where: { $0.domain != current }) {
            show(pending)
        }
        loaded = true
    }

    private func show(_ found: AgentDomain) {
        domain = found
        if found.verified, address.isEmpty { address = model.suggestedAgentAddress(accountID, on: found.domain) }
    }

    private func add() async {
        guard let core = model.core, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            show(try await core.addAgentDomain(accountID, domain: domainText))
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func check() async {
        guard let core = model.core, let current = domain, !checking else { return }
        checking = true
        defer { checking = false }
        do {
            show(try await core.checkAgentDomain(accountID, domainID: current.id))
            lastChecked = Date()
            error = nil
        } catch {
            self.error = error.message
        }
    }

    /// While the sheet is open and the domain not ready, check every 20 s.
    private func keepChecking() async {
        while !Task.isCancelled, let current = domain, !current.verified {
            try? await Task.sleep(for: .seconds(20))
            if Task.isCancelled { return }
            await check()
        }
    }

    private func saveZoneFile(_ domain: AgentDomain) async {
        guard let core = model.core else { return }
        do {
            let text = try await core.agentDomainZoneFile(accountID, domainID: domain.id)
            let panel = NSSavePanel()
            panel.nameFieldStringValue = "\(domain.domain).zone"
            panel.allowedContentTypes = [.plainText]
            guard panel.runModal() == .OK, let url = panel.url else { return }
            try text.write(to: url, atomically: true, encoding: .utf8)
        } catch let error as CoreClientError {
            self.error = error.message
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func useAddress() async {
        guard let core = model.core, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            try await core.setAgentAddress(accountID, address)
            await model.reloadAccounts()
            model.agentMailboxSheet = nil
        } catch {
            self.error = error.message
        }
    }
}
