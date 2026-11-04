import SwiftUI

/// A calendar invitation in a conversation (spec §14.9): what, when and
/// who, and Accept, Maybe and Decline, which answer on the user's Google
/// Calendar as Gmail's buttons do. Shown in the reader and on a
/// workflow's item; nothing for a conversation without one.
struct InviteBar: View {
    @Environment(AppModel.self) private var model
    let threadID: String

    var body: some View {
        Group {
            if let invite = model.invites[threadID] {
                bar(invite)
            }
        }
        .task(id: threadID) { await model.loadInvite(threadID) }
    }

    private func bar(_ invite: CalendarInvite) -> some View {
        HStack(alignment: .center, spacing: Space.l) {
            Image(systemName: invite.state == .cancelled ? "calendar.badge.minus" : "calendar")
                .font(TypeRole.title)
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(invite.title).font(TypeRole.body.weight(.semibold)).lineLimit(1)
                Text(Self.details(invite)).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: Space.l)
            if invite.state == .answerable {
                answers(invite)
            } else {
                Text(Self.why(invite))
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                    .lineLimit(2)
            }
        }
        .padding(Space.l)
        .background(RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.controlFill))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Invitation: \(invite.title), \(Self.details(invite))")
    }

    private func answers(_ invite: CalendarInvite) -> some View {
        HStack(spacing: Space.s) {
            if model.inviteWorking == threadID {
                ProgressView().controlSize(.small)
            }
            ForEach(Self.choices, id: \.title) { choice in
                let chosen = invite.response == choice.response
                Button { Task { await model.respondToInvite(threadID, choice.response) } } label: {
                    Label(choice.title, systemImage: chosen ? "checkmark" : choice.icon)
                }
                .buttonStyle(.bordered)
                .tint(chosen ? .accentColor : nil)
                .controlSize(.small)
                .disabled(model.inviteWorking != nil || chosen)
                .hoverHelp(chosen ? "Your answer on Google Calendar" : "\(choice.title) on your Google Calendar; the organiser is told")
                .accessibilityLabel(chosen ? "\(choice.title), your answer" : choice.title)
            }
        }
    }

    static let choices: [(title: String, icon: String, response: InviteResponse)] = [
        ("Accept", "checkmark.circle", .accepted),
        ("Maybe", "questionmark.circle", .tentative),
        ("Decline", "xmark.circle", .declined),
    ]

    /// "Fri 6 Nov, 6:00 – 8:00 PM · Example Hall · from Events".
    static func details(_ i: CalendarInvite, zone: TimeZone = .current) -> String {
        var parts = [when(i, zone: zone)]
        if let location = i.location, !location.isEmpty { parts.append(location) }
        if let organizer = i.organizer, !organizer.isEmpty { parts.append("from \(organizer)") }
        return parts.joined(separator: " · ")
    }

    static func when(_ i: CalendarInvite, zone: TimeZone = .current) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(i.startMs) / 1000)
        if i.allDay {
            var utc = Date.FormatStyle(date: .abbreviated, time: .omitted)
            utc.timeZone = TimeZone(identifier: "UTC") ?? zone
            return start.formatted(utc.weekday(.abbreviated)) + ", all day"
        }
        var day = Date.FormatStyle().weekday(.abbreviated).day().month(.abbreviated)
        day.timeZone = zone
        var time = Date.FormatStyle(date: .omitted, time: .shortened)
        time.timeZone = zone
        guard let endMs = i.endMs else { return "\(start.formatted(day)), \(start.formatted(time))" }
        let end = Date(timeIntervalSince1970: TimeInterval(endMs) / 1000)
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        if calendar.isDate(start, inSameDayAs: end) {
            return "\(start.formatted(day)), \(start.formatted(time)) – \(end.formatted(time))"
        }
        return "\(start.formatted(day)), \(start.formatted(time)) – \(end.formatted(day)), \(end.formatted(time))"
    }

    /// Why it cannot be answered here.
    static func why(_ i: CalendarInvite) -> String {
        switch i.state {
        case .answerable: ""
        case .notConnected: "Connect Google Calendar in Settings › Accounts to answer here"
        case .noEventAccess: "Connect the calendar again with holds to answer here"
        case .notOnCalendar: "Not on your Google Calendar"
        case .notAGuest: "You organise this event"
        case .cancelled: "Cancelled by the organiser"
        case .past: i.response.map { "Over · \(answer($0))" } ?? "This event is over"
        }
    }

    static func answer(_ r: InviteResponse) -> String {
        switch r {
        case .accepted: "you accepted"
        case .tentative: "you said maybe"
        case .declined: "you declined"
        case .needsAction: "not answered"
        }
    }

    /// The notice after answering.
    static func notice(_ i: CalendarInvite) -> String {
        switch i.response {
        case .accepted: "Accepted “\(i.title)” on your calendar"
        case .tentative: "Said maybe to “\(i.title)” on your calendar"
        case .declined: "Declined “\(i.title)” on your calendar"
        default: "Answered “\(i.title)”"
        }
    }
}
