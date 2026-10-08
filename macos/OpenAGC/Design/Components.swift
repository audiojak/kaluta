import AppKit
import SwiftUI

// Components (docs/design-system.md). Small on purpose: each exists
// because more than one view needed it, or because getting it wrong once
// already showed on screen.

/// The row of controls at the top of a column (the Inbox's Important-only
/// switch, category tabs). Place it with `.columnHeader { }`; it draws no
/// rule of its own.
struct ListHeaderBar<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: Space.m) {
            content
        }
        .font(TypeRole.meta)
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.s)
        .frame(maxWidth: .infinity)
    }
}

/// A separator between items inside a panel, inset from both sides so it
/// never meets a column's edge. The only divider content may draw; menus
/// keep `Divider()`, and columns are separated by the split view.
struct InsetRule: View {
    var inset: CGFloat = Space.l

    var body: some View {
        Divider().padding(.horizontal, inset)
    }
}

/// A vertical rule between two panes that share a column (the reader and
/// the agent column).
struct PaneDivider: View {
    var body: some View {
        Divider()
    }
}

/// The small dot after a sidebar entry with something new (`Tone.newItems`).
struct NewDot: View {
    var body: some View {
        Circle()
            .fill(Tone.newItems)
            .frame(width: Self.size, height: Self.size)
            .accessibilityLabel("New")
    }

    private static let size: CGFloat = 7
}

/// A notice or warning across a column: an icon, a line of text and an
/// optional action (sign in again, cannot send from here).
struct Banner<Actions: View>: View {
    let intent: Tone.Intent
    let systemImage: String?
    let text: String
    /// Horizontal inset: a column's bar padding, or the reader's margin
    /// so the banner lines up with the message.
    var inset: CGFloat
    @ViewBuilder var actions: Actions

    init(_ text: String, systemImage: String? = nil, intent: Tone.Intent = .info, inset: CGFloat = Space.l,
         @ViewBuilder actions: () -> Actions = { EmptyView() }) {
        self.text = text
        self.systemImage = systemImage
        self.intent = intent
        self.inset = inset
        self.actions = actions()
    }

    var body: some View {
        HStack(spacing: Space.m) {
            if let systemImage { Image(systemName: systemImage).foregroundStyle(.secondary) }
            Text(text)
            Spacer(minLength: 0)
            actions.controlSize(.small)
        }
        .font(TypeRole.meta)
        .bandBackground(intent, inset: inset)
        .accessibilityElement(children: .contain)
    }
}

/// A label as a chip: the label's colour, faint, behind its name.
struct LabelChip: View {
    let name: String
    let colorHex: String?

    var body: some View {
        Text(name)
            .font(Font(TypeRole.chip))
            .padding(.horizontal, Space.xs)
            .padding(.vertical, Space.hair)
            .background(Color(nsColor: Tone.chipFill(hex: colorHex)), in: .rect(cornerRadius: Radius.chip))
    }
}

/// A task's category as a chip: the category's colour, faint, behind its
/// name, the same shape as a label chip. `selected` draws the stronger
/// fill used where categories are chosen (the task dialog).
struct CategoryChip: View {
    let name: String
    var selected = false

    var body: some View {
        Text(name)
            .font(Font(TypeRole.chip))
            .lineLimit(1)
            .padding(.horizontal, Space.xs)
            .padding(.vertical, Space.hair)
            .background(Color(nsColor: Tone.category(name).withAlphaComponent(selected ? Tone.chipSelectedOpacity
                                                                                        : Tone.chipFillOpacity)),
                        in: .rect(cornerRadius: Radius.chip))
            .accessibilityLabel("Category: \(name)")
    }
}

