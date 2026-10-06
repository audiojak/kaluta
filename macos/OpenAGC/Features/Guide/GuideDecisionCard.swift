import SwiftUI

/// A learning decision (spec §14.9) in the review flow: what
/// the learning run proposed, with the quotes from the user's mail behind
/// it. Accept (↩), edit (e) or reject (⌫), each undoable. A proposal that
/// mail contradicts an accepted entry offers to use it instead, keep the
/// entry, or edit the entry.
struct GuideDecisionCard: View {
    @Environment(AppModel.self) private var model
    let entry: GuideEntry
    let isCurrent: Bool
    /// The accepted entry this proposal's mail goes against.
    let accepted: GuideEntry?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.s) {
                Text(categoryName).font(TypeRole.caption).foregroundStyle(.secondary)
                GuideKindChip(kind: entry.kind)
                if !entry.scope.isAlways {
                    Text(entry.scope.text).font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
            if let accepted {
                Text("Your mail disagrees with: “\(accepted.statement)”")
                    .font(TypeRole.meta)
                    .foregroundStyle(Tone.caution)
            }
            Text(entry.statement).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if let check = entry.check {
                Label(check.summary, systemImage: "checkmark.shield").font(TypeRole.caption).foregroundStyle(.secondary)
            }
            Text("\(entry.support.formatted()) \(entry.support == 1 ? "message shows" : "messages show") this")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
            ForEach(Array(entry.evidence.filter { !$0.contradicts }.prefix(3).enumerated()), id: \.offset) { _, q in
                Text("“\(q.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary).lineLimit(3)
            }
            HStack(spacing: Space.m) {
                if let accepted {
                    Button("Use This Instead") { Task { await model.acceptDecision(entry) } }
                        .defaultAction(isCurrent)
                        .hoverHelp("Replace your entry with this one (Return)")
                    Button("Keep Mine") { Task { await model.decideProposedRule(reject: entry) } }
                        .hoverHelp("Keep your entry as it is (⌫)")
                    Button("Edit Mine…") { model.guideSheet = .edit(accepted, category: accepted.category) }
                        .hoverHelp("Narrow or change your entry")
                } else {
                    Button("Accept") { Task { await model.acceptDecision(entry) } }
                        .defaultAction(isCurrent)
                        .hoverHelp("Add it to your guide (Return)")
                    Button("Edit…") { model.guideSheet = .edit(entry, category: entry.category) }
                        .hoverHelp("Change it before adding it (e)")
                    Button("Reject") { Task { await model.decideProposedRule(reject: entry) } } // undoable
                        .hoverHelp("Leave it out; it will not be proposed again (⌫)")
                }
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(isCurrent ? .attention : .neutral)
        .accessibilityElement(children: .contain)
    }

    private var categoryName: String {
        let name = model.guide.categories.first { $0.id == entry.category }?.name ?? ""
        return "\(entry.category) \(name)"
    }
}

extension AppModel {
    /// Accept or reject a proposal; undoable.
    func decideGuide(_ entry: GuideEntry, accept: Bool) async {
        await applyGuideEdits([.decide(id: entry.id, status: accept ? .accepted : .rejected)],
                              reason: accept ? "accept" : "reject", actionName: accept ? "Accept Entry" : "Reject Entry",
                              notice: accept ? "Added to your writing guide" : "Left out of your writing guide")
    }

    /// Use a proposal in place of the accepted entry it contradicts: one
    /// change, undoable.
    func replaceGuideEntry(_ old: GuideEntry, with new: GuideEntry) async {
        await applyGuideEdits([.decide(id: new.id, status: .accepted), .decide(id: old.id, status: .rejected)],
                              reason: "replace", actionName: "Replace Entry", notice: "Replaced “\(old.statement)”")
    }
}
