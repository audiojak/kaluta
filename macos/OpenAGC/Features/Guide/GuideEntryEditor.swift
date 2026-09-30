import SwiftUI

/// Add or change an entry by hand (spec §14.9): its category, kind,
/// statement, scope and check. Saving is one undoable change.
struct GuideEntryEditor: View {
    @Environment(AppModel.self) private var model
    let entry: GuideEntry?
    @State var category: String
    @State private var kind: GuideKind = .guideline
    @State private var statement = ""
    @State private var groups: Set<String> = []
    @State private var people = ""
    @State private var types: Set<String> = []
    @State private var checkKind: GuideCheckKind?
    @State private var checkValue = ""
    @State private var error: String?
    @FocusState private var focused: Bool

    init(entry: GuideEntry?, category: String) {
        self.entry = entry
        _category = State(initialValue: entry?.category ?? category)
        _kind = State(initialValue: entry?.kind ?? .guideline)
        _statement = State(initialValue: entry?.statement ?? "")
        _groups = State(initialValue: Set(entry?.scope.groups ?? []))
        _people = State(initialValue: (entry?.scope.people ?? []).joined(separator: ", "))
        _types = State(initialValue: Set(entry?.scope.messageTypes ?? []))
        _checkKind = State(initialValue: entry?.check?.kind)
        _checkValue = State(initialValue: entry?.check?.value ?? "")
    }

    var body: some View {
        Dialog(title: entry == nil ? "New Entry" : "Edit Entry",
               message: "Write it as an instruction to someone drafting for you.") {
            Picker("Category", selection: $category) {
                ForEach(model.guide.categories, id: \.id) { c in Text("\(c.id) \(c.name)").tag(c.id) }
            }
            .hoverHelp("Which part of the guide it belongs to")
            Picker("Kind", selection: $kind) {
                Text("Rule").tag(GuideKind.rule)
                Text("Guideline").tag(GuideKind.guideline)
                Text("Fact").tag(GuideKind.fact)
            }
            .pickerStyle(.segmented)
            .hoverHelp("A rule always holds; a guideline is how you usually write; a fact is true about you")
            TextField("Sign off with 'John'", text: $statement, axis: .vertical)
                .lineLimit(2...4)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
            scope
            HStack(spacing: Space.m) {
                Picker("Check drafts", selection: $checkKind) {
                    Text("No check").tag(GuideCheckKind?.none)
                    Text("Never say").tag(GuideCheckKind?.some(.bannedPhrase))
                    Text("Always say").tag(GuideCheckKind?.some(.requiredPhrase))
                    Text("At most words").tag(GuideCheckKind?.some(.maxWords))
                }
                .fixedSize()
                .hoverHelp("A check the app runs on every draft an AI writes")
                if checkKind != nil {
                    TextField(checkKind == .maxWords ? "150" : "circle back", text: $checkValue)
                        .textFieldStyle(.roundedBorder)
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without saving (Esc)") { model.guideSheet = nil }
            Button("Save") { Task { await save() } }
                .keyboardShortcut(.defaultAction)
                .disabled(statement.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .hoverHelp("Save it to the guide (Return)")
        }
        .onAppear { focused = true }
    }

    @ViewBuilder private var scope: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text("Applies to").font(TypeRole.groupLabel)
            let confirmed = model.guide.groups.filter { $0.status == .confirmed }
            if !confirmed.isEmpty {
                ChipFlow(spacing: Space.xs) {
                    ForEach(confirmed, id: \.id) { g in
                        Toggle(g.name, isOn: Binding(get: { groups.contains(g.name) },
                                                     set: { if $0 { groups.insert(g.name) } else { groups.remove(g.name) } }))
                            .toggleStyle(.button)
                            .controlSize(.small)
                            .hoverHelp("Only when writing to \(g.name)")
                    }
                }
            }
            HStack(spacing: Space.m) {
                ForEach([("new", "New"), ("reply", "Replies"), ("forward", "Forwards")], id: \.0) { value, title in
                    Toggle(title, isOn: Binding(get: { types.contains(value) },
                                                set: { if $0 { types.insert(value) } else { types.remove(value) } }))
                        .hoverHelp("Only in \(title.lowercased())")
                }
            }
            TextField("People or @domains (optional)", text: $people)
                .textFieldStyle(.roundedBorder)
            Text(groups.isEmpty && types.isEmpty && people.isEmpty ? "Always" : "Only where it matches all you chose")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func save() async {
        let scope = GuideScope(groups: Array(groups).sorted(),
                               people: people.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
                                   .filter { !$0.isEmpty },
                               messageTypes: ["new", "reply", "forward"].filter(types.contains), languages: [])
        let check = checkKind.map { GuideCheck(kind: $0, value: checkValue) }
        let fields = GuideEntryFields(category: category, kind: kind, statement: statement, scope: scope, check: check)
        var edits: [GuideEdit] = [entry.map { .update(id: $0.id, fields: fields) }
            ?? .add(fields: fields, status: .accepted, source: .you, origin: nil)]
        // Editing a proposal (from the decisions) accepts it as edited.
        if let entry, entry.status == .proposed { edits.append(.decide(id: entry.id, status: .accepted)) }
        let result = await model.applyGuideEdits(edits, reason: entry == nil ? "add" : "edit",
                                                 actionName: entry == nil ? "Add Entry" : "Edit Entry",
                                                 notice: entry == nil ? "Added to your writing guide" : "Changed the entry")
        switch result {
        case .success:
            model.guideSheet = nil
        case let .failure(e):
            error = e.message
        }
    }
}
