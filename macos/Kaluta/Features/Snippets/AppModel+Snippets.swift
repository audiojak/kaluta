import AppKit

/// Snippets (spec §14.5 amendment 2026-11-18): saved from a message's
/// selected text, and offered by the palette as new messages.
extension AppModel {
    /// Save the composer's selected text as a snippet, named by its first
    /// line; ⌘Z in the mail window deletes it again.
    func saveSnippet(from selection: NSAttributedString) async {
        guard let core else { return }
        let name = SnippetText.name(for: selection.string)
        let snippet = SnippetInfo(id: 0, name: name, bodyHtml: ComposerHTML.html(from: selection), updatedAt: 0)
        guard let id = try? await core.saveSnippet(snippet) else { return }
        let text = "Saved “\(name)” as a snippet. Rename it in Settings › Snippets."
        guard let accountID = openAccountID else { return }
        undo.record(accountID: accountID, actionName: "Save as Snippet", noticeText: text,
                    undo: { _ = try? await core.deleteSnippet(id) },
                    redo: { _ = try? await core.restoreSnippet(SnippetInfo(id: id, name: name,
                                                                          bodyHtml: snippet.bodyHtml, updatedAt: 0)) })
    }

    /// The palette's rows for snippets: a new message starting with one.
    func snippetCommands(_ snippets: [SnippetInfo]) -> [MailCommand] {
        snippets.map { snippet in
            MailCommand(id: "snippet-\(snippet.id)", title: "New Message with “\(snippet.name)”",
                        unavailable: isArchive ? Self.cannotSendReason : nil) {
                let body = ComposerHTML.attributedString(fromHTML: SnippetText.filled(snippet.bodyHtml, recipient: nil)).string
                self.compose(.prefilled(to: [], cc: [], subject: "", body: body))
            }
        }
    }
}
