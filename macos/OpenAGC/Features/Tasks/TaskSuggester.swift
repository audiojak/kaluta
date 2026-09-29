import Foundation
import Observation

/// Asks Claude what to do about some emails (spec §14.8): one turn in a
/// session of its own that can see only those threads. Anything it
/// proposes that would change mail is refused; it only answers. The answer
/// is read by the core (`parseTaskSuggestions`). Used by the task dialog
/// (`t`) and the bulk sheet (`⇧T`).
@MainActor
@Observable
final class TaskSuggester {
    enum State: Equatable {
        case idle, working, done([TaskSuggestion]), failed(String)
    }

    private(set) var state: State = .idle
    /// Told when suggestions arrive (the dialog fills its fields).
    @ObservationIgnored var onDone: (([TaskSuggestion]) -> Void)?

    @ObservationIgnored private var sessionID: String?
    @ObservationIgnored private var threadIDs: [String] = []
    @ObservationIgnored private var reply = ""
    @ObservationIgnored private weak var model: AppModel?

    var suggestions: [TaskSuggestion] {
        if case let .done(found) = state { return found }
        return []
    }

    /// Whether Claude (or the chosen agent) can be asked at all; without
    /// one the dialog opens empty for the user to fill in.
    static func canAsk(_ model: AppModel) -> Bool {
        model.core != nil && model.agent.isProviderReady
    }

    func run(threadIDs: [String], model: AppModel, now: Date = .now) async {
        guard !threadIDs.isEmpty, state != .working, let core = model.core else { return }
        self.model = model
        self.threadIDs = threadIDs
        state = .working
        reply = ""
        do {
            let prompt = try await core.taskPrompt(threadIDs, today: DueDay.string(now))
            let session = try await core.startAgentSession(provider: model.agent.providerID, selection: threadIDs)
            sessionID = session
            model.agentSinks[session] = { [weak self] events in await self?.ingest(events) }
            try await core.sendAgentPrompt(session, prompt)
        } catch let error as CoreClientError {
            fail(error.message)
        } catch {
            fail(error.localizedDescription)
        }
    }

    func cancel() {
        guard state == .working else { return }
        if let core = model?.core, let sessionID { Task { try? await core.cancelAgentTurn(sessionID) } }
        finish()
        state = .idle
    }

    func ingest(_ events: [AgentEventInfo]) async {
        for event in events {
            switch event {
            case let .textDelta(text):
                reply += text
            case let .actionProposed(actionID, _, _, _):
                // Suggesting tasks changes no mail: refuse anything that would.
                try? model?.core?.resolveAgentAction(actionID, approve: false)
            case .turnCompleted:
                await apply()
            case let .turnFailed(message):
                fail(message)
            default:
                break
            }
        }
    }

    private func apply() async {
        guard state == .working, let core = model?.core else { return }
        let text = reply
        let ids = threadIDs
        finish()
        do {
            let found = try await core.parseTaskSuggestions(text, threadIDs: ids)
            state = .done(found)
            onDone?(found)
        } catch let error as CoreClientError {
            state = .failed(error.message)
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private func fail(_ message: String) {
        state = .failed(message)
        finish()
    }

    private func finish() {
        if let sessionID {
            model?.agentSinks[sessionID] = nil
            if let core = model?.core { Task { try? await core.closeAgentSession(sessionID) } }
        }
        sessionID = nil
    }
}
