import SwiftUI

/// The Facts tab's list (spec §14.11), in Analysis or (global facts) in
/// Settings: facts by category, built-ins first then the user's own, a
/// globe on global ones.
struct FactsList: View {
    @Environment(AppModel.self) private var model
    @Bindable var store: FactsStore

    var body: some View {
        List(selection: $store.selection) {
            ForEach(store.sections, id: \.category.key) { section in
                Section {
                    // Global and account ids overlap: rows are told apart by tag.
                    ForEach(section.facts, id: \.listTag) { fact in
                        FactRow(fact: fact).tag(FactsStore.tag(fact))
                    }
                } header: {
                    Text(section.category.name)
                }
            }
        }
        .listStyle(.inset)
        .overlay {
            if store.loaded, store.sections.isEmpty {
                ContentUnavailableView("No Facts Yet", systemImage: "person.text.rectangle",
                                       description: Text("Facts about you that AI drafts may use: your role, time zone, calendar link, the people you mention."))
            }
        }
        .onDeleteCommand {
            if let fact = store.selected { Task { await model.deleteFact(fact) } }
        }
        .task(id: model.factsRevision) { await store.load() }
    }
}

private struct FactRow: View {
    let fact: FactInfo

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(fact.label).font(TypeRole.caption).foregroundStyle(.secondary)
                Text(fact.value).lineLimit(2)
            }
            Spacer(minLength: Space.m)
            if fact.stale {
                Image(systemName: "clock.badge.exclamationmark").foregroundStyle(Tone.caution)
                    .accessibilityLabel("May be out of date")
            }
            if fact.use == .ask {
                Image(systemName: "hand.raised").foregroundStyle(.secondary).accessibilityLabel("Ask before using")
            } else if fact.use == .never {
                Image(systemName: "eye.slash").foregroundStyle(.secondary).accessibilityLabel("Never share")
            }
            if fact.scope == .global {
                Image(systemName: "globe").foregroundStyle(fact.overridden ? .tertiary : .secondary)
                    .accessibilityLabel(fact.overridden ? "Global, replaced here by this account's own" : "Global")
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The chosen fact: what it says, where it came from, and its actions.
struct FactDetail: View {
    @Environment(AppModel.self) private var model
    let fact: FactInfo
    let store: FactsStore
    var onEdit: (FactInfo) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.l) {
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text(store.name(of: fact.category)).font(TypeRole.caption).foregroundStyle(.secondary)
                    Text(fact.label).font(TypeRole.heading)
                    Text(fact.value).font(TypeRole.title).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }
                if fact.overridden {
                    Label("This account has its own “\(fact.label)”, which drafts use instead.", systemImage: "globe")
                        .font(TypeRole.meta).foregroundStyle(.secondary)
                }
                Picker("Drafts", selection: Binding(get: { fact.use }, set: { use in
                    Task {
                        await model.applyFactEdits([.update(id: fact.id, fields: FactFields(
                            category: fact.category, label: fact.label, value: fact.value, use: use, asOf: fact.asOf))],
                            scope: fact.scope, actionName: "Change Use", notice: "“\(fact.label)”: \(use.title.lowercased())")
                    }
                })) {
                    ForEach([FactUse.free, .ask, .never], id: \.self) { use in Text(use.title).tag(use) }
                }
                .pickerStyle(.segmented)
                .fixedSize()
                .hoverHelp("Whether AI drafts may use it, ask you first, or never see it")
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text(fact.scope == .global ? "Every account · \(fact.source.title)" : "This account · \(fact.source.title)")
                    if let asOf = fact.asOf {
                        Text("As of \(Date(timeIntervalSince1970: TimeInterval(asOf) / 1000).formatted(date: .abbreviated, time: .omitted))\(fact.stale ? ": may be out of date" : "")")
                            .foregroundStyle(fact.stale ? Tone.caution : .secondary)
                    }
                }
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
                ForEach(Array(fact.evidence.prefix(3).enumerated()), id: \.offset) { _, q in
                    Text("“\(q.quote)”").font(TypeRole.meta).italic().foregroundStyle(.secondary).lineLimit(3)
                }
                HStack(spacing: Space.m) {
                    Button("Edit…") { onEdit(fact) }
                        .hoverHelp("Change the label, value, category or date")
                    if store.scope == .account {
                        Button(fact.scope == .account ? "Make Global" : "Make This Account's Only") {
                            Task { await model.moveFact(fact) }
                        }
                        .hoverHelp(fact.scope == .account ? "Every account uses it; listed in Settings › Facts"
                                   : "Only this account uses it")
                    }
                    Button("Delete") { Task { await model.deleteFact(fact) } } // undoable
                        .hoverHelp("Delete the fact (⌫); Undo brings it back")
                }
                .controlSize(.small)
            }
            .padding(Space.xxl)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private static let readingWidth: CGFloat = 720
}

