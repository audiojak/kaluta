import SwiftUI

/// The Sync Debugger (Window menu; docs/plans/imap-first-sync.md, decision
/// 4): per account, which transport serves each sync job and why, the IMAP
/// breaker, capabilities and bandwidth, recent operations with timings, and
/// an on-demand comparison of IMAP against the Gmail API. It reads and
/// times only; it changes no mail.
struct SyncDebuggerView: View {
    @Environment(AppModel.self) private var model
    @State private var accountID: String?
    @State private var diagnostics: SyncDiagnostics?
    @State private var comparison: [TransportComparison] = []
    @State private var comparing = false
    @State private var comparisonError: String?

    private var gmailAccounts: [AccountSummary] { model.accounts.filter { $0.kind == .gmail } }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                header
                if let diagnostics {
                    TransportSection(diagnostics: diagnostics)
                    JobsSection(ops: diagnostics.latestByJob)
                    comparisonSection
                    RecentSection(ops: diagnostics.recent)
                } else {
                    ContentUnavailableView("No Gmail Account", systemImage: "arrow.triangle.2.circlepath",
                                           description: Text("Sync details appear here for Gmail accounts."))
                }
            }
            .padding(Space.xxxl)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minWidth: 760, minHeight: 560)
        .task(id: accountID) { await poll() }
        .onAppear { if accountID == nil { accountID = model.openAccountID ?? gmailAccounts.first?.id } }
    }

    private var header: some View {
        HStack(spacing: Space.m) {
            Picker("Account", selection: $accountID) {
                ForEach(gmailAccounts, id: \.id) { account in
                    Text(account.email).tag(Optional(account.id))
                }
            }
            .fixedSize()
            .hoverHelp("The account whose sync to show")
            Spacer()
            Button("Refresh") { Task { await load() } }
                .hoverHelp("Read the account's sync details again")
        }
    }

    private var comparisonSection: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            HStack(spacing: Space.m) {
                Text("IMAP and the Gmail API compared").font(TypeRole.groupLabel)
                Spacer()
                if comparing { ProgressView().controlSize(.small) }
                Button("Run Comparison") { Task { await compare() } }
                    .disabled(comparing || diagnostics?.syncing != true)
                    .hoverHelp("Time listing, headers, bodies and changes both ways; downloads a sample and changes no mail")
            }
            Text("Lists up to 10,000 ids, fetches 500 headers and 500 messages, and reads changes, over IMAP and over the API. It downloads but stores nothing.")
                .font(TypeRole.caption).foregroundStyle(.secondary)
            if let comparisonError {
                Text(comparisonError).font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
            if !comparison.isEmpty {
                Grid(alignment: .leading, horizontalSpacing: Space.xl, verticalSpacing: Space.xs) {
                    GridRow {
                        Text("Job"); Text("IMAP"); Text("Gmail API"); Text("Faster")
                    }
                    .font(TypeRole.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(SyncDebugger.pairs(comparison), id: \.job) { pair in
                        GridRow {
                            Text(pair.job.capitalized)
                            Text(SyncDebugger.measured(pair.imap)).hoverHelp(pair.imap?.error ?? pair.imap?.note ?? "")
                            Text(SyncDebugger.measured(pair.api)).hoverHelp(pair.api?.error ?? pair.api?.note ?? "")
                            Text(SyncDebugger.faster(pair)).foregroundStyle(.secondary)
                        }
                        .font(TypeRole.meta)
                    }
                }
                ForEach(SyncDebugger.notes(comparison), id: \.self) { note in
                    Text(note).font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private func poll() async {
        comparison = []
        comparisonError = nil
        while !Task.isCancelled {
            await load()
            try? await Task.sleep(for: .seconds(2))
        }
    }

    private func load() async {
        guard let core = model.core, let accountID else { diagnostics = nil; return }
        diagnostics = await core.syncDiagnostics(accountID)
    }

    private func compare() async {
        guard let core = model.core, let accountID else { return }
        comparing = true
        comparisonError = nil
        defer { comparing = false }
        do {
            comparison = try await core.compareTransports(accountID)
        } catch let error as CoreClientError {
            comparisonError = error.message
        } catch {
            comparisonError = error.localizedDescription
        }
    }
}

/// Formatting for the Sync Debugger, kept apart for tests.
enum SyncDebugger {
    struct Pair: Equatable {
        let job: String
        let imap: TransportComparison?
        let api: TransportComparison?
    }

    /// One row per job, in the order the comparison ran.
    static func pairs(_ rows: [TransportComparison]) -> [Pair] {
        var order: [String] = []
        for row in rows where !order.contains(row.job) { order.append(row.job) }
        return order.map { job in
            Pair(job: job, imap: rows.first { $0.job == job && $0.via == "imap" },
                 api: rows.first { $0.job == job && $0.via == "api" })
        }
    }

    static func duration(_ millis: UInt64) -> String {
        millis < 1000 ? "\(millis) ms" : String(format: "%.1f s", Double(millis) / 1000)
    }

    /// "1.2 s · 500", or why it was not measured.
    static func measured(_ row: TransportComparison?) -> String {
        guard let row else { return "—" }
        if let error = row.error { return error == "not measured" ? "—" : "Failed" }
        return "\(duration(row.millis)) · \(row.items.formatted())"
    }

    /// Which way was faster per item, when both were measured.
    static func faster(_ pair: Pair) -> String {
        guard let imap = pair.imap, let api = pair.api, imap.error == nil, api.error == nil,
              imap.items > 0, api.items > 0 else { return "" }
        let imapPer = Double(imap.millis) / Double(imap.items)
        let apiPer = Double(api.millis) / Double(api.items)
        if imapPer == apiPer { return "Same" }
        let (winner, ratio) = imapPer < apiPer ? ("IMAP", apiPer / max(imapPer, 0.001)) : ("API", imapPer / max(apiPer, 0.001))
        return ratio >= 10 ? "\(winner), ×\(Int(ratio))" : String(format: "\(winner), ×%.1f", ratio)
    }

    /// The notes that say what each measurement was.
    static func notes(_ rows: [TransportComparison]) -> [String] {
        var seen: [String] = []
        for row in rows {
            guard let note = row.note else { continue }
            let text = "\(row.job.capitalized): \(note)"
            if !seen.contains(text) { seen.append(text) }
        }
        return seen
    }

    /// The breaker in words.
    static func breaker(_ d: SyncDiagnostics, now: Date = Date()) -> String {
        if let until = d.breakerOpenUntil {
            let date = Date(timeIntervalSince1970: TimeInterval(until) / 1000)
            if date > now {
                return "IMAP paused until \(date.formatted(date: .omitted, time: .shortened)) after \(d.consecutiveImapFailures) failures"
            }
        }
        return d.consecutiveImapFailures == 0 ? "Closed" : "Closed, \(d.consecutiveImapFailures) failures in a row"
    }

    static func bytes(_ n: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: n), countStyle: .file)
    }

    static func time(_ at: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(at) / 1000).formatted(date: .omitted, time: .standard)
    }
}

