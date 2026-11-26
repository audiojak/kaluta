import SwiftUI

/// Learning Settings › Contacts (spec §14.16 *Settings*): the switch the
/// ask sets, the daily limit (none by default), what was read, *Learn
/// From Past Mail…* and the tag list.
struct ContactLearningSettings: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let store = model.contacts
        Section {
            if let learning = store.learning {
                if learning.unavailable {
                    Text("Agent mailboxes keep no notes about contacts.").foregroundStyle(.secondary)
                } else {
                    Toggle("Learn about contacts", isOn: Binding(get: { learning.on }, set: { on in
                        Task { await model.setContactLearning(on) }
                    }))
                    .hoverHelp("Read new mail from people you have written to, for facts, open topics and commitments. Off hides nothing already learned")
                    Picker("Conversations a day", selection: Binding(get: { learning.perDay }, set: { perDay in
                        Task {
                            try? await model.core?.setContactConversationsADay(perDay)
                            await store.load()
                        }
                    })) {
                        Text("No limit").tag(UInt32(0))
                        ForEach([10, 25, 50, 100] as [UInt32], id: \.self) { n in Text("\(n)").tag(n) }
                    }
                    .hoverHelp("One agent turn each. With a limit, the rest are read the next day, oldest first: nothing is skipped")
                    Text(Self.readLine(learning)).foregroundStyle(.secondary)
                    pastMail(store)
                    Button("Contact Tags…") { store.settingsSheet = .tags }
                        .hoverHelp("The tags that say what a contact is to you and what they do: add, rename or delete")
                }
            } else if model.openAccountID != nil {
                ProgressView().controlSize(.small)
            }
        } header: {
            Text("Contacts")
        } footer: {
            Text("Learning about contacts sends new mail in conversations with people you have written to, one conversation at a time, to your own agent. Anything about money, agreements or where to send things waits for you to accept it.")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private func pastMail(_ store: ContactsStore) -> some View {
        if let batch = store.batch, batch.state == "running" || batch.state == "paused" {
            VStack(alignment: .leading, spacing: Space.s) {
                Text(Self.batchLine(batch))
                ProgressView(value: Double(min(batch.donePeople, batch.totalPeople)), total: Double(max(batch.totalPeople, 1)))
                HStack(spacing: Space.m) {
                    Button(batch.state == "running" ? "Pause" : "Resume") {
                        Task {
                            try? await model.core?.setContactBatchState(batch.state == "running" ? "paused" : "running")
                            await store.load()
                        }
                    }
                    .hoverHelp(batch.state == "running" ? "Stop reading for now; Resume carries on where it was"
                               : "Carry on reading where it stopped")
                    Button("Stop") {
                        Task {
                            try? await model.core?.setContactBatchState("stopped")
                            await store.load()
                        }
                    }
                    .hoverHelp("Stop for good; what the finished people yielded stays")
                }
                .controlSize(.small)
            }
        } else {
            Button("Learn From Past Mail…") { store.settingsSheet = .pastMail }
                .hoverHelp("Read all your mail with everyone you wrote to in a number of days; says how much first")
        }
    }

    /// "Read today: 12 · yesterday: 30".
    static func readLine(_ learning: ContactLearning) -> String {
        "Read today: \(learning.readToday) · yesterday: \(learning.readYesterday)"
    }

    /// "12 of 85 people · Reese Okafor".
    static func batchLine(_ batch: ContactBatch) -> String {
        let people = "\(batch.donePeople) of \(batch.totalPeople) people"
        if batch.state == "paused" { return "Paused · \(people)" }
        return batch.current.map { "\(people) · \($0)" } ?? people
    }
}

/// *Learn From Past Mail…*: a deep dive for everyone written to in a
/// period (spec §14.16 *A deep dive for many*). The period chooses the
/// people; all the mail with each of them is read.
struct PastMailSheet: View {
    @Environment(AppModel.self) private var model
    @State private var days: UInt32 = 90
    @State private var custom = ""
    @State private var estimate: ContactBatchEstimate?
    @State private var error: String?

    private var chosenDays: UInt32 { UInt32(custom.trimmingCharacters(in: .whitespaces)) ?? days }

