import Foundation
import Testing
@testable import Kaluta

@MainActor
struct SyncDebuggerTests {
    private func row(_ job: String, _ via: String, millis: UInt64 = 100, items: UInt32 = 10,
                     note: String? = nil, error: String? = nil) -> TransportComparison {
        TransportComparison(job: job, via: via, note: note, millis: millis, items: items, error: error)
    }

    @Test func comparisonRowsPairUpByJobInOrder() {
        let rows = [row("list", "api"), row("list", "imap"), row("headers", "imap"),
                    row("headers", "api", error: "not measured")]
        let pairs = SyncDebugger.pairs(rows)
        #expect(pairs.map(\.job) == ["list", "headers"])
        #expect(pairs[0].imap?.via == "imap" && pairs[0].api?.via == "api")
        #expect(SyncDebugger.measured(pairs[1].api) == "—", "not measured is a dash, not a failure")
        #expect(SyncDebugger.measured(row("bodies", "imap", error: "timeout")) == "Failed")
        #expect(SyncDebugger.measured(row("bodies", "imap", millis: 1500, items: 500)) == "1.5 s · 500")
    }

    @Test func theFasterWayIsPerItem() {
        let pair = SyncDebugger.Pair(job: "bodies", imap: row("bodies", "imap", millis: 1000, items: 500),
                                     api: row("bodies", "api", millis: 4000, items: 500))
        #expect(SyncDebugger.faster(pair) == "IMAP, ×4.0")
        let lopsided = SyncDebugger.Pair(job: "list", imap: row("list", "imap", millis: 100, items: 100_000),
                                         api: row("list", "api", millis: 2000, items: 10_000))
        #expect(SyncDebugger.faster(lopsided) == "IMAP, ×200")
        let oneWay = SyncDebugger.Pair(job: "changes", imap: nil, api: row("changes", "api"))
        #expect(SyncDebugger.faster(oneWay) == "")
    }

    @Test func notesAreListedOncePerJob() {
        let rows = [row("list", "api", note: "n"), row("list", "imap", note: "n"), row("bodies", "api")]
        #expect(SyncDebugger.notes(rows) == ["List: n"])
    }

    @Test func theBreakerInWords() {
        var d = SyncDiagnostics(syncing: true,
                                backfill: BackfillStatus(transport: "imap", imapBytesToday: 0, storedMessages: 0),
                                breakerOpenUntil: nil, consecutiveImapFailures: 0, lastImapError: nil,
                                latestByJob: [], recent: [], imapCapabilities: [], imapBudgetBytes: 0)
        #expect(SyncDebugger.breaker(d) == "Closed")
        d.consecutiveImapFailures = 3
        let now = Date(timeIntervalSince1970: 1_000)
        d.breakerOpenUntil = 1_000_000 + 15 * 60 * 1000
        #expect(SyncDebugger.breaker(d, now: now).hasPrefix("IMAP paused until"))
        d.breakerOpenUntil = 500_000
        #expect(SyncDebugger.breaker(d, now: now) == "Closed, 3 failures in a row", "past the pause: half-open")
    }
}
