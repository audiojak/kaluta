import SwiftUI

extension AnalysisFactProposalInfo {
    var symbol: String {
        switch (kind, op) {
        case ("category", _): "folder.badge.plus"
        case ("starter", _): "square.stack.3d.up"
        case (_, .edit): "pencil.circle"
        case (_, .remove): "minus.circle"
        default: "plus.circle"
        }
    }

    /// "Occupation or role: CTO", "New category: Sailing".
    var headline: String {
        switch kind {
        case "category": "New category: \(name)"
        case "starter": "Add the \(starterName) categories"
        default: op == .remove ? "Remove \(label)" : "\(label): \(value)"
        }
    }

    var subtitle: String {
        switch kind {
        // "New category" when none of the facts in Other would move.
        case "category": factCount == 0 ? "New category" : factCount == 1 ? "1 fact from Other" : "\(factCount) facts from Other"
        case "starter": "Your mail keeps showing facts that fit them"
        default:
            switch op {
            case .edit: "\(categoryName) · was \(beforeValue ?? "")"
            case .remove: "\(categoryName) · your mail no longer says so"
            default: "New fact · \(categoryName)"
            }
        }
    }

    /// A fact to add or change, so the user says how freely drafts use it.
    var takesUse: Bool { kind == "fact" && op != .remove }

    var starterName: String {
        switch starter {
        case "business": "Business"
        case "freelance": "Freelance or consulting"
        case "household": "Household"
        default: "Job search"
        }
    }
}

/// A proposed fact as a card in the review flow: what it would say, the
/// words in the user's mail, how freely drafts may use it, and its actions.
struct ProposedFactCard: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo
    let isCurrent: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Label(proposal.subtitle, systemImage: proposal.symbol).font(TypeRole.caption).foregroundStyle(.secondary)
            Text(proposal.headline).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if proposal.kind == "category", !proposal.description.isEmpty {
                Text(proposal.description).foregroundStyle(.secondary)
            }
            if !proposal.quote.isEmpty {
                Text("“\(proposal.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary).lineLimit(3)
            }
            if proposal.takesUse {
                FactUsePicker(proposal: proposal)
            }
            HStack(spacing: Space.m) {
                Button("Accept") { Task { await model.decideProposedFacts([proposal], accept: true) } }
                    .defaultAction(isCurrent)
                    .hoverHelp("Add it to your facts (Return); Undo takes it back")
                Button("Reject") { Task { await model.decideProposedFacts([proposal], accept: false) } } // undoable
                    .hoverHelp("Leave it out; it will not be proposed again (⌫)")
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(isCurrent ? .attention : .neutral)
        .accessibilityElement(children: .contain)
    }
}

/// The chosen proposed fact, in the detail (spec §14.11): its card, with
/// the keys that decide it. The list's Waiting section is the flow.
struct ProposedFactDetail: View {
    let proposal: AnalysisFactProposalInfo

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                ProposedFactCard(proposal: proposal, isCurrent: true)
                Text("Return accepts, ⌫ rejects; each can be undone. The next one waiting is chosen for you.")
                    .font(TypeRole.caption).foregroundStyle(.secondary)
            }
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private static let readingWidth: CGFloat = 720
}

/// How freely drafts may use a proposed fact once it is accepted (spec
/// §14.11), preset to its category's default.
struct FactUsePicker: View {
    @Environment(AppModel.self) private var model
    let proposal: AnalysisFactProposalInfo
    /// A pop-up menu for a row; segments in the detail.
    var compact = false

    var body: some View {
        let picker = Picker("Drafts", selection: Binding(get: { model.analysis.use(of: proposal) },
                                                         set: { model.analysis.factUses[proposal.id] = $0 })) {
            ForEach([FactUse.free, .ask, .never], id: \.self) { use in Text(use.title).tag(use) }
        }
        .fixedSize()
        .hoverHelp("Once accepted: whether AI drafts may use it, ask you first, or never see it")
        if compact {
            picker.labelsHidden().pickerStyle(.menu)
        } else {
            picker.pickerStyle(.segmented)
        }
    }
}

/// The Facts page's detail: the chosen proposed fact's card, or the
/// chosen fact.
struct FactsDetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let proposal = model.selectedFactProposal {
            ProposedFactDetail(proposal: proposal)
        } else if let fact = model.facts.selected {
            FactDetail(fact: fact, store: model.facts) { model.guideSheet = .fact($0, category: nil) }
        } else {
            ContentUnavailableView("No Fact Selected", systemImage: "person.text.rectangle")
        }
    }
}
