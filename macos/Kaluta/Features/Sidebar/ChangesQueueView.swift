import SwiftUI

/// The sidebar footer's line for changes that have not reached the
/// provider (spec §7.4, the queue): shown while a change is retrying or
/// has been given up, and opening the queue on a click.
struct QueueStatusFooter: View {
    @Environment(AppModel.self) private var model
    @State private var showsQueue = false

    var body: some View {
        if let line = Self.line(model.queuedChanges, service: model.pausingService) {
            Button { showsQueue.toggle() } label: {
                HStack(spacing: Space.s) {
                    Image(systemName: line.failed ? "exclamationmark.triangle.fill" : "arrow.triangle.2.circlepath")
                        .foregroundStyle(line.failed ? Tone.caution : Color.secondary)
                    Text(line.text)
                        .font(TypeRole.caption.weight(.medium))
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal, Space.l)
                .padding(.vertical, Space.m)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .hoverHelp("Show the changes waiting for \(model.pausingService)")
            .popover(isPresented: $showsQueue, arrowEdge: .trailing) {
                ChangesQueueView()
                    .environment(model)
            }
        }
    }

    /// The footer's words, or nil when nothing needs the user: a change
    /// that merely waits its turn a moment is not worth a line.
    static func line(_ changes: [QueuedChange], service: String) -> (text: String, failed: Bool)? {
        let failed = changes.count { $0.state == .failed }
        if failed > 0 {
            return ("\(failed) change\(failed == 1 ? "" : "s") didn't reach \(service)", true)
        }
        let waiting = changes.count { $0.attempts > 0 }
        guard waiting > 0 else { return nil }
        let all = changes.count
        return ("\(all) change\(all == 1 ? "" : "s") waiting for \(service)", false)
    }
}

/// The queue: each change in words, where it is, and Try Again or Dismiss
/// for those given up.
struct ChangesQueueView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let changes = model.queuedChanges
        let failed = changes.filter { $0.state == .failed }
        VStack(alignment: .leading, spacing: Space.l) {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text("Changes for \(model.pausingService)").font(TypeRole.heading)
                Text("When \(model.pausingService) can't be reached, Kaluta keeps trying for up to three days, waiting longer between tries. A change that fails is undone here.")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if changes.isEmpty {
                Text("Everything has reached \(model.pausingService).")
                    .font(TypeRole.body)
                    .foregroundStyle(.secondary)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: Space.m) {
                        ForEach(changes, id: \.id) { change in
                            QueuedChangeRow(change: change)
                            if change.id != changes.last?.id { InsetRule(inset: 0) }
                        }
                    }
                }
                .frame(maxHeight: 360)
            }
            HStack(spacing: Space.m) {
                if !failed.isEmpty {
                    Button("Dismiss All") { Task { await model.dismissQueuedChanges(nil) } }
                        .hoverHelp("Forget the changes that didn't reach \(model.pausingService); they stay undone here")
                }
                Spacer(minLength: 0)
                if changes.contains(where: { $0.state == .retrying }) || !failed.isEmpty {
                    Button(failed.isEmpty ? "Try Now" : "Try All Again") {
                        Task { await model.retryQueuedChanges(nil) }
                    }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp(failed.isEmpty ? "Try the waiting changes now rather than at their next try"
                                              : "Make the failed changes here again and send them")
                }
            }
        }
        .padding(Space.xl)
        .frame(width: 380)
        .task { await model.refreshQueuedChanges() }
    }
}

/// One change: what it does, to what, and where it is.
struct QueuedChangeRow: View {
    @Environment(AppModel.self) private var model
    let change: QueuedChange

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(change.target.isEmpty ? change.action : "\(change.action) · \(change.target)")
                .font(TypeRole.body.weight(.medium))
                .lineLimit(2)
            TimelineView(.periodic(from: .now, by: 30)) { context in
                Text(Self.status(change, service: model.pausingService, now: context.date))
                    .font(TypeRole.caption)
                    .foregroundStyle(change.state == .failed ? Tone.caution : Color.secondary)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if change.state == .failed {
                HStack(spacing: Space.m) {
                    Button("Try Again") { Task { await model.retryQueuedChanges(change.id) } }
                        .hoverHelp("Make this change here again and send it")
                    Button("Dismiss") { Task { await model.dismissQueuedChanges(change.id) } }
                        .hoverHelp("Forget this change; it stays undone here")
                }
                .controlSize(.small)
            }
        }
        .accessibilityElement(children: .contain)
    }

    /// "Trying again in 4 minutes · network error: …", "Didn't reach Gmail: …".
    static func status(_ c: QueuedChange, service: String, now: Date) -> String {
        let error = c.error.map { " · \($0)" } ?? ""
        switch c.state {
        case .failed:
            return "Didn't reach \(service) and was undone here" + (c.error.map { ": \($0)" } ?? "")
        case .sending:
            return "Sending now"
        case .retrying:
            guard let at = c.nextAttemptAt else { return "Trying again soon" + error }
            let date = Date(timeIntervalSince1970: TimeInterval(at) / 1000)
            let when = date <= now ? "soon" : DateStyle.relative(date, to: now)
            return "Tried \(c.attempts) time\(c.attempts == 1 ? "" : "s") · trying again \(when)" + error
        case .waiting:
            return c.attempts > 0 ? "Trying again now" + error : "Waiting its turn"
        }
    }
}
