import Foundation
import Testing
@testable import Kaluta

/// Search by meaning (spec §14.3 amendment 2026-11-18, ADR 0023).
@MainActor
struct MeaningSearchTests {
    @Test func theOnDeviceModelGivesVectorsWhereTheMacHasIt() throws {
        let embedding = EmbeddingIndexer.embedding()
        #expect(EmbeddingIndexer.vector(for: "   ", using: embedding) == nil, "nothing to embed")
        guard embedding != nil else { return } // not every Mac has the English model
        let invoice = try #require(EmbeddingIndexer.vector(for: "the invoice for March", using: embedding))
        #expect(!invoice.isEmpty && invoice.allSatisfy(\.isFinite))
    }

    @Test func vectorsAreKeptAndTheNearestConversationIsFound() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let todo = try await core.embeddingBacklog(model: "test", limit: 3)
        #expect(todo.count == 3 && todo.allSatisfy { !$0.text.isEmpty })
        // One message points one way, the others another.
        let vectors = todo.enumerated().map { i, item in
            EmbeddingVector(messageId: item.messageId, vector: i == 0 ? [1, 0, 0] : [0, 1, 0])
        }
        #expect(try await core.storeEmbeddings(model: "test", vectors) == 3)
        let found = try await core.searchByMeaning(model: "test", vector: [1, 0, 0])
        #expect(found.count == 1)
        let next = try await core.embeddingBacklog(model: "test", limit: 3)
        #expect(Set(next.map(\.messageId)).isDisjoint(with: Set(todo.map(\.messageId))), "not asked again")
    }

    /// Matches by meaning come after the matches by words, marked Related,
    /// and never for operator searches.
    @Test func relatedResultsFollowTheWordMatches() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let anyRow = try #require(model.threads.rows.first)
        model.threads.meaning = { _ in [anyRow] }
        model.threads.search("zqxv nothing matches these words")
        await model.threads.searchSettled()
        #expect(model.threads.rows.map(\.id) == [anyRow.id])
        #expect(model.threads.related == [anyRow.id])
        let before = model.threads.rows
        await model.threads.addRelated("from:sam")
        #expect(model.threads.rows == before, "operators ask for exact matches")
    }
}
