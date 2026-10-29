import AppKit
import SwiftUI

/// The open Google account's calendar (spec §14.9, ADR 0019): connect it
/// so Kaluta can offer free times and hold them, see the holds it made
/// and release them, or stop using it.
struct CalendarRow: View {
    @Environment(AppModel.self) private var model
    @State private var status: CalendarStatus?
    @State private var holds: [CalendarHoldInfo] = []
    @State private var working = false
    @State private var problem: String?

    var body: some View {
        LabeledContent("Calendar") {
            VStack(alignment: .trailing, spacing: Space.xs) {
                if let status, status.connected {
                    Text(Self.connectedText(status)).foregroundStyle(.secondary)
                    Button("Stop Using the Calendar") { Task { await disconnect() } }
                        .hoverHelp("Kaluta stops offering times from it; remove Kaluta's access in your Google account to revoke it")
                } else {
                    Button(working ? "Waiting for Google…" : "Connect Google Calendar…") { Task { await connect() } }
                        .disabled(working || status == nil)
                        .hoverHelp("Let Kaluta offer free times from your calendar when someone asks to meet, and hold them while you wait. It sees when you are busy, never what you are doing, and never invites anyone")
                }
                if let problem {
                    Text(problem).font(TypeRole.caption).foregroundStyle(Tone.failure)
                }
            }
        }
        ForEach(holds, id: \.id) { hold in
            LabeledContent(hold.title) {
                HStack(spacing: Space.m) {
                    Text(hold.label).foregroundStyle(.secondary)
                    Button("Release") { Task { await release(hold) } }
                        .hoverHelp("Delete this hold from your calendar")
                }
            }
        }
        .task(id: model.openAccountID) { await load() }
    }

    /// "Connected · 9 am–5 pm, America/Los_Angeles"; "· free times only"
    /// when holds were not granted.
    static func connectedText(_ status: CalendarStatus) -> String {
        "Connected · \(status.hours), \(status.timeZone)" + (status.canHold ? "" : " · free times only")
    }

    private func load() async {
        status = try? await model.core?.calendarStatus()
        holds = status?.connected == true ? ((try? await model.core?.calendarHolds()) ?? []) : []
    }

    private func connect() async {
        problem = await model.connectCalendar()
        await load()
    }

    private func disconnect() async {
        try? await model.core?.disconnectCalendar()
        await load()
    }

    private func release(_ hold: CalendarHoldInfo) async {
        do {
            try await model.core?.calendarRelease(hold.id)
        } catch {
            problem = error.message
        }
        await load()
    }
}

extension AppModel {
    /// Ask Google for calendar access for the open account (spec §14.9):
    /// the browser shows Google's consent; the account keeps its mail
    /// access. A problem to show, or nil.
    func connectCalendar() async -> String? {
        guard let core else { return nil }
        do {
            let start = try await core.beginCalendarConnect()
            signInSession = start.sessionID
            NSWorkspace.shared.open(start.authorizationURL)
            _ = try await core.completeGmailSignIn(start.sessionID)
            signInSession = nil
            let status = try await core.calendarStatus()
            return status.connected ? nil : "Google did not grant calendar access."
        } catch {
            signInSession = nil
            return error.kind == .cancelled ? nil : error.message
        }
    }
}
