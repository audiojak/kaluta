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

/// A row of tabs as pills, for a column header (the Inbox's category
/// tabs), as Mail draws them: equal pills with a symbol each; the chosen
/// one widens to show its name. Counts are in the help tag and read by
/// VoiceOver; the list's subtitle carries the chosen tab's.
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

    var body: some View {
        HStack(spacing: Space.s) {
            ForEach(tabs) { tab in
                let chosen = tab.id == selection
                Button { selection = tab.id } label: {
                    HStack(spacing: Space.xs) {
                        Image(systemName: tab.symbol)
                        if chosen { Text(tab.title).fontWeight(.medium) }
                    }
                    .lineLimit(1)
                    .frame(minWidth: Self.pillWidth, maxWidth: chosen ? .infinity : nil)
                    .padding(.horizontal, Space.m)
                    .padding(.vertical, Space.s)
                    .background(chosen ? Tone.highlight : Tone.controlFill, in: .capsule)
                    .contentShape(.capsule)
                }
                .buttonStyle(.plain)
                .hoverHelp(tab.count > 0 ? "\(tab.title), \(tab.count) \(countNoun)" : tab.title)
                .accessibilityLabel(tab.title)
                .accessibilityValue(tab.count > 0 ? "\(tab.count) \(countNoun)" : "")
                .accessibilityAddTraits(chosen ? [.isSelected] : [])
            }
        }
        .accessibilityElement(children: .contain)
    }

    /// Unchosen pills are this wide at least, so they line up.
    static let pillWidth: CGFloat = 28
}

extension View {
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
