import Foundation
import os

/// The workflow demo (`-KalutaDemoWorkflows YES`, docs/plans/design-review.md):
/// everything the daily workflows need, made once on a fresh scratch data
/// directory with the app's own calls: tasks on the hand-written
/// conversations, a learned writing guide with proposed rules, facts with
/// proposals, an agent mailbox with sign-up mail, and a local sorting
/// routine that has run once. Mail then keeps arriving.
extension AppModel {
    func prepareDemoWorkflows(fresh: Bool) async {
        guard let core else { return }
        if fresh {
            await agent.loadProviders()
            await seedWorkflowTasks()
            await seedWorkflowGuide()
            await seedWorkflowFacts()
            await seedWorkflowRoutine()
            await seedWorkflowAgentMailbox()
        }
        do {
            try core.startDemoArrivals(every: CoreClient.demoArrivalSeconds)
        } catch {
            Logger(subsystem: "org.kaluta.Kaluta", category: "demo")
                .error("demo arrivals: \(error.message, privacy: .public)")
        }
    }

    /// Tasks on the conversations that call for them, due at different
    /// times, one overdue and one done (spec §14.8).
    private func seedWorkflowTasks(now: Date = .now) async {
        guard let core else { return }
        let plan: [(thread: String, title: String, category: String, due: Int?, action: TaskAction, why: String)] = [
            ("demowf-board-prep", "Pick a time for board prep", "Schedule", 0, .reply,
             "Sam is waiting to send the invite."),
            ("demowf-designer", "Decide on the designer offer", "Decide", 0, .reply,
             "Taylor asks whether to start on the offer."),
            ("demowf-redlines", "Answer Casey's two contract questions", "Review", 1, .reply,
             "The liability cap and notice period are open."),
            ("demowf-renewal", "Answer Morgan on two-year pricing", "Decide", 3, .reply,
             "Their finance team needs an answer by Friday."),
            ("demowf-declined", "Update the card on Example Cloud", "Admin", -1, .noEmail,
             "The payment was declined; the subscription stops in 7 days."),
            ("demowf-pilot", "Chase Reese about the pilot", "Follow Up", nil, .reply,
             "No answer to the proposal for six days."),
            ("demowf-thanks", "Send Rowan the workshop slides", "Reply", -1, .reply,
             "Rowan asked for them after the workshop."),
        ]
        let calendar = Calendar.current
        let made = (try? await core.createTasks(plan.map { p in
            NewTask(threadId: p.thread, messageId: nil, title: p.title, notes: "", category: p.category,
                    dueDay: p.due.flatMap { calendar.date(byAdding: .day, value: $0, to: now) }.map { DueDay.string($0) },
                    action: p.action, why: p.why, fromAi: true)
        })) ?? []
        if let done = made.first(where: { $0.title.hasPrefix("Send Rowan") }) {
            _ = try? await core.setTaskDone(done.id, true)
        }
        await tasks.load()
    }

    /// A writing guide learned from the sent mail, two decisions accepted,
    /// and a day of reviews proposing rules (spec §14.9, §14.10).
    private func seedWorkflowGuide() async {
        guard let core else { return }
        _ = try? await core.startGuideRun(GuideRunRequest(kind: .latest, count: 40,
                                                          filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                          focus: nil, agent: agent.providerID))
        for _ in 0..<100 where (try? await core.guideProgress())?.run?.status != .done {
            try? await Task.sleep(for: .milliseconds(100))
        }
        for entry in ((try? await core.guideDecisions()) ?? []).prefix(2) {
            _ = try? await core.applyGuideEdits([.decide(id: entry.id, status: .accepted)], reason: "demo")
        }
        try? await core.debugSeedAnalysis()
        guideProgress = try? await core.guideProgress()
        analysisProgress = try? await core.analysisProgress()
        await guide.load()
        await analysis.load()
    }

    /// Facts the user gave, one global, and proposed facts (spec §14.11).
    private func seedWorkflowFacts() async {
        guard let core else { return }
        func fact(_ c: String, _ l: String, _ v: String, _ u: FactUse = .free) -> FactEdit {
            .add(fields: FactFields(category: c, label: l, value: v, use: u, asOf: nil), status: .accepted, source: .you)
        }
        let made = try? await core.applyFactEdits([
            fact("identity", "Preferred name", "Jo"), fact("identity", "Pronouns", "they/them"),
            fact("availability", "Time zone", "Pacific"),
            fact("availability", "Calendar link", "https://cal.example.com/jo"),
            fact("people", "Sam Rivera", "My assistant", .ask), fact("work", "Occupation or role", "Founder"),
            fact("contact", "Mailing address", "1 Main St", .never),
        ], reason: "demo")
        if let id = made?.facts.first(where: { $0.label == "Time zone" })?.id {
            _ = try? await core.makeFactGlobal(id)
        }
        try? await core.debugSeedFactProposals()
        await facts.load()
        await analysis.load()
    }

    /// The sorting routine, run once on this Mac so it has a run to look
    /// at and undo (spec §11.6).
    private func seedWorkflowRoutine() async {
        guard let core, let routine = try? await core.createRoutineFromTemplate(runner: RoutineRunner.local.rawValue)
        else { return }
        _ = try? await core.runRoutineNow(routine.id)
        await routines.load()
    }

    /// An agent mailbox with the mail a sign-up brings (spec §7.9), then
    /// back to the demo mailbox.
    private func seedWorkflowAgentMailbox() async {
        guard let core, CoreClient.usesFakeAgentMail,
              let scout = try? await createAgentMailbox(name: "Research Scout") else { return }
        try? core.deliverToAgentMailbox(scout.accountId, from: "Example Library <hello@example.org>",
                                        subject: "Confirm your sign-up",
                                        body: "Thanks for signing up. Confirm your address within 24 hours to activate your library card.")
        try? core.deliverToAgentMailbox(scout.accountId, from: "Example Journals <codes@example.org>",
                                        subject: "Your verification code",
                                        body: "Your verification code is 482 913. It expires in 10 minutes.")
        await switchAccount(to: Self.demoAccountID)
    }
}