/// Add or change a fact (spec §14.11). Saving is one undoable change.
struct FactEditor: View {
    @Environment(AppModel.self) private var model
    let fact: FactInfo?
    let scope: FactScope
    let categories: [FactCategoryInfo]
    var onClose: () -> Void
    @State private var category: String
    @State private var label: String
    @State private var value: String
    @State private var use: FactUse
    @State private var dated: Bool
    @State private var asOf: Date
    @State private var error: String?
    @FocusState private var focused: Bool

    init(fact: FactInfo?, scope: FactScope, categories: [FactCategoryInfo], category: String? = nil,
         onClose: @escaping () -> Void) {
        self.fact = fact
        self.scope = scope
        self.categories = categories
        self.onClose = onClose
        let key = fact?.category ?? category ?? categories.first?.key ?? "other"
        _category = State(initialValue: key)
        _label = State(initialValue: fact?.label ?? "")
        _value = State(initialValue: fact?.value ?? "")
        _use = State(initialValue: fact?.use ?? categories.first { $0.key == key }?.defaultUse ?? .free)
        _dated = State(initialValue: fact?.asOf != nil)
        _asOf = State(initialValue: fact?.asOf.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) } ?? Date())
    }

    private var suggestions: [String] { categories.first { $0.key == category }?.suggestedLabels ?? [] }

    var body: some View {
        Dialog(title: fact == nil ? "New Fact" : "Edit Fact",
               message: scope == .global ? "Every account's AI drafts may use it." : "AI drafts for this account may use it.") {
            Picker("Category", selection: $category) {
                ForEach(categories.filter { !$0.hidden }, id: \.key) { c in Text(c.name).tag(c.key) }
            }
            .hoverHelp("Where it belongs; a category's description says what goes there")
            HStack(spacing: Space.s) {
                TextField("Label, such as Time zone", text: $label)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                if !suggestions.isEmpty {
                    Menu {
                        ForEach(suggestions, id: \.self) { s in Button(s) { label = s } } // no-help: menu
                    } label: {
                        Label("Suggestions", systemImage: "list.bullet")
                    }
                    .labelStyle(.iconOnly)
                    .fixedSize()
                    .hoverHelp("Labels this category suggests")
                }
            }
            TextField("Value", text: $value, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.roundedBorder)
            Picker("Drafts", selection: $use) {
                ForEach([FactUse.free, .ask, .never], id: \.self) { u in Text(u.title).tag(u) }
            }
            .pickerStyle(.segmented)
            .hoverHelp("Whether AI drafts may use it, ask you first, or never see it")
            HStack(spacing: Space.m) {
                Toggle("True as of", isOn: $dated)
                    .hoverHelp("For facts that age, such as travel dates; old ones are flagged for review")
                if dated {
                    DatePicker("", selection: $asOf, displayedComponents: .date).labelsHidden()
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without saving (Esc)") { onClose() }
            Button("Save") { Task { await save() } }
                .keyboardShortcut(.defaultAction)
                .disabled(label.trimmingCharacters(in: .whitespaces).isEmpty || value.trimmingCharacters(in: .whitespaces).isEmpty)
                .hoverHelp("Save the fact (Return)")
        }
        .onAppear { focused = true }
    }

    private func save() async {
        let fields = FactFields(category: category, label: label, value: value, use: use,
                                asOf: dated ? Int64(asOf.timeIntervalSince1970 * 1000) : nil)
        let edit: FactEdit = fact.map { .update(id: $0.id, fields: fields) }
            ?? .add(fields: fields, status: .accepted, source: .you)
        if let failure = await model.applyFactEdits([edit], scope: scope, actionName: fact == nil ? "Add Fact" : "Edit Fact",
                                                    notice: fact == nil ? "Added “\(label)”" : "Changed “\(label)”") {
            error = failure.message
        } else {
            onClose()
        }
    }
}

/// A new category of the user's own, offering a close one instead.
struct NewFactCategorySheet: View {
    @Environment(AppModel.self) private var model
    let scope: FactScope
    var onClose: () -> Void
    @State private var name = ""
    @State private var description = ""
    @State private var similar: FactCategoryInfo?
    @State private var error: String?

    var body: some View {
        Dialog(title: "New Category",
               message: "Its description tells drafting, and what learns facts from your mail, what belongs there.") {
            TextField("Name, such as Properties", text: $name)
                .textFieldStyle(.roundedBorder)
                .onChange(of: name) { Task { similar = try? await model.core?.similarFactCategory(name) } }
            TextField("Description, such as Properties I'm currently selling", text: $description, axis: .vertical)
                .lineLimit(1...3)
                .textFieldStyle(.roundedBorder)
            if let similar, !name.isEmpty {
                Label("You have “\(similar.name)” already; use it instead.", systemImage: "info.circle")
                    .font(TypeRole.meta).foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without adding (Esc)") { onClose() }
            Button("Add") {
                Task {
                    if let failure = await model.editFactCategories([.add(name: name, description: description)], scope: scope,
                                                                    actionName: "Add Category", notice: "Added “\(name)”") {
                        error = failure.message
                    } else {
                        onClose()
                    }
                }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || similar != nil)
            .hoverHelp("Add the category (Return)")
        }
    }
}

/// Every category: hide built-ins, rename or delete the user's own.
struct FactCategoriesSheet: View {
    @Environment(AppModel.self) private var model
    let store: FactsStore
    var onClose: () -> Void
    @State private var editing: String?
    @State private var name = ""
    @State private var description = ""

    var body: some View {
        Dialog(title: "Categories", message: "Built-in categories can be hidden. Deleting one of yours moves its facts to Other; Undo brings it back.") {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.s) {
                    ForEach(store.categories, id: \.key) { c in
                        row(c)
                    }
                }
            }
            .frame(minHeight: Self.listHeight)
        } buttons: {
            Button("Done") { onClose() }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Close (Return)")
        }
    }

    @ViewBuilder private func row(_ c: FactCategoryInfo) -> some View {
        if editing == c.key {
            VStack(alignment: .leading, spacing: Space.xs) {
                TextField("Name", text: $name).textFieldStyle(.roundedBorder)
                TextField("Description", text: $description).textFieldStyle(.roundedBorder)
                HStack {
                    Button("Save") {
                        Task {
                            await model.editFactCategories([.update(key: c.key, name: name, description: description)],
                                                           scope: store.scope, actionName: "Rename Category",
                                                           notice: "Renamed “\(c.name)”")
                            editing = nil
                            await store.load()
                        }
                    }
                    .hoverHelp("Save the name and description")
                    Button("Cancel") { editing = nil } // inline
                        .hoverHelp("Keep it as it was")
                }
                .controlSize(.small)
            }
        } else {
            HStack(spacing: Space.m) {
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text(c.name)
                    if !c.description.isEmpty {
                        Text(c.description).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: Space.m)
                if c.builtin {
                    Toggle("Shown", isOn: Binding(get: { !c.hidden }, set: { shown in
                        Task {
                            await model.editFactCategories([.hide(key: c.key, hidden: !shown)], scope: store.scope,
                                                           actionName: shown ? "Show Category" : "Hide Category",
                                                           notice: shown ? "Showing \(c.name)" : "Hid \(c.name)")
                            await store.load()
                        }
                    }))
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                    .hoverHelp("Hide a built-in category you do not need")
                } else {
                    Button("Rename…") {
                        name = c.name
                        description = c.description
                        editing = c.key
                    }
                    .hoverHelp("Change its name and description")
                    Button("Delete") {
                        Task {
                            await model.editFactCategories([.delete(key: c.key)], scope: store.scope, actionName: "Delete Category",
                                                           notice: "Deleted “\(c.name)”; its facts are in Other")
                            await store.load()
                        }
                    } // undoable
                    .hoverHelp("Delete it; its facts move to Other")
                }
            }
            .controlSize(.small)
        }
    }

    private static let listHeight: CGFloat = 280
}
