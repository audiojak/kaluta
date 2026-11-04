import SwiftUI

/// *Tidy the Inbox* on the Dashboard (spec §14.12, on the Dashboard): a
/// few large groups of Inbox mail worth archiving whole, each one click
/// and one ⌘Z, and the way to Clean Up for the rest.
struct InboxTidyCard: View {
    @Environment(AppModel.self) private var model
    let tidy: InboxTidy

    var body: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            HStack(alignment: .firstTextBaseline, spacing: Space.l) {
                Text("Tidy the Inbox").font(TypeRole.workflowHeading)
                Spacer(minLength: Space.l)
                Button("Open Clean Up…", systemImage: "wand.and.stars") { model.openCleanUp?() }
                    .buttonStyle(.link)
                    .hoverHelp("Clean Up: the whole mailbox by sender, subject, time or size")
            }
            Text(Self.summary(tidy))
                .font(TypeRole.workflowContext)
                .foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(Array(tidy.suggestions.enumerated()), id: \.element.title) { index, suggestion in
                    if index > 0 { InsetRule(inset: 0) }
                    row(suggestion)
                }
            }
        }
        .padding(Space.xxl)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background { RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.workflowSurface) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Tidy the Inbox")
    }

    private func row(_ s: TidySuggestion) -> some View {
        HStack(spacing: Space.l) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(s.title).font(TypeRole.workflowContext.weight(.semibold)).lineLimit(1)
                if s.detail != s.title {
                    Text(s.detail).font(TypeRole.workflowKey).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: Space.l)
            Text(s.count.formatted())
                .font(TypeRole.workflowContext.monospacedDigit())
                .foregroundStyle(.secondary)
            if model.tidyWorking == s.title {
                ProgressView().controlSize(.small).frame(width: Self.buttonWidth)
            } else {
                Button("Archive") { Task { await model.archiveTidy(s) } }
                    .frame(width: Self.buttonWidth)
                    .disabled(model.tidyWorking != nil)
                    .hoverHelp("Archive \(Self.messages(s.count)) (⌘Z undoes it)")
                    .accessibilityLabel("Archive \(s.title), \(Self.messages(s.count))")
            }
        }
        .padding(.vertical, Space.m)
    }

    /// "6,706 messages in the Inbox. These groups are seldom answered: archive one whole."
    static func summary(_ tidy: InboxTidy) -> String {
        "\(messages(tidy.inbox)) in the Inbox. Mail like this is seldom answered; archive a group whole and ⌘Z brings it back."
    }

    static func messages(_ n: UInt64) -> String {
        "\(n.formatted()) message\(n == 1 ? "" : "s")"
    }

    static let buttonWidth: CGFloat = 84
}
