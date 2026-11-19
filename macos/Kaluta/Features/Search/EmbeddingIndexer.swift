import Foundation
import NaturalLanguage

/// Search by meaning (spec §14.3 amendment 2026-11-18, ADR 0023): each
/// message's subject and snippet as a vector from Apple's on-device
/// sentence model, computed here and kept by the core. Nothing leaves the
/// Mac. It works through the backlog newest first, a batch at a time,
/// with pauses, while the app runs.
@MainActor
final class EmbeddingIndexer {
    /// The model's name in the store: a change of model starts over.
    static let model = "apple-sentence-en-1"
    static let batch: UInt32 = 64
    static let pause: Duration = .seconds(2)

    private let core: CoreClient
    private var task: Task<Void, Never>?

    init(core: CoreClient) {
        self.core = core
    }

    /// The sentence model, if the Mac has it.
    nonisolated static func embedding() -> NLEmbedding? {
        NLEmbedding.sentenceEmbedding(for: .english)
    }

    /// A text's vector, or nil when the model is missing or the text says nothing.
    nonisolated static func vector(for text: String, using embedding: NLEmbedding?) -> [Float]? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let embedding, let vector = embedding.vector(for: String(text.prefix(1_000))) else {
            return nil
        }
        return vector.map(Float.init)
    }

    /// Work through the backlog until it is empty, then stop.
    func start() {
        task?.cancel()
        let core = core
        task = Task.detached(priority: .background) {
            guard let embedding = Self.embedding() else { return }
            while !Task.isCancelled {
                guard let todo = try? await core.embeddingBacklog(model: Self.model, limit: Self.batch), !todo.isEmpty
                else { return }
                let vectors = todo.compactMap { item in
                    Self.vector(for: item.text, using: embedding).map { EmbeddingVector(messageId: item.messageId, vector: $0) }
                }
                // A batch with nothing to embed would come back: stop here.
                guard !vectors.isEmpty, (try? await core.storeEmbeddings(model: Self.model, vectors)) != nil else { return }
                try? await Task.sleep(for: Self.pause)
            }
        }
    }

    func stop() {
        task?.cancel()
        task = nil
    }

    /// Conversations near a search's meaning; none without the model.
    func related(to query: String, limit: UInt32 = 20) async -> [ThreadRow] {
        guard let vector = Self.vector(for: query, using: Self.embedding()) else { return [] }
        return (try? await core.searchByMeaning(model: Self.model, vector: vector, limit: limit)) ?? []
    }
}
