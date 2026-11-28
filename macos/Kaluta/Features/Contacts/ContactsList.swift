import SwiftUI

/// The Contacts page's list (spec §14.16): the contacts with something
/// known or held, most recent first, each with its tags; a field that
/// finds any contact by name or address, and a filter by tag.
struct ContactsList: View {
    @Environment(AppModel.self) private var model
    @Bindable var store: ContactsStore
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 0) {
            ContactsFilterBar(store: store)
            Group {
                // Built only once it has rows, as Facts' list is (oagc-9hf).
                if store.list.isEmpty {
                    if store.loaded { empty }
                } else {
                    list
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onChange(of: model.threadListFocusRequests) { focused = true }
        .onChange(of: store.selection) { Task { await store.load() } }
        .task(id: model.openAccountID) { await store.load() }
    }

    @ViewBuilder private var empty: some View {
        if !store.search.isEmpty || !store.filterTagIDs.isEmpty {
            ContentUnavailableView("No Contacts Found", systemImage: "person.crop.circle.badge.questionmark",
                                   description: Text("Nobody matches. Clear the field or the tag filter to see everyone."))
        } else {
            ContentUnavailableView("No Contacts Yet", systemImage: "person.2",
                                   description: Text("People you correspond with appear here once something is known about them. Type a name or an address above to open anyone's page."))
        }
    }

    private var list: some View {
        List(selection: $store.selection) {
            ForEach(store.list, id: \.listTag) { row in
                ContactRow(row: row).tag(ContactsStore.tag(row))
                    .background(FocusRegionProbe(cycle: model.focus, region: .list))
            }
        }
        .listStyle(.inset)
        // Tab from the sidebar (the main window's loop, spec §14.3).
        .focused($focused)
    }
}

extension ContactSummary {
    var listTag: String { ContactsStore.tag(self) }
}

private struct ContactRow: View {
    let row: ContactSummary

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(row.name).lineLimit(1)
                if row.tags.isEmpty {
                    Text(row.email).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
                } else {
                    Text(ContactTagLine.text(row.tags)).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: Space.m)
            if row.held > 0 {
                Text("\(row.held)")
                    .font(TypeRole.numeric)
                    .foregroundStyle(Tone.caution)
                    .accessibilityLabel("\(row.held) to review")
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// A contact's tags as one line, relationships before roles:
/// "Customer · VP, Engineering".
enum ContactTagLine {
    static func text(_ tags: [ContactTagInfo]) -> String {
        [ContactTagSet.relationship, .role]
            .map { set in tags.filter { $0.set == set }.map(\.name).joined(separator: ", ") }
            .filter { !$0.isEmpty }
            .joined(separator: " · ")
    }
}

/// The field and the tag filter over the Contacts list.
private struct ContactsFilterBar: View {
    @Bindable var store: ContactsStore
    @State private var choosing = false

    var body: some View {
        ListHeaderBar {
            TextField("Find a contact", text: $store.search)
                .textFieldStyle(.roundedBorder)
                .onChange(of: store.search) { Task { await store.load() } }
                .accessibilityLabel("Find a contact by name or address")
            Button {
                choosing = true
            } label: {
                Label(store.filterTagIDs.isEmpty ? "Tags" : "\(store.filterTagIDs.count)",
                      systemImage: store.filterTagIDs.isEmpty ? "tag" : "tag.fill")
            }
            .hoverHelp(store.filterTagIDs.isEmpty ? "Show only contacts with the tags you choose"
                       : "Showing contacts with: \(chosenNames). Click to change")
            .popover(isPresented: $choosing, arrowEdge: .bottom) {
                TagPicker(tags: store.tags, chosen: store.filterTagIDs, showsCounts: true,
                          title: "Show contacts with every tag chosen") { id in
                    if let at = store.filterTagIDs.firstIndex(of: id) {
                        store.filterTagIDs.remove(at: at)
                    } else {
                        store.filterTagIDs.append(id)
                    }
                    Task { await store.load() }
                }
            }
            if !store.filterTagIDs.isEmpty {
                Button("Clear") {
                    store.filterTagIDs = []
                    Task { await store.load() }
                }
                .hoverHelp("Show contacts whatever their tags")
            }
        }
    }

    private var chosenNames: String {
        store.tags.filter { store.filterTagIDs.contains($0.id) }.map(\.name).joined(separator: ", ")
    }
}

/// Tags to choose from: a field that finds one by typing, with the list
/// under its headings below (spec §14.16). Used to tag a contact, and to
/// filter the Contacts list.
struct TagPicker: View {
    let tags: [ContactTagInfo]
    /// Tags already chosen (or already on the contact): shown ticked.
    var chosen: [String] = []
    var showsCounts = false
    var title: String
    /// A tag was clicked, or chosen with Return.
    var onChoose: (String) -> Void
    @State private var typed = ""
    @FocusState private var focused: Bool

    private var groups: [(set: ContactTagSet, heading: String, tags: [ContactTagInfo])] {
        ContactsStore.grouped(tags, typed: typed)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text(title).font(TypeRole.caption).foregroundStyle(.secondary)
            TextField("Type to find a tag", text: $typed)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit {
                    if let first = groups.first?.tags.first { onChoose(first.id) }
                }
            ScrollView {
                VStack(alignment: .leading, spacing: Space.m) {
                    ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
                        VStack(alignment: .leading, spacing: Space.xs) {
                            Text("\(group.set.title) · \(group.heading)")
                                .font(TypeRole.caption).foregroundStyle(.secondary)
                            ChipFlow(spacing: Space.xs) {
                                ForEach(group.tags, id: \.id) { tag in
                                    Button {
                                        onChoose(tag.id)
                                    } label: {
                                        HStack(spacing: Space.xs) {
                                            if chosen.contains(tag.id) { Image(systemName: "checkmark") }
                                            Text(showsCounts && tag.contacts > 0 ? "\(tag.name) \(tag.contacts)" : tag.name)
                                        }
                                    }
                                    .buttonStyle(.bordered)
                                    .controlSize(.small)
                                    .hoverHelp(Self.help(tag, chosen: chosen.contains(tag.id)))
                                }
                            }
                        }
                    }
                    if groups.isEmpty {
                        Text("No tag named “\(typed)”. Add your own in Learning Settings › Contact Tags.")
                            .font(TypeRole.meta).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: Self.listHeight)
        }
        .padding(Space.xl)
        .frame(width: Self.width)
        .onAppear { focused = true }
    }

    static func help(_ tag: ContactTagInfo, chosen: Bool) -> String {
        let people = tag.contacts == 1 ? "1 contact has it" : "\(tag.contacts) contacts have it"
        return chosen ? "\(tag.name): chosen. \(people)" : "\(tag.name). \(people)"
    }

    private static let width: CGFloat = 420
    private static let listHeight: CGFloat = 320
}