private struct TransportSection: View {
    let diagnostics: SyncDiagnostics

    var body: some View {
        let d = diagnostics
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Transport").font(TypeRole.groupLabel)
            Grid(alignment: .leading, horizontalSpacing: Space.xl, verticalSpacing: Space.xs) {
                row("Syncing", d.syncing ? "Yes" : "No")
                row("Downloads", d.backfill.transport)
                row("IMAP breaker", SyncDebugger.breaker(d))
                if let error = d.lastImapError { row("Last IMAP error", error) }
                row("IMAP today", d.imapBudgetBytes > 0
                    ? "\(SyncDebugger.bytes(d.backfill.imapBytesToday)) of \(SyncDebugger.bytes(d.imapBudgetBytes))"
                    : SyncDebugger.bytes(d.backfill.imapBytesToday))
                row("Messages stored", d.backfill.storedMessages.formatted())
                row("Capabilities", d.imapCapabilities.isEmpty ? "Not logged in yet" : d.imapCapabilities.joined(separator: " "))
            }
            .font(TypeRole.meta)
            .textSelection(.enabled)
        }
    }

    private func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct JobsSection: View {
    let ops: [TransportOp]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Each job, last time it ran").font(TypeRole.groupLabel)
            if ops.isEmpty {
                Text("Nothing has run yet.").font(TypeRole.meta).foregroundStyle(.secondary)
            } else {
                OpsGrid(ops: ops)
            }
        }
    }
}

private struct RecentSection: View {
    let ops: [TransportOp]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Recent operations").font(TypeRole.groupLabel)
            if ops.isEmpty {
                Text("Nothing has run yet.").font(TypeRole.meta).foregroundStyle(.secondary)
            } else {
                OpsGrid(ops: ops)
            }
        }
    }
}

private struct OpsGrid: View {
    let ops: [TransportOp]

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: Space.xl, verticalSpacing: Space.xs) {
            GridRow {
                Text("When"); Text("Job"); Text("Via"); Text("Time"); Text("Items"); Text("Why the API")
            }
            .font(TypeRole.caption.weight(.semibold)).foregroundStyle(.secondary)
            ForEach(Array(ops.enumerated()), id: \.offset) { _, op in
                GridRow {
                    Text(SyncDebugger.time(op.at)).monospacedDigit()
                    Text(op.job)
                    Text(op.via == "imap" ? "IMAP" : "API")
                    Text(SyncDebugger.duration(op.millis)).monospacedDigit()
                    Text(op.items.formatted()).monospacedDigit()
                    Text(op.ok ? (op.reason ?? "") : "Failed: \(op.reason ?? "")")
                        .foregroundStyle(op.ok ? Color.secondary : Tone.failure)
                        .lineLimit(2)
                }
                .font(TypeRole.meta)
            }
        }
        .textSelection(.enabled)
    }
}
