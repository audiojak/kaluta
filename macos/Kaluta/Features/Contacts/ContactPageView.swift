import SwiftUI

/// A contact's page (spec §14.16): who they are, what is held for review,
/// what is open between the user and them, the facts known, and (found by
/// query, not copied) the replies the user is waiting for from them, the
/// tasks made from their mail and their recent conversations.
struct ContactPageView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var store = model.contacts
        Group {
            if let contact = store.contact {
                ContactPage(contact: contact, store: store)
            } else if let learning = store.learning, !learning.asked, !learning.unavailable {
                // The first time Contacts is opened: the ask, in place of the empty page.
                ScrollView { ContactLearningAsk(learning: learning, contact: nil).padding(Space.page) }
            } else {
                ContentUnavailableView("No Contact Selected", systemImage: "person.crop.circle",
                                       description: Text("What you know about a person, what is open between you, the replies you are waiting for from them and the tasks from their mail."))
            }
        }
        .sheet(item: $store.sheet) { sheet in
            switch sheet {
            case let .fact(item): ContactFactEditor(item: item)
            case .addTag: AddTagSheet()
            case .samePerson: SamePersonSheet()
            case .dive: ContactDiveSheet()
            case let .review(items): ContactReviewSheet(items: items)
            }
        }
    }
}

private struct ContactPage: View {
    @Environment(AppModel.self) private var model
    let contact: ContactInfo
    let store: ContactsStore

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                header
                if let learning = store.learning, !learning.unavailable {
                    if !learning.asked {
                        ContactLearningAsk(learning: learning, contact: contact)
                    } else if learning.offerAfterDive, store.dive?.state == "done" {
                        KeepLearningOffer()
                    }
                }
                if let dive = store.dive, ["running", "queued", "paused", "failed"].contains(dive.state) {
                    DiveProgress(dive: dive, contact: contact)
                }
                if !store.needsReview.isEmpty { reviewSection }
                if !store.openTopics.isEmpty {
                    ContactSection(title: "Open topics") {
                        ForEach(store.openTopics, id: \.id) { ContactItemRow(item: $0, store: store) }
                    }
                }
                commitments
                ForEach(store.factsByClass, id: \.heading) { group in
                    ContactSection(title: group.heading.title) {
                        ForEach(group.facts, id: \.id) { ContactItemRow(item: $0, store: store) }
                    }
                }
                if !store.hasItems, store.learning?.asked == true, store.learning?.on == false {
                    notLearning
                }
                if let links = store.links { ContactLinksView(links: links) }
                if !store.closed.isEmpty {
                    DisclosureGroup("Finished (\(store.closed.count))") {
                        VStack(alignment: .leading, spacing: Space.m) {
                            ForEach(store.closed, id: \.id) { ContactItemRow(item: $0, store: store) }
                        }
                        .padding(.top, Space.s)
                    }
                    .font(TypeRole.groupLabel)
                }
            }
            .padding(Space.page)
            .frame(maxWidth: Self.readingWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: contact.personId) {
            // Seen: nothing on the page is New the next time.
            try? await Task.sleep(for: .seconds(2))
            try? await model.core?.markContactSeen(contact.personId)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text(contact.name).font(TypeRole.display).textSelection(.enabled)
            if !contact.tags.isEmpty {
                ChipFlow(spacing: Space.xs) {
                    ForEach(contact.tags, id: \.id) { tag in
                        ContactTagChip(tag: tag) {
                            Task {
                                await model.applyContactEdits([.removeTag(personId: contact.personId, tagId: tag.id)],
                                                              actionName: "Remove Tag", notice: "Removed “\(tag.name)”")
                            }
                        }
                    }
                }
            }
            VStack(alignment: .leading, spacing: Space.hair) {
                ForEach(contact.addresses, id: \.email) { a in
                    Text(a.email).textSelection(.enabled)
                }
                if let links = store.links, let first = links.firstMail, let latest = links.latestMail {
                    Text("Mail since \(Self.day(first)), latest \(Self.day(latest)) · \(links.sent) from you, \(links.received) from them")
                }
            }
            .font(TypeRole.meta)
            .foregroundStyle(.secondary)
            HStack(spacing: Space.m) {
                Button("Learn From All Our Mail…") { store.sheet = .dive }
                    .hoverHelp("Read all your mail with \(contact.name) for facts, open topics and commitments; says how much first")
                Button("Add Fact…") { store.sheet = .fact(nil) }
                    .hoverHelp("Write down a fact, a topic or a commitment yourself")
                Button("Add Tag…") { store.sheet = .addTag }
                    .hoverHelp("Say what they are to you, or what they do, with tags from your list")
                Menu("More") {
                    Button("Same Person As…") { store.sheet = .samePerson } // no-help: menu
                    Button("Forget This Contact") { Task { await model.forgetContact(contact) } } // no-help: menu
                        .disabled(!store.hasItems)
                }
                .fixedSize()
                .hoverHelp("Join this contact with another, or forget what is known about them")
            }
            .controlSize(.small)
        }
    }

    private var reviewSection: some View {
        ContactSection(title: "Needs review") {
            Text("Held because of what they say or who said it. Nothing here is used in drafts until you accept it.")
                .font(TypeRole.meta).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            ForEach(store.needsReview, id: \.id) { item in
                HeldItemCard(item: item, changes: store.facts.first { $0.id == item.changesId })
            }
            if store.needsReview.count > 1 {
                Button("Review All \(store.needsReview.count)…") { store.sheet = .review(store.needsReview) }
                    .controlSize(.small)
                    .hoverHelp("Go through them one at a time: Return accepts, Delete rejects")
            }
        }
    }

    @ViewBuilder private var commitments: some View {
        let yours = store.commitments(owedBy: .you)
        let theirs = store.commitments(owedBy: .them)
        if !yours.isEmpty {
            ContactSection(title: "You owe") {
                ForEach(yours, id: \.id) { ContactItemRow(item: $0, store: store) }
            }
        }
        if !theirs.isEmpty {
            ContactSection(title: "They owe") {
                ForEach(theirs, id: \.id) { ContactItemRow(item: $0, store: store) }
            }
        }
    }

    /// Where the learned sections would be, after *Not Now*.
    private var notLearning: some View {
        HStack(spacing: Space.s) {
            Text("Kaluta is not learning about contacts ·").foregroundStyle(.secondary)
            Button("Turn On…") { Task { await model.setContactLearning(true) } }
                .buttonStyle(.link)
                .hoverHelp("Read new mail from people you have written to, for facts, open topics and commitments")
        }
        .font(TypeRole.meta)
    }

    static func day(_ millis: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(millis) / 1000).formatted(date: .abbreviated, time: .omitted)
    }

    private static let readingWidth: CGFloat = 760
}