/// Chips laid out in lines, wrapping at the available width (the task
/// dialog's categories).
struct ChipFlow: Layout {
    var spacing: CGFloat = Space.xs

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = lines(width: proposal.width ?? .infinity, subviews: subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in lines(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Line { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func lines(width: CGFloat, subviews: Subviews) -> [Line] {
        var rows: [Line] = [Line()]
        for (index, view) in subviews.enumerated() {
            let size = view.sizeThatFits(.unspecified)
            let added = rows[rows.count - 1].items.isEmpty ? size.width : rows[rows.count - 1].width + spacing + size.width
            if added > width, !rows[rows.count - 1].items.isEmpty {
                rows.append(Line(items: [index], width: size.width, height: size.height))
            } else {
                rows[rows.count - 1].items.append(index)
                rows[rows.count - 1].width = added
                rows[rows.count - 1].height = max(rows[rows.count - 1].height, size.height)
            }
        }
        return rows
    }
}

/// A sheet asking for one decision or a short form (spec §14.8's task
/// dialog, Import Mailbox, a routine's prompt): a title, at most one
/// sentence under it, the content, and a button bar. In the bar, extra
/// actions go leading ("Ask Again"); trailing come Cancel and then the
/// default action. Cancel answers Escape (`CancelButton`) and the default
/// action Return (`.keyboardShortcut(.defaultAction)`); focus starts in the
/// first field.
struct Dialog<Content: View, Leading: View, Buttons: View>: View {
    let title: String
    var message: String?
    /// Fixed width; `nil` lets the content decide (a large editor).
    var width: CGFloat? = DialogMetrics.width
    @ViewBuilder var content: Content
    @ViewBuilder var leading: Leading
    @ViewBuilder var buttons: Buttons

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(title).font(TypeRole.title)
                if let message {
                    Text(message).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            content
            HStack(spacing: Space.m) {
                leading
                Spacer(minLength: 0)
                buttons
            }
        }
        .padding(Space.xxxl)
        .frame(width: width)
    }
}

extension Dialog where Leading == EmptyView {
    init(title: String, message: String? = nil, width: CGFloat? = DialogMetrics.width,
         @ViewBuilder content: () -> Content, @ViewBuilder buttons: () -> Buttons) {
        self.init(title: title, message: message, width: width, content: content, leading: { EmptyView() },
                  buttons: buttons)
    }
}

enum DialogMetrics {
    /// A dialog's usual width.
    static let width: CGFloat = 460
}

/// A dialog's Cancel: answers Escape.
struct CancelButton: View {
    var title = "Cancel"
    var help = "Close without changes (Esc)"
    let action: () -> Void

    var body: some View {
        Button(title, role: .cancel, action: action)
            .keyboardShortcut(.cancelAction)
            .hoverHelp(help)
    }
}

/// A row of tabs as pills, for a column header (the Inbox's category
/// tabs), as Mail draws them: equal pills with a symbol each; the chosen
/// one widens to show its name. Two tabs whose names fit both show their
/// names. The highlight slides to the chosen pill (none with Reduce
/// Motion). Counts are in the help tag and read by VoiceOver; the list's
/// subtitle carries the chosen tab's.
struct CapsuleTabs: View {
    struct Tab: Identifiable, Equatable {
        let id: String
        let title: String
        let symbol: String
        /// Read in the help tag and by VoiceOver when non-zero (unread).
        var count: Int = 0
    }

    let tabs: [Tab]
    @Binding var selection: String?
    /// Read after the count by VoiceOver and in the help tag ("unread").
    var countNoun = "unread"
    @Namespace private var highlight
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if tabs.count <= Self.namedTabs {
                // Every name when they fit; otherwise only the chosen one's.
                ViewThatFits(in: .horizontal) {
                    row(named: true)
                    row(named: false)
                }
            } else {
                row(named: false)
            }
        }
        // Also when the selection changes from elsewhere (a key, a link).
        .animation(reduceMotion ? nil : .snappy(duration: Self.duration), value: selection)
        .accessibilityElement(children: .contain)
    }

    private func row(named: Bool) -> some View {
        HStack(spacing: Space.s) {
            ForEach(tabs) { tab in
                pill(tab, named: named)
            }
        }
    }

