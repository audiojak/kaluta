import SwiftUI

/// A Follow-ups item (spec §14.14): the meeting, who was there, why
/// Kaluta thinks it deserves a follow-up, and the ways to write it, each
/// on its key (1–3), which open in a composer to edit and send.
struct FollowUpItemView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let item: WorkflowItem

    var body: some View {
        if let meeting = item.meetingID.flatMap({ session.meetings[$0] }) {
            VStack(alignment: .leading, spacing: Space.xxl) {
                VStack(alignment: .leading, spacing: Space.s) {
                    Text(Self.context(meeting)).font(TypeRole.workflowContext).foregroundStyle(.secondary)
                    Text(meeting.title.isEmpty ? "A meeting" : meeting.title).font(TypeRole.workflowDisplay)
                    if !meeting.reason.isEmpty {
                        Label(meeting.reason, systemImage: "sparkle")
                            .font(TypeRole.workflowContext)
                            .foregroundStyle(.secondary)
                    }
                }
                VStack(alignment: .leading, spacing: Space.l) {
                    ForEach(Array(meeting.options.prefix(3).enumerated()), id: \.offset) { index, option in
                        FollowUpOptionCard(option: option, key: index + 1, kind: session.kind) {
                            Task { await model.performWorkflow(Self.pick(index)) }
                        }
                    }
                }
            }
        } else {
            ProgressView().controlSize(.small)
        }
    }

    static func pick(_ index: Int) -> WorkflowCommand {
        switch index {
        case 0: .pick1
        case 1: .pick2
        default: .pick3
        }
    }

    /// "Yesterday at 11:20 AM · with Reese Park and Sam Rivera".
    static func context(_ m: MeetingFollowUp, now: Date = .now) -> String {
        let at = Date(timeIntervalSince1970: TimeInterval(m.meetingAt) / 1000)
        let when = at.formatted(.relative(presentation: .named, unitsStyle: .wide))
        return "\(when.prefix(1).uppercased() + when.dropFirst()) · \(with(m.people))"
    }

    static func with(_ people: [MeetingPerson]) -> String {
        let names = people.map { $0.name.isEmpty ? ($0.email ?? "someone") : $0.name }
        switch names.count {
        case 0: return "no one else"
        case 1: return "with \(names[0])"
        case 2: return "with \(names[0]) and \(names[1])"
        default: return "with \(names[0]), \(names[1]) and \(names.count - 2) more"
        }
    }
}

/// One way to follow up: its label and key, to whom, and the draft.
private struct FollowUpOptionCard: View {
    let option: FollowUpOption
    let key: Int
    let kind: WorkflowKind
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: Space.s) {
                HStack(spacing: Space.m) {
                    Text(option.label).font(TypeRole.workflowHeading)
                    Spacer(minLength: 0)
                    Keycap(key: "\(key)")
                }
                Text("To \(option.to.joined(separator: ", ")) · \(option.subject)")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Text(option.body)
                    .font(TypeRole.workflowContext)
                    .lineLimit(6)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(Space.xl)
            .background {
                RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.workflowSurface).workflowMorph(kind)
            }
            .contentShape(RoundedRectangle(cornerRadius: Radius.panel))
        }
        .buttonStyle(.plain)
        .hoverHelp("Open this follow-up in a new email to edit and send (\(key))")
        .accessibilityLabel("\(option.label). To \(option.to.joined(separator: ", ")). \(option.subject)")
    }
}

extension WorkflowItem {
    /// A Follow-ups item's meeting (`meeting:<id>`).
    var meetingID: String? {
        key.hasPrefix("meeting:") ? String(key.dropFirst("meeting:".count)) : nil
    }
}
