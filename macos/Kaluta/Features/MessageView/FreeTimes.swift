import AppKit
import SwiftUI

/// Free times beside a conversation that asks to meet, and in the
/// composer (spec §14.9 amendment 2026-11-18). Only free times are shown:
/// the calendar's events and their titles never are (ADR 0019).
enum FreeTimes {
    /// The free times to show: those in the next `days` days, else the
    /// first few after that, at most `limit`.
    static func shown(_ slots: [CalendarSlot], now: Date = .now, days: Int = 3, limit: Int = 6,
                      calendar: Calendar = .current) -> [CalendarSlot] {
        let end = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) ?? now
        let soon = slots.filter { CoreClient.date($0.startMs) < end }
        return Array((soon.isEmpty ? slots : soon).prefix(limit))
    }

    /// What Insert Free Times writes: a line a time, as the workflow's
    /// replies offer them.
    static func insertion(_ slots: [CalendarSlot], zone: String) -> String {
        guard !slots.isEmpty else { return "" }
        let lines = slots.map { "- \($0.label)" }.joined(separator: "\n")
        return zone.isEmpty ? lines + "\n" : "\(lines)\n(\(zone))\n"
    }

    /// Minutes a free time lasts, as a hold takes them.
    static func minutes(_ slot: CalendarSlot) -> UInt32 {
        UInt32(max(15, min(240, (slot.endMs - slot.startMs) / 60_000)))
    }
}

/// Over a conversation that asks to meet, with the calendar connected:
/// the next free times, each of which can be held or copied.
struct FreeTimesBar: View {
    @Environment(AppModel.self) private var model
    let threadID: String
    let subject: String
    /// The conversation's latest text, to tell whether it asks to meet.
    let text: String

    @State private var slots: [CalendarSlot] = []
    @State private var zone = ""
    @State private var canHold = false
    @State private var held: Set<Int64> = []
    @State private var error: String?

    var body: some View {
        Group {
            if !slots.isEmpty {
                VStack(alignment: .leading, spacing: Space.xs) {
                    HStack(spacing: Space.s) {
                        Image(systemName: "calendar").foregroundStyle(.secondary).accessibilityHidden(true)
                        Text("You're free").font(TypeRole.caption.weight(.semibold))
                        Text(zone).font(TypeRole.caption).foregroundStyle(.secondary)
                        if let error { Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure) }
                    }
                    ScrollView(.horizontal) {
                        HStack(spacing: Space.s) {
                            ForEach(slots, id: \.startMs) { slot in chip(slot) }
                        }
                    }
                    .scrollIndicators(.hidden)
                }
                .padding(.horizontal, Space.xxl)
                .padding(.bottom, Space.s)
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Free times")
            }
        }
        .task(id: threadID) { await load() }
    }

    private func chip(_ slot: CalendarSlot) -> some View {
        let isHeld = held.contains(slot.startMs)
        return Menu {
            Button(isHeld ? "Held on Your Calendar" : "Hold on Your Calendar") { Task { await hold(slot) } }
                .disabled(!canHold || isHeld)
            Button("Copy") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(slot.label, forType: .string)
            }
        } label: {
            Label(slot.label, systemImage: isHeld ? "checkmark" : "clock")
                .font(TypeRole.caption)
        }
        .menuStyle(.button)
        .controlSize(.small)
        .fixedSize()
        .hoverHelp(canHold ? "Hold this time on your calendar, or copy it" : "Copy this time")
    }

    private func load() async {
        slots = []
        held = []
        error = nil
        guard WritingText.asksToMeet(subject + "\n" + text), let core = model.core,
              let status = try? await core.calendarStatus(), status.connected,
              let found = try? await core.calendarFreeSlots() else { return }
        canHold = status.canHold
        zone = found.timeZone
        slots = FreeTimes.shown(found.slots)
    }

    private func hold(_ slot: CalendarSlot) async {
        guard let core = model.core else { return }
        do {
            _ = try await core.calendarHold(startMs: slot.startMs, minutes: FreeTimes.minutes(slot),
                                            title: subject.isEmpty ? "Hold" : subject, threadID: threadID)
            held.insert(slot.startMs)
            error = nil
        } catch {
            self.error = error.message
        }
    }
}