/// A titled group on the page.
struct ContactSection<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text(title).font(TypeRole.groupLabel).foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
            content
        }
    }
}

/// A tag on the page's header: its name, and a way to take it off.
private struct ContactTagChip: View {
    let tag: ContactTagInfo
    var onRemove: () -> Void

    var body: some View {
        HStack(spacing: Space.xs) {
            Text(tag.name)
            Button(action: onRemove) {
                Image(systemName: "xmark").imageScale(.small)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Remove \(tag.name)")
            .hoverHelp("Take “\(tag.name)” off this contact; Undo puts it back")
        }
        .font(Font(TypeRole.chip))
        .padding(.horizontal, Space.s)
        .padding(.vertical, Space.hair)
        .background(Color(nsColor: Tone.chipFill(hex: nil)), in: .rect(cornerRadius: Radius.chip))
        .accessibilityElement(children: .contain)
    }
}

/// One accepted item: what it says, where it came from, and its actions.
private struct ContactItemRow: View {
    @Environment(AppModel.self) private var model
    let item: ContactFactInfo
    let store: ContactsStore
    @State private var showsQuotes = false

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                VStack(alignment: .leading, spacing: Space.hair) {
                    HStack(spacing: Space.s) {
                        Text(item.label).font(TypeRole.heading)
                        if item.isNew { NewDot() }
                    }
                    Text(item.value).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: Space.m)
                marks
                menu
            }
            Text(caption).font(TypeRole.caption).foregroundStyle(item.stale ? Tone.caution : .secondary)
            if store.heldChange(to: item) != nil {
                Label("A change to this waits under Needs review", systemImage: "hourglass")
                    .font(TypeRole.caption).foregroundStyle(Tone.caution)
            }
            if showsQuotes {
                ForEach(Array(item.evidence.enumerated()), id: \.offset) { _, quote in
                    ContactQuoteView(quote: quote)
                }
            }
        }
    }

    @ViewBuilder private var marks: some View {
        if item.use == .ask {
            Image(systemName: "hand.raised").foregroundStyle(.secondary).accessibilityLabel("Ask before using")
        } else if item.use == .never {
            Image(systemName: "eye.slash").foregroundStyle(.secondary).accessibilityLabel("Never used")
        }
        if item.stale {
            Image(systemName: "clock.badge.exclamationmark").foregroundStyle(Tone.caution)
                .accessibilityLabel(item.kind == .fact ? "May be out of date" : "Gone quiet: left out of drafts")
        }
    }

    private var menu: some View {
        Menu {
            actions
        } label: {
            Label("Actions", systemImage: "ellipsis.circle")
        }
        .labelStyle(.iconOnly)
        .menuStyle(.borderlessButton)
        .fixedSize()
        .hoverHelp("Edit, finish, change how drafts use it, see where it came from, or delete")
    }

    /// The row's menu.
    @ViewBuilder private var actions: some View {
        Button("Edit…") { store.sheet = .fact(item) } // no-help: menu
        if item.kind == .topic || item.kind == .commitment {
            Button(item.open == true ? "Mark as Finished" : "Reopen") { // no-help: menu
                Task {
                    await model.applyContactEdits([.setOpen(id: item.id, open: item.open != true)],
                                                  actionName: item.open == true ? "Finish" : "Reopen",
                                                  notice: item.open == true ? "Finished “\(item.label)”" : "Reopened “\(item.label)”")
                }
            }
        }
        if item.kind == .commitment, item.owedBy == .you, item.open == true {
            Button("Make Task") { Task { await model.makeTask(from: item) } } // no-help: menu
                .disabled(!item.evidence.contains { $0.threadId != nil })
        }
        Menu("In Drafts") { // no-help: menu
            ForEach([FactUse.free, .ask, .never], id: \.self) { use in
                Button(item.use == use ? "✓ \(use.contactTitle)" : use.contactTitle) { // no-help: menu
                    Task {
                        await model.applyContactEdits([.setUse(id: item.id, use: use)], actionName: "Change Use",
                                                      notice: "“\(item.label)”: \(use.contactTitle.lowercased())")
                    }
                }
            }
        }
        if !item.evidence.isEmpty {
            Button(showsQuotes ? "Hide Where It Came From" : "Show Where It Came From") { showsQuotes.toggle() } // no-help: menu
        }
        Divider() // menu
        Button("Delete") { // no-help: menu
            Task {
                await model.applyContactEdits([.delete(id: item.id)], actionName: "Delete", notice: "Deleted “\(item.label)”")
            }
        }
    }

    private var caption: String {
        var parts: [String] = []
        parts.append(item.source == .you ? "You wrote this" : "Learned from your mail")
        if let asOf = item.asOf { parts.append("as of \(ContactPage.day(asOf))") }
        if let due = item.due { parts.append("due \(ContactPage.day(due))") }
        if item.stale { parts.append(item.kind == .fact ? "may be out of date" : "gone quiet, so left out of drafts") }
        return parts.joined(separator: " · ")
    }
}