    var body: some View {
        Dialog(title: "Learn From Past Mail",
               message: "The period chooses the people: everyone you wrote to in it. For each of them, all your mail with them is read (the newest 2,000 messages), not only the mail from those days.") {
            Picker("People I wrote to in the last", selection: $days) {
                ForEach([30, 90, 180, 365] as [UInt32], id: \.self) { d in Text("\(d) days").tag(d) }
            }
            .pickerStyle(.segmented)
            .disabled(!custom.isEmpty)
            .hoverHelp("Who is read about: the people you sent mail to in this time")
            TextField("Or a number of days", text: $custom)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("A number of days, in place of the choices above")
            if let estimate {
                Text(Self.line(estimate)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else {
                ProgressView().controlSize(.small)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton { model.contacts.settingsSheet = nil }
            Button("Start") { Task { await start() } }
                .keyboardShortcut(.defaultAction)
                .disabled((estimate?.people ?? 0) == 0)
                .hoverHelp("Start reading, the person you wrote to most first (Return); pause or stop it in Learning Settings")
        }
        .task(id: chosenDays) {
            estimate = nil
            guard let core = model.core else { return }
            do throws(CoreClientError) {
                estimate = try await core.contactBatchEstimate(days: chosenDays)
                error = nil
            } catch {
                self.error = error.message
            }
        }
    }

    /// "85 people · 4,210 messages not yet read · about 260 agent turns."
    static func line(_ e: ContactBatchEstimate) -> String {
        if e.people == 0 { return "Nobody you wrote to in that time has mail left to read." }
        return "\(e.people) \(e.people == 1 ? "person" : "people") · \(e.messages.formatted()) message\(e.messages == 1 ? "" : "s") not yet read · about \(e.turns.formatted()) agent turn\(e.turns == 1 ? "" : "s")."
    }

    private func start() async {
        guard let core = model.core else { return }
        do {
            _ = try await core.startContactBatch(days: chosenDays)
            model.contacts.settingsSheet = nil
            await model.contacts.load()
        } catch {
            self.error = error.message
        }
    }
}

/// *Contact Tags*: the two sets under their headings, with add, rename and
/// delete. Deleting a tag takes it off every contact; Undo puts it back.
struct ContactTagsSheet: View {
    @Environment(AppModel.self) private var model
    @State private var set: ContactTagSet = .relationship
    @State private var newName = ""
    @State private var newHeading = ""
    @State private var renaming: String?
    @State private var name = ""
    @State private var error: String?

    private var groups: [(set: ContactTagSet, heading: String, tags: [ContactTagInfo])] {
        ContactsStore.grouped(model.contacts.tags).filter { $0.set == set }
    }

    var body: some View {
        Dialog(title: "Contact Tags",
               message: "Tags say what a contact is to you and what they do. Kaluta chooses only from this list when it suggests one.", width: Self.width) {
            Picker("Set", selection: $set) {
                Text("Relationship").tag(ContactTagSet.relationship)
                Text("Role").tag(ContactTagSet.role)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .hoverHelp("Relationship: what they are to you. Role: what they do")
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
                        VStack(alignment: .leading, spacing: Space.xs) {
                            Text(group.heading).font(TypeRole.caption).foregroundStyle(.secondary)
                            ForEach(group.tags, id: \.id) { tag in row(tag) }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: Self.listHeight)
            HStack(spacing: Space.s) {
                TextField("A tag of your own", text: $newName)
                    .textFieldStyle(.roundedBorder)
                Picker("Heading", selection: $newHeading) {
                    Text("Other").tag("")
                    ForEach(groups.map(\.heading).filter { $0 != "Other" }, id: \.self) { h in Text(h).tag(h) }
                }
                .labelsHidden()
                .fixedSize()
                .hoverHelp("The heading it is listed under")
                Button("Add") { Task { await add() } }
                    .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty || newName.count > Self.maxName)
                    .hoverHelp("Add the tag to your list")
            }
            if let message = error ?? (newName.count > Self.maxName ? "A tag's name is at most \(Self.maxName) characters." : nil) {
                Label(message, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            Button("Done") { model.contacts.settingsSheet = nil }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Close (Return)")
        }
    }

    @ViewBuilder private func row(_ tag: ContactTagInfo) -> some View {
        if renaming == tag.id {
            HStack(spacing: Space.s) {
                TextField("Name", text: $name).textFieldStyle(.roundedBorder)
                Button("Save") {
                    Task {
                        error = await model.editContactTags([.rename(id: tag.id, name: name)], actionName: "Rename Tag",
                                                            notice: "Renamed “\(tag.name)” to “\(name)”")?.message
                        if error == nil { renaming = nil }
                    }
                }
                .hoverHelp("Save the name; contacts with the tag show the new one")
                Button("Cancel") { renaming = nil } // inline
                    .hoverHelp("Keep it as it was")
            }
            .controlSize(.small)
        } else {
            HStack(spacing: Space.m) {
                Text(tag.name)
                if tag.contacts > 0 {
                    Text(tag.contacts == 1 ? "1 contact" : "\(tag.contacts) contacts")
                        .font(TypeRole.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: Space.m)
                Button("Rename…") {
                    name = tag.name
                    renaming = tag.id
                }
                .hoverHelp("Change its name")
                Button("Delete") {
                    Task {
                        error = await model.editContactTags([.delete(id: tag.id)], actionName: "Delete Tag",
                                                            notice: tag.contacts > 0 ? "Deleted “\(tag.name)” and took it off \(tag.contacts) contact\(tag.contacts == 1 ? "" : "s")"
                                                                : "Deleted “\(tag.name)”")?.message
                    }
                } // undoable
                .hoverHelp("Delete it and take it off every contact; Undo brings it back")
            }
            .controlSize(.small)
        }
    }

    private func add() async {
        error = await model.editContactTags([.add(name: newName, set: set, heading: newHeading)], actionName: "Add Tag",
                                            notice: "Added the tag “\(newName)”")?.message
        if error == nil { newName = "" }
    }

    private static let width: CGFloat = 520
    private static let listHeight: CGFloat = 300
    private static let maxName = 30
}
