import SwiftUI

/// What an email hides from its reader (spec §14.4): text styled not to
/// show, and invisible characters. Kaluta left it out of what it reads;
/// here the user sees it, in the words it had.
struct HiddenTextNote: View {
    let hidden: [HiddenTextInfo]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            ForEach(Array(Self.lines(hidden).enumerated()), id: \.offset) { _, line in
                Text(line)
                    .font(TypeRole.workflowContext)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: Radius.panel))
        .accessibilityLabel("What the email hides")
    }

    /// The banner's line: what kind of thing was hidden.
    nonisolated static func summary(_ hidden: [HiddenTextInfo]) -> String {
        let texts = hidden.reduce(0) { $0 + $1.texts.count }
        let invisible = hidden.reduce(0) { $0 + Int($1.invisible) }
        if texts > 0 {
            return "This email has text styled not to show. Kaluta left it out of what it reads."
        }
        return "This email has \(invisible) invisible characters. Kaluta left them out of what it reads."
    }

    /// What was hidden, a line each; with no words, what the characters were.
    nonisolated static func lines(_ hidden: [HiddenTextInfo]) -> [String] {
        let texts = hidden.flatMap(\.texts)
        if !texts.isEmpty { return texts }
        let invisible = hidden.reduce(0) { $0 + Int($1.invisible) }
        return ["\(invisible) invisible characters (zero-width or direction marks), spelling nothing."]
    }
}
