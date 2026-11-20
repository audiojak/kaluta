import Foundation
import Testing
@testable import Kaluta

/// The Dashboard's catch-up line (spec §14.13 amendment 2026-11-18).
@MainActor
struct CatchUpTests {
    @Test func theLineSaysWhatCameInSinceAndWhatNeedsTheUser() {
        let now = Date.now
        let earlier = Calendar.current.startOfDay(for: now).addingTimeInterval(9 * 3600)
        let since = min(earlier, now.addingTimeInterval(-60))
        let at = since.formatted(date: .omitted, time: .shortened)
        let c = { (new: UInt32, needs: UInt32) in CatchUp(since: CoreClient.millis(since), new: new, needsYou: needs) }
        #expect(StartScreenView.catchUpLine(c(18, 3), now: now) == "18 new since \(at) · 3 need you")
        #expect(StartScreenView.catchUpLine(c(1, 1), now: now) == "1 new since \(at) · 1 needs you")
        #expect(StartScreenView.catchUpLine(c(4, 0), now: now) == "4 new since \(at)")
        #expect(StartScreenView.catchUpLine(c(0, 0), now: now) == nil, "nothing came in: no line")
    }

    @Test func lookingMovesTheLineOn() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let before = try await core.catchUp()
        try await core.markLooked()
        let after = try await core.catchUp()
        #expect(after.since > before.since)
        #expect(after.new <= before.new)
    }
}
