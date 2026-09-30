import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Merge a Guide… (spec §14.9): from another account in the app or an
/// exported file. Identical entries are skipped; the rest is listed to add,
/// and where the guides differ each point is a decision: keep mine, take
/// theirs, or keep both with theirs for one audience. Nothing changes until
/// Merge; then it is one undoable change and a new version.
struct MergeGuideSheet: View {
    @Environment(AppModel.self) private var model
    @State private var fromAccount: String?
    @State private var file: (name: String, json: String)?
    @State private var plan: GuideMergePlan?
    @State private var skipped: Set<Int> = []
    @State private var choices: [Int: Choice] = [:]
    @State private var addGroups = true
    @State private var working = false
    @State private var error: String?

    enum Choice: Hashable {
        case mine, theirs, both(String)
    }

    var body: some View {
        Dialog(title: "Merge a Writing Guide", message: plan == nil
               ? "Bring rules and guidelines in from another account, or from a guide you exported. Quotes from mail never travel."
               : "From \(plan!.origin). Choose what to keep.", width: Self.width) {
            if let plan {
                review(plan)
            } else {
                source
            }
            if working {
                HStack(spacing: Space.s) {
                    ProgressView().controlSize(.small)
                    Text("Comparing with \(model.agent.providerName)…").foregroundStyle(.secondary)
                }
                .font(TypeRole.meta)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } buttons: {
            CancelButton(help: "Close without merging (Esc)") { model.guideSheet = nil }
            if let plan {
                Button("Merge") { Task { await merge(plan) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(choices.count < plan.decisions.count || working)
                    .hoverHelp("Merge as chosen; Undo reverses it (Return)")
            } else {
                Button("Compare") { Task { await compare() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(working || (fromAccount == nil && file == nil))
                    .hoverHelp("See what merging would change before anything does (Return)")
            }
        }
    }

    @ViewBuilder private var source: some View {
        let others = model.accounts.filter { $0.id != model.openAccountID }
        Picker("From account", selection: Binding(get: { fromAccount }, set: { fromAccount = $0; if $0 != nil { file = nil } })) {
            Text("Choose…").tag(String?.none)
            ForEach(others, id: \.id) { a in Text(a.email).tag(String?.some(a.id)) }
        }
        .disabled(others.isEmpty)
        .hoverHelp("Another account's writing guide")
        HStack(spacing: Space.m) {
            Button("Choose a File…") { chooseFile() }
                .hoverHelp("A guide exported with Export for Another Account")
            if let file { Text(file.name).foregroundStyle(.secondary) }
        }
    }

    private func review(_ plan: GuideMergePlan) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                if plan.identical > 0 {
                    Text("\(plan.identical) entries are already in your guide and are skipped.")
                        .font(TypeRole.meta).foregroundStyle(.secondary)
                }
                if !plan.additions.isEmpty {
                    Text("New to your guide").font(TypeRole.groupLabel)
                    ForEach(Array(plan.additions.enumerated()), id: \.offset) { i, e in
                        Toggle(isOn: Binding(get: { !skipped.contains(i) },
                                             set: { if $0 { skipped.remove(i) } else { skipped.insert(i) } })) {
                            Text("\(e.category) · \(e.statement)").fixedSize(horizontal: false, vertical: true)
                        }
                        .hoverHelp("Add this entry")
                    }
                }
                if !plan.decisions.isEmpty {
                    Text("Where the guides differ").font(TypeRole.groupLabel)
                    ForEach(Array(plan.decisions.enumerated()), id: \.offset) { i, d in decision(i, d) }
                }
                if !plan.groups.isEmpty {
                    Toggle("Add their audiences: \(plan.groups.map(\.name).joined(separator: ", "))", isOn: $addGroups)
                        .hoverHelp("Add the audience groups the other guide uses")
                }
            }
        }
        .frame(maxHeight: Self.listHeight)
    }

    private func decision(_ i: Int, _ d: GuideMergeDecision) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(d.point).font(TypeRole.heading)
            if !d.summary.isEmpty { Text(d.summary).font(TypeRole.meta).foregroundStyle(.secondary) }
            Grid(alignment: .leading, horizontalSpacing: Space.m, verticalSpacing: Space.xs) {
                GridRow {
                    Text("Yours").foregroundStyle(.secondary)
                    Text(d.mine.isEmpty ? "nothing" : d.mine.map(\.statement).joined(separator: "; "))
                }
                GridRow {
                    Text("Theirs").foregroundStyle(.secondary)
                    Text(d.incoming.map(\.statement).joined(separator: "; "))
                }
            }
            .font(TypeRole.meta)
            Picker("Keep", selection: Binding(get: { choices[i] }, set: { choices[i] = $0 })) {
                Text("Keep Mine").tag(Choice?.some(.mine))
                Text("Take Theirs").tag(Choice?.some(.theirs))
                ForEach(audiences, id: \.self) { a in Text("Both, Theirs for \(a)").tag(Choice?.some(.both(a))) }
            }
            .fixedSize()
            .hoverHelp("Which guide wins on this point")
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(choices[i] == nil ? .attention : .neutral)
    }

    private var audiences: [String] {
        let mine = model.guide.groups.filter { $0.status == .confirmed }.map(\.name)
        let theirs = addGroups ? (plan?.groups.map(\.name) ?? []) : []
        return Array(Set(mine + theirs)).sorted()
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url, let text = try? String(contentsOf: url, encoding: .utf8) else { return }
        file = (url.lastPathComponent, text)
        fromAccount = nil
    }

    private func compare() async {
        guard let core = model.core else { return }
        working = true
        defer { working = false }
        do {
            plan = try await core.planGuideMerge(fromAccount: fromAccount, json: file?.json, agent: model.agent.providerID)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func merge(_ plan: GuideMergePlan) async {
        guard let core = model.core else { return }
        if addGroups {
            for g in plan.groups { _ = try? await core.saveAudienceGroup(g) }
        }
        let origin = plan.origin
        var edits: [GuideEdit] = plan.additions.enumerated().filter { !skipped.contains($0.offset) }
            .map { .add(fields: $0.element, status: .accepted, source: .merged, origin: origin) }
        for (i, d) in plan.decisions.enumerated() {
            switch choices[i] {
            case .theirs:
                edits += d.mine.map { .delete(id: $0.id) }
                edits += d.incoming.map { .add(fields: $0, status: .accepted, source: .merged, origin: origin) }
            case let .both(audience):
                edits += d.incoming.map { fields in
                    var f = fields
                    f.scope.groups = [audience]
                    return .add(fields: f, status: .accepted, source: .merged, origin: origin)
                }
            case .mine, nil:
                break
            }
        }
        guard !edits.isEmpty else { model.guideSheet = nil; return }
        let result = await model.applyGuideEdits(edits, reason: "merge", actionName: "Merge Guide",
                                                 notice: "Merged a writing guide from \(origin)")
        switch result {
        case .success: model.guideSheet = nil
        case let .failure(e): error = e.message
        }
    }

    private static let width: CGFloat = 620
    private static let listHeight: CGFloat = 460
}
