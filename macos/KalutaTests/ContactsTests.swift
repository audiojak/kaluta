import Foundation
import Testing
@testable import Kaluta

/// Contacts and contact facts (spec §14.16).
@MainActor
struct ContactsTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        return model
    }

    private func tag(_ id: String, _ name: String, _ set: ContactTagSet, _ heading: String, contacts: Int64 = 0) -> ContactTagInfo {
        ContactTagInfo(id: id, name: name, set: set, heading: heading, builtin: true, contacts: contacts)
    }

    @Test func thePageOpensForAPersonAndEditsUndo() async throws {
        let model = try await demo()
        let account = model.openAccountID
        #expect(model.contacts.learning?.on == false, "off until the user says yes")
        #expect(model.contacts.learning?.asked == false)
        #expect(model.contacts.list.isEmpty && model.contacts.held == 0)

        // Any person's address opens a page; an automated sender's does not.
        #expect(await model.contacts.open(email: "reese@acme.example") == nil)
        #expect(await model.contacts.open(email: "no-reply@shop.example") != nil)
        let contact = try #require(model.contacts.contact)
        #expect(contact.addresses.map(\.email) == ["reese@acme.example"])
        model.openContacts()
        #expect(model.isContacts && model.listMailboxID == nil, "the sidebar entry opens the page, not a mailbox")

        let fields = ContactFactFields(kind: .commitment, label: "Send the revised contract", value: "By Friday",
                                       class: .other, owedBy: .you, due: nil, use: .free, asOf: nil)
        #expect(await model.applyContactEdits([.add(personId: contact.personId, fields: fields),
                                               .addTag(personId: contact.personId, tagId: "rel.customer"),
                                               .addTag(personId: contact.personId, tagId: "role.vp")],
                                              actionName: "Add Note", notice: "Added") == nil)
        #expect(model.contacts.commitments(owedBy: .you).map(\.label) == ["Send the revised contract"])
        #expect(model.contacts.commitments(owedBy: .them).isEmpty)
        #expect(model.contacts.contact?.tags.map(\.name) == ["Customer", "VP"])
        #expect(model.contacts.list.map(\.name) == [contact.name], "a contact with something known is listed")
        #expect(ContactTagLine.text(model.contacts.list[0].tags) == "Customer · VP")
        #expect(model.undo.undoTitle(in: account) == "Undo Add Note")
        let long = ContactFactFields(kind: .fact, label: "Note", value: String(repeating: "x", count: 161),
                                     class: .other, owedBy: nil, due: nil, use: .free, asOf: nil)
        #expect(await model.applyContactEdits([.add(personId: contact.personId, fields: long)], actionName: "Add Note",
                                              notice: "Added") != nil, "an over-long value is refused")

        // The tag filter: every tag chosen.
        model.contacts.filterTagIDs = ["rel.customer", "role.cto"]
        await model.contacts.load()
        #expect(model.contacts.list.isEmpty)
        model.contacts.filterTagIDs = ["rel.customer"]
        await model.contacts.load()
        #expect(model.contacts.list.count == 1)
        model.contacts.filterTagIDs = []

        model.undo.undo(in: account)
        for _ in 0..<100 where model.contacts.hasItems {
            try await Task.sleep(for: .milliseconds(20))
            await model.contacts.load()
        }
        #expect(!model.contacts.hasItems, "one undo takes back the whole change")
        model.undo.redo(in: account)
        for _ in 0..<100 where !model.contacts.hasItems {
            try await Task.sleep(for: .milliseconds(20))
            await model.contacts.load()
        }
        #expect(model.contacts.contact?.tags.count == 2)
    }

    @Test func learningIsTurnedOnOnlyByTheUserAndTheLimitIsTheirs() async throws {
        let model = try await demo()
        let core = try #require(model.core)
        try await core.contactLearningWasAsked()
        await model.contacts.load()
        #expect(model.contacts.learning?.asked == true && model.contacts.learning?.on == false, "Not Now: asked, still off")
        await model.setContactLearning(true)
        #expect(model.contacts.learning?.on == true)
        #expect(model.contacts.learning?.perDay == 0, "no limit unless the user sets one")
        try await core.setContactConversationsADay(25)
        await model.contacts.load()
        #expect(model.contacts.learning?.perDay == 25)
        await model.setContactLearning(false)
        #expect(model.contacts.learning?.on == false)
    }

    @Test func theUsersOwnTagsAreAddedRenamedAndDeleted() async throws {
        let model = try await demo()
        await model.contacts.load()
        #expect(model.contacts.tags.filter { $0.set == .relationship }.count == 61)
        #expect(model.contacts.tags.filter { $0.set == .role }.count == 77)
        #expect(await model.editContactTags([.add(name: "Design partner", set: .relationship, heading: "")],
                                            actionName: "Add Tag", notice: "Added") == nil)
        let mine = try #require(model.contacts.tags.first { $0.name == "Design partner" })
        #expect(!mine.builtin && mine.heading == "Other")
        #expect(await model.editContactTags([.add(name: "design-partner", set: .relationship, heading: "")],
                                            actionName: "Add Tag", notice: "Added") != nil, "a name is unique within its set")
        #expect(await model.editContactTags([.rename(id: mine.id, name: "Launch partner")], actionName: "Rename Tag",
                                            notice: "Renamed") == nil)
        #expect(model.contacts.tags.contains { $0.id == mine.id && $0.name == "Launch partner" })
        #expect(await model.editContactTags([.delete(id: mine.id)], actionName: "Delete Tag", notice: "Deleted") == nil)
        #expect(!model.contacts.tags.contains { $0.id == mine.id })
    }

    @Test func tagsAreFoundByAnyWordAndGroupedUnderTheirHeadings() {
        #expect(ContactsStore.matches("VP of Engineering", typed: "eng"))
        #expect(ContactsStore.matches("Services client", typed: "services-cl"))
        #expect(ContactsStore.matches("Business Development", typed: "dev bus"))
        #expect(ContactsStore.matches("Customer", typed: ""))
        #expect(!ContactsStore.matches("Customer", typed: "ustomer"), "the start of a word, not its middle")
        let tags = [tag("rel.customer", "Customer", .relationship, "They buy from me"),
                    tag("rel.prospect", "Prospect", .relationship, "They buy from me"),
                    tag("rel.vendor", "Vendor", .relationship, "I buy from them"),
                    tag("role.vp", "VP", .role, "Level")]
        let all = ContactsStore.grouped(tags)
        #expect(all.map(\.heading) == ["They buy from me", "I buy from them", "Level"])
        #expect(all[0].tags.map(\.name) == ["Customer", "Prospect"])
        #expect(ContactsStore.grouped(tags, typed: "v").flatMap(\.tags).map(\.name) == ["Vendor", "VP"])
        #expect(ContactTagLine.text([tags[3], tags[0]]) == "Customer · VP", "relationships before roles")
    }

    @Test func typingTagInTheSearchFieldOffersTheAccountsTags() throws {
        // What is being typed: the last term, when it starts with tag:.
        #expect(TagCompletion.partial(in: "invoice") == nil)
        #expect(TagCompletion.partial(in: "tag:customer has:attachment") == nil, "only the term being typed")
        let plain = try #require(TagCompletion.partial(in: "has:attachment tag:cus"))
        #expect(plain.before == "has:attachment " && plain.typed == "cus")
        let quoted = try #require(TagCompletion.partial(in: "tag:\"services cl"))
        #expect(quoted.before == "" && quoted.typed == "services cl")
        let negated = try #require(TagCompletion.partial(in: "in:inbox -TAG:"))
        #expect(negated.before == "in:inbox -" && negated.typed == "")
        #expect(TagCompletion.partial(in: "tag:\"services client\"") == nil, "a closed quote is finished")
        let grouped = try #require(TagCompletion.partial(in: "(tag:ven"))
        #expect(grouped.before == "(" && grouped.typed == "ven")

        #expect(TagCompletion.term("Customer") == "tag:Customer")
        #expect(TagCompletion.term("Services client") == "tag:\"Services client\"")

        let tags = [tag("rel.customer", "Customer", .relationship, "They buy from me", contacts: 14),
                    tag("rel.services-client", "Services client", .relationship, "They buy from me", contacts: 2),
                    tag("rel.vendor", "Vendor", .relationship, "I buy from them"),
                    tag("role.cto", "CTO", .role, "Title", contacts: 3),
                    tag("role.vp", "VP", .role, "Level", contacts: 1)]
        #expect(TagCompletion.groups(for: "invoice", tags: tags).isEmpty)
        let menu = TagCompletion.groups(for: "has:attachment tag:", tags: tags)
        #expect(menu.map(\.title) == ["Relationship", "Role", "On no contact yet"], "tags in use first")
        #expect(menu[0].suggestions.map(\.tag.name) == ["Customer", "Services client"], "the most used first")
        #expect(menu[0].suggestions[0].spoken == "Customer, 14 contacts")
        #expect(menu[1].suggestions.last?.spoken == "VP, 1 contact")
        #expect(menu[0].suggestions[1].query == "has:attachment tag:\"Services client\" ", "quoted when it has a space")
        let narrowed = TagCompletion.groups(for: "tag:c", tags: tags)
        #expect(narrowed.flatMap(\.suggestions).map(\.tag.name) == ["Customer", "Services client", "CTO"], "any word of a name")
        #expect(TagCompletion.groups(for: "-tag:ven", tags: tags).first?.suggestions.first?.query == "-tag:Vendor ")
        #expect(TagCompletion.groups(for: "tag:vendor", tags: tags).isEmpty, "typed in full: nothing to complete")
        #expect(TagCompletion.groups(for: "tag:wizard", tags: tags).isEmpty)
    }

    @Test func theAskAndTheEstimatesSayWhatWillBeRead() {
        let learning = { (low: UInt32, high: UInt32) in
            ContactLearning(on: false, asked: false, perDay: 0, readToday: 12, readYesterday: 30, estimateLow: low,
                            estimateHigh: high, offerAfterDive: false, unavailable: false)
        }
        #expect(ContactLearningAsk.estimate(learning(12, 30)).contains("about 12 to 30 agent turns a day"))
        #expect(ContactLearningAsk.estimate(learning(5, 5)).contains("about 5 agent turns a day"))
        #expect(ContactLearningAsk.estimate(learning(0, 0)).contains("very few"))
        #expect(ContactLearningSettings.readLine(learning(0, 0)) == "Read today: 12 · yesterday: 30")
        #expect(ContactDiveSheet.line(ContactDiveEstimate(messages: 46, unread: 46, turns: 4)) == "46 messages to read: about 4 agent turns.")
        #expect(ContactDiveSheet.line(ContactDiveEstimate(messages: 46, unread: 6, turns: 2)).contains("(40 already read)"))
        #expect(ContactDiveSheet.line(ContactDiveEstimate(messages: 46, unread: 0, turns: 0)) == "All 46 messages have been read already.")
        #expect(PastMailSheet.line(ContactBatchEstimate(people: 85, messages: 4210, turns: 260)).hasPrefix("85 people · "))
        #expect(PastMailSheet.line(ContactBatchEstimate(people: 0, messages: 0, turns: 0)).hasPrefix("Nobody"))
        let batch = ContactBatch(id: 1, days: 90, state: "running", totalPeople: 85, donePeople: 12, current: "Reese Okafor", why: nil)
        #expect(ContactLearningSettings.batchLine(batch) == "12 of 85 people · Reese Okafor")
    }
}
