import Foundation
import Testing
@testable import OpenAGC

/// Store reads one at a time, and never stale after `await load()`.
@MainActor
struct SerialReadsTests {
    @Test func aLoadDuringAReadReturnsOnlyAfterAFreshRead() async throws {
        let reads = SerialReads()
        var source = 1
        var shown = 0
        var count = 0
        let read: @MainActor () async -> Void = {
            count += 1
            let value = source
            try? await Task.sleep(for: .milliseconds(50))
            shown = value
        }
        let first = Task { await reads.run(read) }
        try await Task.sleep(for: .milliseconds(10))
        // Changed while the first read is under way, then loaded again.
        source = 2
        await reads.run(read)
        #expect(shown == 2, "the second caller sees the change")
        await first.value
        #expect(count == 2, "one more read, not one per caller")
        await reads.run(read)
        #expect(count == 3, "a load after the reads have finished reads again")
    }
}
