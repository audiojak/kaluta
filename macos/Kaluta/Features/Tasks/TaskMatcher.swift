import Observation
import SwiftUI

/// Looks for an open task the email belongs to while a task is being made
/// for it (spec §14.8): it never holds the user up, and once the task is
/// made (or the form is closed) whatever it finds is let go.
@MainActor
@Observable
final class TaskMatcher {
    enum State: Equatable {
        case idle, searching, found([TaskMatch])
    }

    private(set) var state: State = .idle
    @ObservationIgnored private var generation = 0

    var matches: [TaskMatch] {
        if case let .found(found) = state { return found }
        return []
    }

    func run(threadID: String, core: CoreClient) async {
        generation += 1
        let mine = generation
        state = .searching
        let found = (try? await core.findTaskMatches(threadID: threadID)) ?? []
        guard mine == generation else { return }
        state = .found(found)
    }

    /// The task was made, or the form put away: anything found is ignored.
    func stop() {
        generation += 1
        state = .idle
    }
}

/// Under a task form: a progress bar while Kaluta looks for a task the
/// email may belong to, then up to three, each added to with a click or
/// ⌘1–⌘3. Making a new task stays one key away throughout.
struct TaskMatchesView: View {
    let matcher: TaskMatcher
    var large = false
    let add: (TaskMatch) -> Void

    var body: some View {
        switch matcher.state {
        case .idle:
            EmptyView()
        case .searching:
            VStack(alignment: .leading, spacing: Space.xs) {
                ProgressView()
                    .progressViewStyle(.linear)
                    .controlSize(.small)
                    .frame(maxWidth: 240)
                Text("Looking for a task this email belongs to…")
                    .font(large ? TypeRole.workflowContext : TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
        case let .found(matches) where matches.isEmpty:
            Text("No open task looks like this one.")
                .font(large ? TypeRole.workflowContext : TypeRole.caption)
                .foregroundStyle(.secondary)
        case let .found(matches):
            VStack(alignment: .leading, spacing: Space.s) {
                Text(matches.count == 1 ? "This may belong to a task you have" : "This may belong to a task you have:")
                    .font(large ? TypeRole.workflowContext.weight(.semibold) : TypeRole.caption.weight(.semibold))
                ForEach(Array(matches.enumerated()), id: \.element.task.id) { index, match in
                    row(match, key: index + 1)
                }
            }
        }
    }

    private func row(_ match: TaskMatch, key: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(match.task.title)
                    .font(large ? TypeRole.workflowContext : TypeRole.body)
                    .lineLimit(1)
                Text(Self.detail(match))
                    .font(large ? TypeRole.workflowKey : TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: Space.m)
            Button("Add to This Task") { add(match) }
                .keyboardShortcut(KeyEquivalent(Character("\(key)")), modifiers: .command)
                .controlSize(.small)
                .hoverHelp("Add this email to “\(match.task.title)” instead of making a new task (⌘\(key))")
        }
        .padding(.vertical, Space.xs)
        .accessibilityElement(children: .combine)
    }

    /// "Review · due Tue · Same subject and sender · 2 emails already".
    static func detail(_ m: TaskMatch) -> String {
        var parts = [m.task.category]
        if let day = m.task.dueDay { parts.append("due \(TriageCardText.dueWords(day, now: .now))") }
        let emails = 1 + m.task.linkedThreadIds.count
        if emails > 1 { parts.append("\(emails) emails already") }
        if !m.why.isEmpty { parts.append(m.why) }
        return parts.joined(separator: " · ")
    }
}
