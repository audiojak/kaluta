import SwiftUI

/// A Teach Kaluta item (spec §14.13): the change it makes, large, with
/// its cues; its evidence (the AI's drafts beside what was sent, the
/// messages or the words in the user's mail, the conversations moved
/// back) on O.
struct TeachItemView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let item: WorkflowItem

    var body: some View {
        let t = model.teachItem(item)
        let evidence = session.showsInFull
        VStack(alignment: .leading, spacing: Space.xxl) {
            switch t {
            case let .routine(p): RoutineDecisionBody(proposal: p, evidence: evidence)
            case let .forward(p):
                ForwardDecisionBody(proposal: p, evidence: evidence)
                if session.choosingTime == .forwardMode {
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Forward automatically, or ask each time?").font(TypeRole.workflowHeading)
                        Text("1  \(ForwardMode.automatic.explanation)")
                        Text("2  \(ForwardMode.askEachTime.explanation)")
                    }
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
                    .id(WorkflowWriteView.scrollID)
                }
            case let .rule(entry): RuleDecisionBody(entry: entry, evidence: evidence)
            case let .proposal(p): ProposalDecisionBody(proposal: p, evidence: evidence)
            case let .fact(p): FactDecisionBody(proposal: p, evidence: evidence)
            case nil:
                Text("Decided on its page meanwhile.")
                    .font(TypeRole.workflowContent)
                    .foregroundStyle(.secondary)
            }
            if let t, AppModel.hasEvidence(t), !evidence {
                HStack(spacing: Space.s) {
                    Image(systemName: "text.magnifyingglass")
                    Text(Self.evidenceHint(t))
                    Keycap(key: "O")
                }
                .font(TypeRole.workflowContext)
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("\(Self.evidenceHint(t)): Show the evidence")
            }
        }
        .padding(Space.xxxl)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
        // Edit… on a proposed forwarding rule (spec §14.15).
        .sheet(item: Binding(get: { model.forwardRuleSheet }, set: { model.forwardRuleSheet = $0 })) { sheet in
            ForwardRuleEditor(sheet: sheet) { model.forwardRuleSheet = nil }
        }
    }

    /// What O shows, for the item's kind.
    static func evidenceHint(_ t: AppModel.TeachItem) -> String {
        switch t {
        case .routine: "The conversations you moved back"
        case .forward: "The emails you forwarded"
        case .rule: "The messages that show it"
        case .proposal: "Kaluta's drafts beside what you sent"
        case .fact: "Where your mail says so"
        }
    }
}
