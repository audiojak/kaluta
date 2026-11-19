import SwiftUI

/// The command palette (⌘K): a field over every command, mailbox and
/// label. ↑↓ select, Return runs, Esc closes; text that matches no command
/// is asked of Kaluta.
struct PaletteView: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @State private var selected = 0
    @FocusState private var focused: Bool
    let close: () -> Void

    var body: some View {
        let palette = PaletteModel(commands: model.mailCommands, query: text)
        VStack(alignment: .leading, spacing: Space.s) {
            TextField("Type a command, a mailbox, or a question for Kaluta", text: $text)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit { run(palette, selected) }
                .onKeyPress(.downArrow) { selected = palette.moved(from: selected, by: 1); return .handled }
                .onKeyPress(.upArrow) { selected = palette.moved(from: selected, by: -1); return .handled }
                .onChange(of: text) {
                    selected = PaletteModel(commands: model.mailCommands, query: text).firstRunnable
                }
                .accessibilityLabel("Command")
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(palette.rows.enumerated()), id: \.element.id) { index, row in
                            rowView(row, index: index, palette: palette).id(row.id)
                        }
                    }
                }
                .frame(height: 320)
                .onChange(of: selected) { _, now in
                    if palette.rows.indices.contains(now) { proxy.scrollTo(palette.rows[now].id) }
                }
            }
            HStack {
                KeyHints(hints: [KeyHint(keys: "↑↓", does: "Select"), KeyHint(keys: "Return", does: "Run"),
                                 KeyHint(keys: "Esc", does: "Close")])
                Button("Close", action: close)
                    .keyboardShortcut(.cancelAction)
                    .hoverHelp("Close the palette (Esc)")
            }
        }
        .padding(Space.l)
        .frame(width: 520)
        .onAppear { focused = true }
    }

    private func rowView(_ row: PaletteModel.Row, index: Int, palette: PaletteModel) -> some View {
        Button { run(palette, index) } label: {
            HStack(spacing: Space.m) {
                if case .ask = row {
                    Image(systemName: "sparkles").foregroundStyle(.secondary).accessibilityHidden(true)
                }
                Text(row.title).lineLimit(1)
                Spacer(minLength: Space.m)
                if let reason = row.unavailable {
                    Text(reason).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
                } else if case let .command(command) = row, let keys = command.keys {
                    Keycap(key: keys)
                }
            }
            .foregroundStyle(row.unavailable == nil ? .primary : .secondary)
            .padding(.horizontal, Space.s)
            .padding(.vertical, Space.xs)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(index == selected ? Tone.highlight : AnyShapeStyle(.clear),
                        in: RoundedRectangle(cornerRadius: Radius.chip))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(row.unavailable != nil)
        .hoverHelp(row.unavailable ?? row.title)
        .accessibilityAddTraits(index == selected ? .isSelected : [])
    }

    private func run(_ palette: PaletteModel, _ index: Int) {
        guard palette.rows.indices.contains(index), palette.rows[index].unavailable == nil else { return }
        let row = palette.rows[index]
        close()
        switch row {
        case let .command(command): command.run()
        case let .ask(text): model.askFromPalette(text)
        }
    }
}

/// Presents the palette over the mail window. Its own view, so the
/// window's long chain of sheets stays one the compiler can check.
struct PaletteHost: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Color.clear
            .sheet(isPresented: Binding(get: { model.paletteShown }, set: { model.paletteShown = $0 })) {
                PaletteView { model.paletteShown = false }
            }
            .accessibilityHidden(true)
    }
}
