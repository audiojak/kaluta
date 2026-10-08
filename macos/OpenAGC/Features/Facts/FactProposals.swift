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

/// The Facts page's detail: the chosen fact.
struct FactsDetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let fact = model.facts.selected {
            FactDetail(fact: fact, store: model.facts) { model.guideSheet = .fact($0, category: nil) }
        } else {
            ContentUnavailableView("No Fact Selected", systemImage: "person.text.rectangle",
                                   description: Text("Facts about you that AI drafts may use: your role, time zone, calendar link, the people you mention. Each day's review finds new ones in the mail you send."))
        }
    }
}
