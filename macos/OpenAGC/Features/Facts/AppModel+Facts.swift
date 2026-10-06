import AppKit
import Foundation
import UniformTypeIdentifiers

/// Facts (spec §14.11): changing them, each change on the account's undo
/// stack (ADR 0006).
extension AppModel {
    /// Open Analysis on its Facts tab.
    func openFacts() {
        openAnalysis()
        analysis.showsFacts = true
    }

    /// Apply fact edits in `scope` as one change.
    @discardableResult
    func applyFactEdits(_ edits: [FactEdit], scope: FactScope = .account, actionName: String,
                        notice: String) async -> CoreClientError? {
        guard let core else { return CoreClientError(kind: .notFound, message: "No account is open") }
        return await recordFacts(actionName, notice: notice, global: scope == .global) { () async throws(CoreClientError) -> FactChange in
            scope == .global ? try await core.applyGlobalFactEdits(edits, reason: actionName)
                : try await core.applyFactEdits(edits, reason: actionName)
        }
    }

    @discardableResult
    func editFactCategories(_ edits: [CategoryEdit], scope: FactScope = .account, actionName: String,
                            notice: String) async -> CoreClientError? {
        guard let core else { return CoreClientError(kind: .notFound, message: "No account is open") }
        return await recordFacts(actionName, notice: notice, global: scope == .global) { () async throws(CoreClientError) -> FactChange in
            scope == .global ? try await core.editGlobalFactCategories(edits) : try await core.editFactCategories(edits)
        }
    }

    @discardableResult
    func addFactStarterSet(_ set: StarterSetInfo) async -> CoreClientError? {
        guard let core else { return nil }
        return await recordFacts("Add \(set.name) Categories", notice: "Added the \(set.name) categories", global: false) { () async throws(CoreClientError) -> FactChange in
            try await core.addFactStarterSet(set.set)
        }
    }

    /// Make Global, or Make This Account's Only: undone on this account's stack.
    func moveFact(_ fact: FactInfo) async {
        guard let core else { return }
        let global = fact.scope == .account
        await recordFacts(global ? "Make Global" : "Make This Account's Only",
                          notice: global ? "Every account now uses “\(fact.label)”" : "Only this account uses “\(fact.label)”",
                          global: false) { () async throws(CoreClientError) -> FactChange in
            global ? try await core.makeFactGlobal(fact.id) : try await core.makeFactLocal(fact.id)
        }
    }

    func deleteFact(_ fact: FactInfo) async {
        await applyFactEdits([.delete(id: fact.id)], scope: fact.scope, actionName: "Delete Fact",
                             notice: "Deleted “\(fact.label)”")
    }

    /// Accept or reject fact proposals from Analysis: one change.
    func decideFactProposals(_ proposals: [AnalysisFactProposalInfo], accept: Bool) async {
        guard let core, !proposals.isEmpty else { return }
        await recordFacts(accept ? "Accept Fact" : "Reject Fact",
                          notice: accept ? "Added to your facts" : "Left out; it will not be proposed again",
                          global: false) { () async throws(CoreClientError) -> FactChange in
            try await core.decideFactProposals(proposals.map(\.id), accept: accept)
        }
        await analysisChanged()
    }

    /// Save this account's facts as Markdown or JSON (spec §14.11).
    func exportFacts(json: Bool) {
        Task {
            guard let core, let text = try? await core.exportFacts(json: json) else { return }
            let panel = NSSavePanel()
            panel.nameFieldStringValue = json ? "Facts.json" : "Facts.md"
            panel.allowedContentTypes = json ? [.json] : [UTType(filenameExtension: "md") ?? .plainText]
            guard panel.runModal() == .OK, let url = panel.url else { return }
            try? text.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    /// Merge another account's exported facts: new ones added (undoable),
    /// differences proposed in Analysis.
    func mergeFactsFromFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url, let json = try? String(contentsOf: url, encoding: .utf8)
        else { return }
        Task { await mergeFacts(json) }
    }

    func mergeFacts(_ json: String) async {
        guard let core, let accountID = openAccountID else { return }
        do {
            let result = try await core.mergeFacts(json)
            var parts = ["Added \(result.added) \(result.added == 1 ? "fact" : "facts")"]
            if result.proposed > 0 { parts.append("\(result.proposed) that differ wait in Analysis") }
            let notice = parts.joined(separator: "; ")
            if result.changeId != 0 {
                undo.record(accountID: accountID, actionName: "Merge Facts", noticeText: notice,
                            undo: { try? await core.undoFactChange(result.changeId) },
                            redo: { try? await core.redoFactChange(result.changeId) })
            }
            analysisError = nil
            await facts.load()
            await analysisChanged()
        } catch {
            analysisError = error.message
        }
    }

    @discardableResult
    private func recordFacts(_ actionName: String, notice: String, global: Bool,
                             _ change: () async throws(CoreClientError) -> FactChange) async -> CoreClientError? {
        guard let core, let accountID = openAccountID else { return nil }
        do {
            let made = try await change()
            undo.record(accountID: accountID, actionName: actionName, noticeText: notice,
                        undo: {
                            if global { try? await core.undoGlobalFactChange(made.changeId) } else {
                                try? await core.undoFactChange(made.changeId)
                            }
                        },
                        redo: {
                            if global { try? await core.redoGlobalFactChange(made.changeId) } else {
                                try? await core.redoFactChange(made.changeId)
                            }
                        })
            await facts.load()
            return nil
        } catch {
            return error
        }
    }
}