extension FactUse {
    /// As a contact's item says it (facts about the user say "Never share").
    var contactTitle: String {
        switch self {
        case .free: "Use freely"
        case .ask: "Ask before using"
        case .never: "Never use"
        }
    }
}

/// A quote an item rests on: who said it, when, and the words.
struct ContactQuoteView: View {
    @Environment(AppModel.self) private var model
    let quote: ContactQuote

    var body: some View {
        HStack(alignment: .top, spacing: Space.m) {
            Rectangle().fill(.quaternary).frame(width: Self.bar)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text("“\(quote.quote)”").italic().textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                HStack(spacing: Space.s) {
                    Text("\(quote.saidBy.title), \(ContactPage.day(quote.messageDate))")
                        .foregroundStyle(quote.saidBy == .someoneElse ? Tone.caution : .secondary)
                    if let threadID = quote.threadId {
                        Button("Open the Conversation") { model.reveal(threadID: threadID) }
                            .buttonStyle(.link)
                            .hoverHelp("Show the conversation this was said in")
                    }
                }
                .font(TypeRole.caption)
            }
        }
        .font(TypeRole.meta)
    }

    private static let bar: CGFloat = 3
}

/// A held item on the page: what it would say, why it is held, the
/// quotes, and Accept or Reject.
struct HeldItemCard: View {
    @Environment(AppModel.self) private var model
    let item: ContactFactInfo
    /// The item it would change, for a held change.
    let changes: ContactFactInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            HeldItemBody(item: item, changes: changes)
            HStack(spacing: Space.m) {
                Button("Accept") { Task { _ = await model.decideContactFacts([item], accept: true) } }
                    .hoverHelp("Keep it: drafts to this person may use it")
                Button("Reject") { Task { _ = await model.decideContactFacts([item], accept: false) } }
                    .hoverHelp("Leave it out; it will not be suggested again")
            }
            .controlSize(.small)
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(Self.fill), in: .rect(cornerRadius: Radius.card))
    }

    private static let fill = 0.5
}

