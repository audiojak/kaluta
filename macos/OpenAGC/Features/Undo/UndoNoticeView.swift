import SwiftUI

/// "Archived 3 conversations — Undo ⌘Z" at the bottom of the thread list
/// (spec §14.6a). One at a time; 8 seconds, paused while the pointer is
/// over it, while it has keyboard focus and while the window is inactive.
struct UndoNoticeView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.appearsActive) private var appearsActive
    @FocusState private var focused: Bool

    var body: some View {
        let undo = model.undo
        ZStack(alignment: .bottom) {
            if let notice = undo.notice, notice.accountID == model.openAccountID {
                HStack(spacing: Space.m) {
                    Text(notice.text)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if notice.offersUndo {
                        Button {
                            model.undoMailAction()
                        } label: {
                            HStack(spacing: Space.xs) {
                                Text("Undo").fontWeight(.semibold)
                                Text("⌘Z").foregroundStyle(.secondary)
                            }
                        }
                        .hoverHelp("Undo this (⌘Z)")
                        .buttonStyle(.plain)
                        .focused($focused)
                        .accessibilityLabel("Undo")
                        .accessibilityHint("Or press Command Z")
                    }
                    Button {
                        undo.dismissNotice()
                    } label: {
                        Image(systemName: "xmark").font(TypeRole.caption.weight(.semibold))
                    }
                    .hoverHelp("Dismiss")
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Close")
                }
                .font(TypeRole.meta)
                .glassCapsule()
                .onHover { undo.setPaused(.hover, $0) }
                .padding(.bottom, Space.l)
                .padding(.horizontal, Space.l)
                .transition(reduceMotion ? .opacity : .move(edge: .bottom).combined(with: .opacity))
                .id(notice.id)
            }
        }
        .frame(maxWidth: .infinity)
        .animation(reduceMotion ? nil : .snappy(duration: 0.25), value: undo.notice?.id)
        .onChange(of: focused) { _, isFocused in undo.setPaused(.focus, isFocused) }
        .onChange(of: appearsActive, initial: true) { _, active in undo.setPaused(.inactiveWindow, !active) }
    }
}
