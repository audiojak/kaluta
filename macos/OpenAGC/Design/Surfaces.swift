import SwiftUI

// Surfaces (docs/design-system.md): the few ways something sits above or
// on the window. Each one is a modifier so views say what they are, not
// how they are drawn.

extension View {
    /// A floating glass capsule: the undo notice, the agent prompt,
    /// suggestion chips. Floats over content, never pinned to a column's
    /// edge, and holds no dividers.
    func glassCapsule(interactive: Bool = false) -> some View {
        padding(.horizontal, Space.xl)
            .padding(.vertical, Space.m)
            .glassEffect(interactive ? .regular.interactive() : .regular, in: .capsule)
    }

    /// A card on the background: approval requests, tool calls, previews.
    /// Cards are the only content with outlines.
    func card(_ intent: Tone.Intent = .neutral, padding: CGFloat = Space.l) -> some View {
        self.padding(padding)
            .background(intent.fill, in: .rect(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).strokeBorder(intent.stroke))
    }

    /// A full-width band in a column (see `Banner`), tinted by intent. Its
    /// fill stays inside the column's safe area, so it never reaches under
    /// the floating sidebar.
    func bandBackground(_ intent: Tone.Intent, inset: CGFloat = Space.l) -> some View {
        padding(.horizontal, inset)
            .padding(.vertical, Space.s)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(intent.fill)
    }

    /// A column header that the content scrolls under (macOS 26): a
    /// safe-area bar with the system's hard scroll edge instead of a drawn
    /// rule, so the edge is drawn only across the column's visible part.
    func columnHeader(@ViewBuilder _ content: () -> some View) -> some View {
        safeAreaBar(edge: .top, spacing: 0, content: content)
            .scrollEdgeEffectStyle(.hard, for: .top)
    }
}