/// A held item as a document: its kind and class, what it says, why it
/// was held, and each quote with who said it.
struct HeldItemBody: View {
    let item: ContactFactInfo
    let changes: ContactFactInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Label(title, systemImage: item.kind.symbol).font(TypeRole.caption).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(item.headline).font(TypeRole.heading)
                if item.kind != .tag {
                    Text(item.value).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }
            }
            if let changes {
                Text(changeLine(changes)).font(TypeRole.meta).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let why = item.riskReason {
                Label(why, systemImage: "exclamationmark.shield").font(TypeRole.meta).foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(item.evidence.enumerated()), id: \.offset) { _, quote in
                ContactQuoteView(quote: quote)
            }
        }
    }

    private var title: String {
        if item.kind == .tag { return "Tag · \(item.tag?.set.title ?? "")" }
        return "\(item.kindTitle) · \(item.class.title)"
    }

    private func changeLine(_ old: ContactFactInfo) -> String {
        if item.open == false, old.open == true { return "Would mark “\(old.label)” as finished. It stays open until you accept." }
        if item.value != old.value { return "Would replace “\(old.value)”." }
        if item.due != old.due { return "Would change when “\(old.label)” is due." }
        return "Would change “\(old.label)”."
    }
}

/// Waits, tasks and recent conversations: found by query each time the
/// page opens, and acted on in their own places.
private struct ContactLinksView: View {
    @Environment(AppModel.self) private var model
    let links: ContactLinks
    @State private var showsDone = false

    var body: some View {
        if !links.waiting.isEmpty {
            ContactSection(title: "Waiting on them") {
                ForEach(links.waiting, id: \.threadId) { wait in
                    row(wait.subject.isEmpty ? "(no subject)" : wait.subject, detail: "since \(ContactPage.day(wait.since))",
                        symbol: "hourglass", threadID: wait.threadId, help: "Open the conversation you are waiting on")
                }
            }
        }
        let open = links.tasks.filter { !$0.done }
        let done = links.tasks.filter(\.done)
        if !links.tasks.isEmpty {
            ContactSection(title: "Tasks from their mail") {
                ForEach(open, id: \.id) { task in
                    row(task.title, detail: task.dueDay.map { DueDay.label($0) } ?? "", symbol: "circle",
                        threadID: task.threadId, help: "Open the conversation the task is on")
                }
                if !done.isEmpty {
                    DisclosureGroup("Done (\(done.count))", isExpanded: $showsDone) {
                        VStack(alignment: .leading, spacing: Space.s) {
                            ForEach(done, id: \.id) { task in
                                row(task.title, detail: "", symbol: "checkmark.circle", threadID: task.threadId,
                                    help: "Open the conversation the task was on")
                            }
                        }
                        .padding(.top, Space.xs)
                    }
                    .font(TypeRole.meta)
                }
            }
        }
        if !links.recent.isEmpty {
            ContactSection(title: "Recent conversations") {
                ForEach(links.recent, id: \.threadId) { c in
                    row(c.subject.isEmpty ? "(no subject)" : c.subject, detail: ContactPage.day(c.latest), symbol: "envelope",
                        threadID: c.threadId, help: "Open the conversation")
                }
            }
        }
    }

