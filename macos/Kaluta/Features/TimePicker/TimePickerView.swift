import AppKit
import SwiftUI

/// A time picker: a field that reads a typed time over the ready choices.
/// ↑↓ select, Return chooses, Esc closes.
struct TimePickerView: View {
    let title: String
    let presets: [AppModel.ReminderTime]
    /// What choosing a time does, for each row's help: "Remind you", "Send".
    var verb = "Remind you"
    let choose: (AppModel.ReminderTime) -> Void

    @State private var text = ""
    @State private var selected = 0
    @FocusState private var focused: Bool

    private var picker: TimePickerModel { TimePickerModel(text: text, presets: presets) }

    var body: some View {
        let picker = picker
        VStack(alignment: .leading, spacing: Space.s) {
            Text(title).font(TypeRole.groupLabel)
            TextField(TimeInput.prompt, text: $text)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit { pick(picker) }
                .onKeyPress(.downArrow) { selected = picker.moved(from: selected, by: 1); return .handled }
                .onKeyPress(.upArrow) { selected = picker.moved(from: selected, by: -1); return .handled }
                .onChange(of: text) { selected = 0 }
                .accessibilityLabel(title)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(picker.rows.enumerated()), id: \.element.id) { index, row in
                    Button { choose(row.time) } label: {
                        HStack(spacing: Space.m) {
                            Text(row.title)
                            Spacer(minLength: Space.m)
                            Text(row.detail).foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, Space.s)
                        .padding(.vertical, Space.xs)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(index == selected ? Tone.highlight : AnyShapeStyle(.clear),
                                    in: RoundedRectangle(cornerRadius: Radius.chip))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .hoverHelp("\(verb) \(row.time.phrase)")
                    .accessibilityAddTraits(index == selected ? .isSelected : [])
                }
                if picker.rows.isEmpty {
                    Text("Not a time Kaluta can read")
                        .foregroundStyle(.secondary)
                        .padding(.vertical, Space.xs)
                }
            }
            KeyHints(hints: KeyHints.choose)
        }
        .padding(Space.m)
        .frame(width: 300)
        .onAppear { focused = true }
    }

    private func pick(_ picker: TimePickerModel) {
        guard picker.rows.indices.contains(selected) else { return }
        choose(picker.rows[selected].time)
    }
}

/// Hosts the picker in a transient popover anchored at a table row (`b`
/// in the mail window).
@MainActor
enum RemindPopover {
    static func show(relativeTo rect: NSRect, of view: NSView, model: AppModel) {
        let popover = NSPopover()
        popover.behavior = .transient
        let picker = TimePickerView(title: "Remind you when?", presets: AppModel.reminderTimes()) { time in
            popover.performClose(nil)
            Task { await model.remindSelection(time) }
        }
        popover.contentViewController = NSHostingController(rootView: picker.environment(model))
        popover.show(relativeTo: rect, of: view, preferredEdge: .maxY)
    }
}
