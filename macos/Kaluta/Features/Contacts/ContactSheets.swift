import SwiftUI

/// Add or change a fact, a topic or a commitment about a contact (spec
/// §14.16). Saving is one undoable change; what the user writes is theirs.
struct ContactFactEditor: View {
    @Environment(AppModel.self) private var model
    let item: ContactFactInfo?
    @State private var kind: ContactFactKind
    @State private var label: String
    @State private var value: String
    @State private var itemClass: ContactFactClass
    @State private var owedBy: ContactParty
    @State private var hasDue: Bool
    @State private var due: Date
    @State private var use: FactUse
    @State private var error: String?
    @FocusState private var focused: Bool

    init(item: ContactFactInfo?) {
        self.item = item
        _kind = State(initialValue: item?.kind ?? .fact)
        _label = State(initialValue: item?.label ?? "")
        _value = State(initialValue: item?.value ?? "")
        _itemClass = State(initialValue: item?.class ?? .other)
        _owedBy = State(initialValue: item?.owedBy ?? .you)
        _hasDue = State(initialValue: item?.due != nil)
        _due = State(initialValue: item?.due.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) } ?? Date())
        _use = State(initialValue: item?.use ?? .free)
    }

    private var tooLong: String? {
        if label.count > Self.maxLabel { return "A label is at most \(Self.maxLabel) characters." }
        if value.count > Self.maxValue { return "A value is at most \(Self.maxValue) characters." }
        return nil
    }

    var body: some View {
        Dialog(title: item == nil ? "New Note About \(model.contacts.contact?.name ?? "This Contact")" : "Edit",
               message: "Drafts to this person may use it. Nothing here goes into mail to anyone else.") {
            Picker("Kind", selection: $kind) {
                Text("Fact").tag(ContactFactKind.fact)
                Text("Topic").tag(ContactFactKind.topic)
                Text("Commitment").tag(ContactFactKind.commitment)
            }
            .pickerStyle(.segmented)
            .disabled(item != nil)
            .hoverHelp("A fact about them, something open between you, or something one of you owes the other")
            TextField(kind == .commitment ? "What is owed, such as Send the revised contract" : "Label, such as Role", text: $label)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
            TextField("Value", text: $value, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.roundedBorder)
            if kind == .commitment {
                Picker("Owed by", selection: $owedBy) {
                    Text("You").tag(ContactParty.you)
                    Text("Them").tag(ContactParty.them)
                }
                .pickerStyle(.segmented)
                .hoverHelp("Who owes it: you, or them")
                HStack(spacing: Space.m) {
                    Toggle("Due", isOn: $hasDue)
                        .hoverHelp("A day it is due, if one was agreed")
                    if hasDue { DatePicker("Due day", selection: $due, displayedComponents: .date).labelsHidden() }
                }
            }
            Picker("About", selection: $itemClass) {
                ForEach(ContactFactClass.allCases, id: \.self) { c in Text(c.title).tag(c) }
            }
            .hoverHelp("What it concerns; facts are grouped by this on the page")
            Picker("Drafts", selection: $use) {
                ForEach([FactUse.free, .ask, .never], id: \.self) { u in Text(u.contactTitle).tag(u) }
            }
            .pickerStyle(.segmented)
            .hoverHelp("Whether drafts to this person may use it, ask you first, or never see it")
            if let message = error ?? tooLong {
                Label(message, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without saving (Esc)") { model.contacts.sheet = nil }
            Button("Save") { Task { await save() } }
                .keyboardShortcut(.defaultAction)
                .disabled(label.trimmingCharacters(in: .whitespaces).isEmpty
                    || value.trimmingCharacters(in: .whitespaces).isEmpty || tooLong != nil)
                .hoverHelp("Save it (Return)")
        }
        .onAppear { focused = true }
        .onChange(of: itemClass) { if item == nil, itemClass == .personal { use = .ask } }
    }

    private func save() async {
        guard let contact = model.contacts.contact else { return }
        let fields = ContactFactFields(kind: kind, label: label, value: value, class: itemClass,
                                       owedBy: kind == .commitment ? owedBy : nil,
                                       due: kind == .commitment && hasDue ? Int64(due.timeIntervalSince1970 * 1000) : nil,
                                       use: use, asOf: item?.asOf)
        let edit: ContactEdit = item.map { .update(id: $0.id, fields: fields) } ?? .add(personId: contact.personId, fields: fields)
        if let failure = await model.applyContactEdits([edit], actionName: item == nil ? "Add Note" : "Edit Note",
                                                       notice: item == nil ? "Added “\(label)”" : "Changed “\(label)”") {
            error = failure.message
        } else {
            model.contacts.sheet = nil
        }
    }

    private static let maxLabel = 60
    private static let maxValue = 160
}

/// *Add Tag*: tags from the account's list, found by typing or under their headings.
struct AddTagSheet: View {
    @Environment(AppModel.self) private var model
    @State private var error: String?

    var body: some View {
        Dialog(title: "Tags for \(model.contacts.contact?.name ?? "This Contact")",
               message: "What they are to you, and what they do. A role can take two: a VP of Engineering is VP and Engineering.") {
            TagPicker(tags: model.contacts.tags, chosen: model.contacts.contact?.tags.map(\.id) ?? [],
                      title: "Click a tag to add it, or again to take it off") { id in
                Task { await toggle(id) }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            Button("Done") { model.contacts.sheet = nil }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Close (Return)")
        }
    }

    private func toggle(_ id: String) async {
        guard let contact = model.contacts.contact, let tag = model.contacts.tags.first(where: { $0.id == id }) else { return }
        let has = contact.tags.contains { $0.id == id }
        let edit: ContactEdit = has ? .removeTag(personId: contact.personId, tagId: id) : .addTag(personId: contact.personId, tagId: id)
        error = await model.applyContactEdits([edit], actionName: has ? "Remove Tag" : "Add Tag",
                                              notice: has ? "Removed “\(tag.name)”" : "Tagged \(contact.name) “\(tag.name)”")?.message
    }
}

/// *Same Person As…*: join this contact's addresses and items to another.
/// The user's act: Kaluta never joins two on its own (a display name is
/// whatever the sender typed).
struct SamePersonSheet: View {
    @Environment(AppModel.self) private var model
    @State private var typed = ""
    @State private var found: [ContactSummary] = []
    @State private var error: String?
    @FocusState private var focused: Bool

    var body: some View {
        Dialog(title: "Same Person As…",
               message: "Their addresses and what is known about them become one contact. Undo separates them again.") {
            TextField("A name or an address", text: $typed)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onChange(of: typed) { Task { await find() } }
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xs) {
                    ForEach(found, id: \.listTag) { other in
                        Button {
                            Task { await join(other) }
                        } label: {
                            VStack(alignment: .leading, spacing: Space.hair) {
                                Text(other.name)
                                Text(other.email).font(TypeRole.caption).foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                        .hoverHelp("Join with \(other.name)")
                    }
                    if found.isEmpty, !typed.isEmpty {
                        Text("Nobody found.").font(TypeRole.meta).foregroundStyle(.secondary)
                    }
                }
            }
            .frame(height: Self.listHeight)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton { model.contacts.sheet = nil }
        }
        .onAppear { focused = true }
    }

    private func find() async {
        let asked = typed
        guard let core = model.core, !asked.trimmingCharacters(in: .whitespaces).isEmpty else {
            found = []
            return
        }
        let rows = (try? await core.contacts(tagIDs: [], search: asked)) ?? []
        // Only the answer for what the field says now; never the contact itself.
        if asked == typed { found = rows.filter { $0.personId != model.contacts.contact?.personId } }
    }

    private func join(_ other: ContactSummary) async {
        guard let contact = model.contacts.contact, let core = model.core else { return }
        // A search result with no contact yet gets one first.
        var target = other
        if target.personId == nil {
            guard let made = try? await core.contact(forAddress: other.email) else {
                error = "That address is not a person's."
                return
            }
            target = ContactSummary(personId: made.personId, name: made.name, email: other.email, tags: made.tags,
                                    held: made.held, latest: other.latest)
        }
        if let failure = await model.joinContact(contact, into: target) {
            error = failure.message
        } else {
            model.contacts.sheet = nil
        }
    }

    private static let listHeight: CGFloat = 220
}

/// *Learn From All Our Mail*: says how many messages and agent turns it is
/// before anything is read (spec §14.16 *The deep dive*).
struct ContactDiveSheet: View {
    @Environment(AppModel.self) private var model
    @State private var maxMessages: UInt32 = 2_000
    @State private var estimate: ContactDiveEstimate?
    @State private var error: String?

    var body: some View {
        Dialog(title: "Learn From All Our Mail",
               message: "Your agent reads the mail between you and \(model.contacts.contact?.name ?? "this contact"), twenty messages a turn, oldest first. What it finds goes through the same review as new mail.") {
            Picker("Read", selection: $maxMessages) {
                Text("The newest 500").tag(UInt32(500))
                Text("The newest 2,000").tag(UInt32(2_000))
                Text("The newest 10,000").tag(UInt32(10_000))
            }
            .hoverHelp("How far back to read; older mail than this is left")
            if let estimate {
                Text(Self.line(estimate)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else {
                ProgressView().controlSize(.small)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton { model.contacts.sheet = nil }
            Button("Start") { Task { await start() } }
                .keyboardShortcut(.defaultAction)
                .disabled((estimate?.unread ?? 0) == 0)
                .hoverHelp("Start reading (Return); you can stop it from the page")
        }
        .task(id: maxMessages) {
            guard let id = model.contacts.contact?.personId else { return }
            estimate = try? await model.core?.contactDiveEstimate(id, maxMessages: maxMessages)
        }
    }

    static func line(_ e: ContactDiveEstimate) -> String {
        if e.unread == 0 {
            return e.messages == 0 ? "There is no mail between you yet." : "All \(e.messages) messages have been read already."
        }
        let already = e.messages > e.unread ? " (\(e.messages - e.unread) already read)" : ""
        return "\(e.unread) message\(e.unread == 1 ? "" : "s") to read\(already): about \(e.turns) agent turn\(e.turns == 1 ? "" : "s")."
    }

    private func start() async {
        guard let core = model.core, let id = model.contacts.contact?.personId else { return }
        do {
            _ = try await core.startContactDive(id, maxMessages: maxMessages)
            model.contacts.sheet = nil
            await model.contacts.load()
        } catch {
            self.error = error.message
        }
    }
}

/// Held items one at a time, each as a document: its kind and class, who
/// said it, the quote and why it was held. Return accepts, Delete rejects.
struct ContactReviewSheet: View {
    @Environment(AppModel.self) private var model
    /// The queue as it was when the sheet opened.
    let items: [ContactFactInfo]
    @State private var at = 0
    @State private var outcomes: [Int64: Bool] = [:]
    @State private var error: String?

    private var current: ContactFactInfo? { items.indices.contains(at) ? items[at] : nil }

    var body: some View {
        Dialog(title: "Needs Review", message: progress, width: Self.width) {
            if let item = current {
                ScrollView {
                    VStack(alignment: .leading, spacing: Space.l) {
                        HeldItemBody(item: item, changes: model.contacts.facts.first { $0.id == item.changesId })
                        if let outcome = outcomes[item.id] {
                            Label(outcome ? "Accepted" : "Rejected", systemImage: outcome ? "checkmark.circle" : "xmark.circle")
                                .foregroundStyle(outcome ? Tone.approved : .secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(height: Self.height)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } leading: {
            Button("Skip") { advance() }
                .disabled(at >= items.count - 1)
                .hoverHelp("Leave this one waiting and go to the next (→)")
                .keyboardShortcut(.rightArrow, modifiers: [])
        } buttons: {
            CancelButton(title: "Done", help: "Close; what you have not decided keeps waiting (Esc)") { model.contacts.sheet = nil }
            Button("Reject") { Task { await decide(false) } }
                .keyboardShortcut(.delete, modifiers: [])
                .disabled(current.map { outcomes[$0.id] != nil } ?? true)
                .hoverHelp("Leave it out; it will not be suggested again (Delete)")
            Button("Accept") { Task { await decide(true) } }
                .keyboardShortcut(.defaultAction)
                .disabled(current.map { outcomes[$0.id] != nil } ?? true)
                .hoverHelp("Keep it: drafts to this person may use it (Return)")
        }
    }

    private var progress: String {
        let left = items.count - outcomes.count
        return "\(min(at + 1, items.count)) of \(items.count) · \(left) still waiting. Nothing here is used in drafts until you accept it."
    }

    private func advance() {
        if let next = items.indices.first(where: { $0 > at && outcomes[items[$0].id] == nil }) { at = next }
    }

    private func decide(_ accept: Bool) async {
        guard let item = current else { return }
        if let failure = await model.decideContactFacts([item], accept: accept) {
            error = failure.message
            return
        }
        error = nil
        outcomes[item.id] = accept
        if outcomes.count == items.count {
            model.contacts.sheet = nil
        } else {
            advance()
        }
    }

    private static let width: CGFloat = 560
    private static let height: CGFloat = 320
}