    private func row(_ title: String, detail: String, symbol: String, threadID: String, help: String) -> some View {
        Button {
            model.reveal(threadID: threadID)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                Image(systemName: symbol).foregroundStyle(.secondary)
                Text(title).lineLimit(1)
                Spacer(minLength: Space.m)
                Text(detail).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .hoverHelp(help)
    }
}

/// The ask, the first time Contacts or a contact's page is opened (spec
/// §14.16 *Turning it on*): what is learned, that mail from people the
/// user has written to is read by their own agent, about how many turns a
/// day that is, and three answers. It does not return.
struct ContactLearningAsk: View {
    @Environment(AppModel.self) private var model
    let learning: ContactLearning
    /// The contact whose page it is on, for *Just This Person*.
    let contact: ContactInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            Text("Learn about the people you write to?").font(TypeRole.title)
            Text("Kaluta can keep notes on each person you correspond with: what is open between you, who owes whom what, and who they are. Drafts to that person use them. Anything about money, agreements or where to send things waits for you to accept it.")
                .fixedSize(horizontal: false, vertical: true)
            Text("New mail in conversations with people you have written to is read by your own agent (Claude Code or Codex), one conversation at a time. \(Self.estimate(learning)) You can set a daily limit in Learning Settings.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: Space.m) {
                Button("Turn On") { Task { await model.setContactLearning(true) } }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Start learning from mail that arrives from now on")
                if contact != nil {
                    Button("Just This Person…") {
                        Task {
                            try? await model.core?.contactLearningWasAsked()
                            await model.contacts.load()
                            model.contacts.sheet = .dive
                        }
                    }
                    .hoverHelp("Read your mail with this one person; nothing else is read")
                }
                Button("Not Now") {
                    Task {
                        try? await model.core?.contactLearningWasAsked()
                        await model.contacts.load()
                    }
                }
                .hoverHelp("Read nothing. The page still shows waits, tasks, recent conversations and what you add yourself")
            }
        }
        .padding(Space.xl)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(Self.fill), in: .rect(cornerRadius: Radius.panel))
    }

    /// "That is about 12 to 30 agent turns a day, going by last week's mail."
    static func estimate(_ learning: ContactLearning) -> String {
        let (low, high) = (learning.estimateLow, learning.estimateHigh)
        if high == 0 { return "Going by last week's mail, that is very few agent turns a day." }
        let range = low == high ? "\(high)" : "\(low) to \(high)"
        return "That is about \(range) agent turn\(high == 1 ? "" : "s") a day, going by last week's mail."
    }

    private static let fill = 0.5
}

/// After a first deep dive with learning still off: offered once.
private struct KeepLearningOffer: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: Space.m) {
            Text("Keep learning about the people you write to?")
            Spacer(minLength: Space.m)
            Button("Turn On") { Task { await model.setContactLearning(true) } }
                .hoverHelp("Read new mail from people you have written to, from now on")
            Button("Not Now") {
                Task {
                    try? await model.core?.contactLearningOfferDeclined()
                    await model.contacts.load()
                }
            }
            .hoverHelp("Leave it off; you can turn it on in Learning Settings")
        }
        .controlSize(.small)
        .padding(Space.l)
        .background(.quaternary.opacity(Self.fill), in: .rect(cornerRadius: Radius.card))
    }

    private static let fill = 0.5
}

/// A deep dive under way on the page: how far it is, and Stop.
private struct DiveProgress: View {
    @Environment(AppModel.self) private var model
    let dive: ContactDive
    let contact: ContactInfo

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.m) {
                if dive.state == "running" || dive.state == "queued" { ProgressView().controlSize(.small) }
                Text(Self.line(dive))
                Spacer(minLength: Space.m)
                if dive.state == "paused" || dive.state == "failed" {
                    Button("Carry On") { model.contacts.sheet = .dive }
                        .hoverHelp("Read the rest; what was read stays read")
                }
                Button("Stop") {
                    Task {
                        try? await model.core?.stopContactDive(contact.personId)
                        await model.contacts.load()
                    }
                }
                .hoverHelp("Stop reading; what was found so far stays")
            }
            .controlSize(.small)
            if dive.totalMessages > 0 {
                ProgressView(value: Double(min(dive.doneMessages, dive.totalMessages)), total: Double(dive.totalMessages))
            }
            if let error = dive.error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        }
        .font(TypeRole.meta)
        .padding(Space.l)
        .background(.quaternary.opacity(Self.fill), in: .rect(cornerRadius: Radius.card))
    }

    static func line(_ dive: ContactDive) -> String {
        let read = "\(dive.doneMessages) of \(dive.totalMessages) messages read"
        return switch dive.state {
        case "queued": "Waiting its turn · \(read)"
        case "paused": "Paused · \(read)"
        case "failed": "Stopped by a problem · \(read)"
        default: "Learning from your mail · \(read)"
        }
    }

    private static let fill = 0.5
}