    private func pill(_ tab: Tab, named: Bool) -> some View {
        let chosen = tab.id == selection
        return Button {
            selection = tab.id
        } label: {
            HStack(spacing: Space.xs) {
                Image(systemName: tab.symbol)
                if named || chosen {
                    Text(tab.title).fontWeight(chosen ? .medium : .regular)
                }
            }
            .lineLimit(1)
            .frame(minWidth: Self.pillWidth, maxWidth: named || chosen ? .infinity : nil)
            .padding(.horizontal, Space.m)
            .padding(.vertical, Space.s)
            .background {
                if chosen {
                    Capsule().fill(Tone.highlight).matchedGeometryEffect(id: "chosen", in: highlight)
                } else {
                    Capsule().fill(Tone.controlFill)
                }
            }
            .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .hoverHelp(tab.count > 0 ? "\(tab.title), \(tab.count) \(countNoun)" : tab.title)
        .accessibilityLabel(tab.title)
        .accessibilityValue(tab.count > 0 ? "\(tab.count) \(countNoun)" : "")
        .accessibilityAddTraits(chosen ? [.isSelected] : [])
    }

    /// Unchosen pills are this wide at least, so they line up.
    static let pillWidth: CGFloat = 28
    /// Up to this many tabs, every name shows when they fit.
    static let namedTabs = 2
    private static let duration: Double = 0.2
}

/// One answer to a question, as a large button that answers on click (the
/// writing guide's questions): its title, a number key, and a check on the
/// answer given before.
struct AnswerButton: View {
    let title: String
    /// A line under the answer, secondary (what a service is).
    var detail: String?
    /// 1 to 9: the key that picks it.
    let number: Int
    var chosen = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                Text("\(number)")
                    .font(TypeRole.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text(title).multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                    if let detail {
                        Text(detail).font(TypeRole.caption).foregroundStyle(.secondary)
                            .multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: Space.m)
                if chosen {
                    Image(systemName: "checkmark").foregroundStyle(.tint).accessibilityHidden(true)
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(chosen ? Tone.highlight : Tone.controlFill, in: .rect(cornerRadius: Radius.card))
            .contentShape(.rect(cornerRadius: Radius.card))
        }
        .buttonStyle(.plain)
        .keyboardShortcut(KeyEquivalent(Character("\(number)")), modifiers: [])
        .hoverHelp(chosen ? "Your answer; choose it to keep it and go on (\(number))" : "Answer this and go on (\(number))")
        .accessibilityAddTraits(chosen ? [.isSelected] : [])
    }
}

extension KeyEquivalent {
    /// ⌫ as the keyboard sends it (DEL, U+007F; `.delete` is backspace,
    /// U+0008, which macOS keyboards never send) and ⌦: for
    /// `onKeyPress(keys:)`.
    static let deleteKeys: Set<KeyEquivalent> = [.delete, .deleteForward, KeyEquivalent("\u{7F}")]
}

extension View {
    /// What Return does, drawn prominent while it is (the current card in
    /// a review flow); otherwise an ordinary button.
    @ViewBuilder func defaultAction(_ isDefault: Bool) -> some View {
        if isDefault {
            buttonStyle(.borderedProminent)
        } else {
            buttonStyle(.bordered)
        }
    }

    /// `.help` that also shows where SwiftUI's own tool tips do not: in a
    /// `.columnHeader` bar (safe-area bars) on macOS 26 nothing appears on
    /// hover. Adds an AppKit tool tip over the control that lets every
    /// click through.
    func hoverHelp(_ text: String) -> some View {
        help(text).overlay(ToolTipArea(text: text))
    }
}

private struct ToolTipArea: NSViewRepresentable {
    let text: String

    func makeNSView(context: Context) -> PassThroughToolTipView { PassThroughToolTipView() }

    func updateNSView(_ view: PassThroughToolTipView, context: Context) {
        if view.toolTip != text { view.toolTip = text }
    }

    final class PassThroughToolTipView: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}

/// A tip introducing a feature, as Mail introduces Categories: an icon, a
/// title, one sentence, the main action and a dismiss. Shown one at a time
/// at the top of a column; any button puts it away for good.
struct TipCard: View {
    let systemImage: String
    let title: String
    let text: String
    let action: String
    let actionHelp: String
    let dismiss: String
    let onAction: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: Space.m) {
            Image(systemName: systemImage)
                .font(.title3)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(title).font(TypeRole.groupLabel)
                Text(text).font(TypeRole.meta).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: Space.m) {
                    Button(action, action: onAction)
                        .buttonStyle(.borderedProminent)
                        .hoverHelp(actionHelp)
                    Button(dismiss, action: onDismiss)
                        .hoverHelp("Put this tip away; it will not come back")
                }
                .controlSize(.small)
                .padding(.top, Space.xs)
            }
            Spacer(minLength: 0)
        }
        .card(.info)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
    }
}
