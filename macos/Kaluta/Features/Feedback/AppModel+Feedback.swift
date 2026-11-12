import AppKit
import KalutaCore

/// Help › Report a Bug… and Suggest a Feature… (spec §17.1).
extension AppModel {
    /// Why the open account cannot send a report, if it cannot.
    func feedbackCannotSend(_ kind: FeedbackKind) -> String? {
        if openAccountID == nil {
            return "No account is open to send this from. You can write to \(kind.address) from any mail program."
        }
        if isArchive {
            return "This is an imported mailbox; it cannot send mail. Switch to another account, or write to \(kind.address) from any mail program."
        }
        return nil
    }

    /// Take the mail window's picture, then open the sheet over it.
    func openFeedback(_ kind: FeedbackKind) async {
        guard feedbackDraft == nil else { return }
        var picture: CGImage?
        if let window = NSApp.windows.first(where: { $0.isVisible && $0.identifier?.rawValue.hasPrefix("main") == true }) {
            picture = await WindowCapture.image(of: window)
        }
        // The address the mail will be from, as the core will write it.
        let sender = openAccountID == nil ? nil : try? await core?.accountAddress()
        feedbackDraft = makeFeedbackDraft(kind, picture: picture, sender: sender)
    }

    func makeFeedbackDraft(_ kind: FeedbackKind, picture: CGImage?, sender: String? = nil) -> FeedbackDraft {
        let version = FeedbackReport.appVersion
        let account = accounts.first { $0.id == openAccountID }
        let label = openAccountID == nil
            ? FeedbackReport.accountLabel(nil, demo: false)
            : FeedbackReport.accountLabel(account?.kind ?? .gmail, demo: openAccountID == Self.demoAccountID)
        return FeedbackDraft(
            kind: kind, accountID: openAccountID, sender: sender ?? account?.email ?? accountEmail,
            cannotSend: feedbackCannotSend(kind),
            context: FeedbackReport.context(app: version.version, build: version.build, core: core?.version ?? "?",
                                            macOS: FeedbackReport.macOSVersion, account: label),
            log: kind == .bug ? (core?.recentLog(maxLines: FeedbackReport.logLines) ?? "") : "",
            picture: picture)
    }

    /// Send the report as mail from the account the sheet opened on. It
    /// goes through the outbox like any message, Undo Send included, and
    /// ends up in Sent.
    func sendFeedback(_ draft: FeedbackDraft) async {
        guard let core, draft.canSend, let accountID = draft.accountID else { return }
        draft.sending = true
        draft.error = nil
        defer { draft.sending = false }
        var message = DraftInfo.prefilled(to: [draft.kind.address], cc: [],
                                          subject: draft.title.trimmingCharacters(in: .whitespacesAndNewlines),
                                          body: draft.body)
        let files = DraftAttachments(directory: URL(filePath: core.dataDirectory)
            .appending(path: "DraftAttachments", directoryHint: .isDirectory))
        if let picture = draft.pictureToSend {
            let folder = FileManager.default.temporaryDirectory
                .appending(path: "kaluta-report-\(UUID().uuidString)", directoryHint: .isDirectory)
            defer { try? FileManager.default.removeItem(at: folder) }
            let file = folder.appending(path: FeedbackReport.pictureName)
            guard let png = WindowCapture.png(picture),
                  (try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)) != nil,
                  (try? png.write(to: file)) != nil else {
                draft.error = "The picture could not be prepared. Untick it to send without one."
                return
            }
            message.attachments = files.copy([file])
            if message.attachments.isEmpty {
                draft.error = "The picture could not be attached. Untick it to send without one."
                return
            }
        }
        let client = DraftClient(core: core, account: accountID)
        do throws(CoreClientError) {
            let id = try await client.saveDraft(message)
            do throws(CoreClientError) {
                if try await client.sendDraft(id) {
                    sendHeld(draftID: id, accountID: accountID)
                } else {
                    message.attachments.forEach(files.remove)
                    undo.show(draft.kind.sentNotice, accountID: accountID, offersUndo: false)
                }
            } catch {
                // Not left behind as a draft the user never wrote.
                try? await client.deleteDraft(id)
                throw error
            }
            feedbackDraft = nil
        } catch {
            message.attachments.forEach(files.remove)
            draft.error = error.message
        }
    }
}
