import Foundation
import Testing
import UserNotifications
@testable import OpenAGC

/// Analysis settings and its notification (spec §14.10).
@MainActor
struct AnalysisSettingsTests {
    @Test func theNotificationIsOptInOnceADayAndOnlyInTheBackground() throws {
        let defaults = try #require(UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)"))
        var posted: [UNNotificationRequest] = []
        let notifier = NewMailNotifier(defaults: defaults, post: { posted.append($0) })
        notifier.isAppActive = { false }
        notifier.announceAnalysis(proposals: 3, accountID: "a", today: "2026-10-05")
        #expect(posted.isEmpty, "off unless turned on")
        defaults.set(true, forKey: NewMailNotifier.analysisKey)
        notifier.isAppActive = { true }
        notifier.announceAnalysis(proposals: 3, accountID: "a", today: "2026-10-05")
        #expect(posted.isEmpty, "not while OpenAGC is in front")
        notifier.isAppActive = { false }
        notifier.announceAnalysis(proposals: 3, accountID: "a", today: "2026-10-05")
        notifier.announceAnalysis(proposals: 1, accountID: "a", today: "2026-10-05")
        #expect(posted.map(\.content.body) == ["3 new proposals in Analysis"], "once a day")
        #expect(posted.first?.content.userInfo["analysis"] as? Bool == true)
        notifier.announceAnalysis(proposals: 1, accountID: "a", today: "2026-10-06")
        #expect(posted.count == 2)
    }

    @Test func settingsAreKeptPerAccountAndTheMenuShowsADot() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        var settings = try await core.analysisSettings()
        #expect(settings.keepDays == 30 && settings.factsFrom == .mailWrittenWithAi && settings.dailyReview)
        settings.keepDays = 7
        settings.factsFrom = .off
        try await core.setAnalysisSettings(settings)
        #expect(try await core.analysisSettings() == settings)

        _ = try await core.startGuideRun(GuideRunRequest(kind: .latest, count: 20,
                                                          filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                          focus: nil, agent: model.agent.providerID))
        for _ in 0..<200 where (try await core.guideProgress()).run?.status != .done {
            try await Task.sleep(for: .milliseconds(50))
        }
        try await core.debugSeedAnalysis()
        await model.refreshAnalysisDots()
        #expect(model.unseenAnalysisAccounts == [model.openAccountID ?? ""])
        model.openAnalysis()
        await model.analysisShown()
        #expect(model.unseenAnalysisAccounts.isEmpty, "seen: the dot goes")
    }
}
