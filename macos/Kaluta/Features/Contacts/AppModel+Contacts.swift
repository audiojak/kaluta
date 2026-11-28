import Foundation

/// Contacts (spec §14.16): opening a contact's page and changing what is
/// known about them, each change on the account's undo stack (ADR 0006).
extension AppModel {
    /// The sidebar's Contacts entry, in place of a mailbox id.
    static let contactsMailboxID = "@contacts"

    var isContacts: Bool { selectedMailboxID == Self.contactsMailboxID && threads.searchQuery == nil }

    func openContacts() {
        guidePrompt = nil
        selectedMailboxID = Self.contactsMailboxID
    }

    /// *Show Contact*: the page of whoever has this address.
    func showContact(email: String) {
        Task {
            if let failure = await contacts.open(email: email) {
                contactNotice(failure)
            } else {
                openContacts()
            }
        }
    }

    /// *Show Contact* for the open conversation: the first of its people
    /// who is a person and not the user.
    func showContactOfSelection() {
        guard let row = taskTarget else { return }
        let people = row.participants.map(\.email)
        Task {
            var failure = "Nobody in this conversation has a contact page"
            for email in people {
                guard let problem = await contacts.open(email: email) else {
                    openContacts()
                    return
                }
                failure = problem
            }
            contactNotice(failure)
        }
    }

    /// Say something on the page's notice bar (a failure, mostly).
    func contactNotice(_ text: String) {
        guard let accountID = openAccountID else { return }
        undo.show(text, accountID: accountID, offersUndo: false)
    }

    /// Contacts changed in the store (an edit, an undo, the daily job, a deep dive).
    func contactsChanged() async {
        await contacts.load()
    }

    /// Apply edits to contacts' items as one change.
    @discardableResult
    func applyContactEdits(_ edits: [ContactEdit], actionName: String, notice: String) async -> CoreClientError? {
        guard let core else { return CoreClientError(kind: .notFound, message: "No account is open") }
        return await recordContacts(actionName, notice: notice) { () async throws(CoreClientError) -> ContactChange in
            try await core.applyContactEdits(edits, reason: actionName)
        }
    }

    /// Change the account's tag list as one change.
    @discardableResult
    func editContactTags(_ edits: [ContactTagEdit], actionName: String, notice: String) async -> CoreClientError? {
        guard let core else { return CoreClientError(kind: .notFound, message: "No account is open") }
        return await recordContacts(actionName, notice: notice) { () async throws(CoreClientError) -> ContactChange in
            try await core.editContactTags(edits)
        }
    }

    /// *Same Person As…*: this contact's addresses and items join `into`.
    @discardableResult
    func joinContact(_ contact: ContactInfo, into other: ContactSummary) async -> CoreClientError? {
        guard let core, let otherID = other.personId else {
            return CoreClientError(kind: .notFound, message: "Choose a contact")
        }
        let failure = await recordContacts("Same Person", notice: "\(contact.name) and \(other.name) are one contact now") { () async throws(CoreClientError) -> ContactChange in
            try await core.joinContacts(contact.personId, into: otherID)
        }
        if failure == nil { contacts.selection = ContactsStore.tag(person: otherID) }
        await contacts.load()
        return failure
    }

    func forgetContact(_ contact: ContactInfo) async {
        guard let core else { return }
        await recordContacts("Forget Contact", notice: "Forgot what was known about \(contact.name)") { () async throws(CoreClientError) -> ContactChange in
            try await core.forgetContact(contact.personId)
        }
    }

    /// Accept or reject held items; the next one waiting is chosen by the caller.
    func decideContactFacts(_ items: [ContactFactInfo], accept: Bool) async -> CoreClientError? {
        await applyContactEdits(items.map { .decide(id: $0.id, accept: accept) },
                                actionName: accept ? "Accept" : "Reject",
                                notice: accept ? "Accepted" : "Left out; it will not be suggested again")
    }

    /// *Make Task* on a commitment the user owes: a task on the
    /// conversation it was said in (spec §14.8).
    func makeTask(from commitment: ContactFactInfo) async {
        guard let core, let accountID = openAccountID,
              let said = commitment.evidence.last(where: { $0.threadId != nil }), let threadID = said.threadId
        else {
            contactNotice("There is no conversation to make the task on")
            return
        }
        do {
            let category = (try? await core.taskCategories())?.first ?? "Reply"
            let due = commitment.due.map { DueDay.string(Date(timeIntervalSince1970: TimeInterval($0) / 1000)) }
            let made = try await core.createTasks([NewTask(threadId: threadID, messageId: said.messageId,
                                                           title: commitment.label, notes: commitment.value,
                                                           category: category, dueDay: due, action: .noEmail,
                                                           why: "", fromAi: false)])
            recordTaskUndo(made, accountID: accountID)
            await contacts.load()
        } catch let error as CoreClientError {
            contactNotice(error.message)
        } catch {
            contactNotice(error.localizedDescription)
        }
    }

    /// Turn learning about contacts on or off: only the user's act.
    func setContactLearning(_ on: Bool) async {
        guard let core else { return }
        do {
            try await core.setContactLearning(on)
        } catch {
            contactNotice(error.message)
        }
        await contacts.load()
    }

    @discardableResult
    private func recordContacts(_ actionName: String, notice: String,
                                _ change: () async throws(CoreClientError) -> ContactChange) async -> CoreClientError? {
        guard let core else { return nil }
        do {
            let made = try await change()
            if let accountID = openAccountID {
                undo.record(accountID: accountID, actionName: actionName, noticeText: notice,
                            undo: { try? await core.undoContactChange(made.changeId) },
                            redo: { try? await core.redoContactChange(made.changeId) })
            }
            await contacts.load()
            return nil
        } catch {
            return error
        }
    }
}
