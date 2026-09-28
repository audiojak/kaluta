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
