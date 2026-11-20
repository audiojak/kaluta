import Foundation
import Testing
@testable import Kaluta

/// Routines on rows and in their list (spec §11.5 amendment 2026-11-18).
@MainActor
struct RoutineSurfaceTests {
    private func definition(_ buckets: [String]) async throws -> RoutineDefinition {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        let info = try await #require(model.core).createRoutineFromTemplate(runner: "local")
        var d = try RoutineDefinition.decode(info.definitionJson)
        d.parentLabel = "Kaluta"
        d.buckets = buckets.enumerated().map { i, title in
            var b = RoutineDefinition.Bucket.new(order: i + 1)
            b.title = title
            b.labelName = "\(i + 1)-\(title)"
            b.color = i == 0 ? "blue" : "green"
            return b
        }
        return d
    }

    @Test func aRoutineIsSaidInOneSentence() async throws {
        #expect(RoutinesStore.summary(try await definition([])) == "Files nothing yet: it has no buckets.")
        #expect(RoutinesStore.summary(try await definition(["Newsletters"])) == "Files mail into Newsletters.")
        #expect(RoutinesStore.summary(try await definition(["A", "B", "C", "D", "E"])) == "Files mail into A, B and 3 more.")
        // The bucket labels under the parent, with their colours.
        let d = try await definition(["Newsletters"])
        let info = RoutineInfo(id: "r", name: "Sort", enabled: true, runner: "local", definitionJson: try d.encoded(),
                               changedSincePublish: false, cloudUrl: nil, updatedAt: 0)
        #expect(AppModel.routineBucketColors([info]) == ["kaluta/1-newsletters": "#007AFF"])
        var paused = info
        paused.enabled = false
        #expect(AppModel.routineBucketColors([paused]).isEmpty, "a paused routine's labels are plain")
    }

    @Test func routineBucketsComeFirstInTheirColour() throws {
        let chips: [String: ThreadRowView.Chip] = [
            "L1": .init(path: "Archive/Old", color: nil),
            "L2": .init(path: "Kaluta/1-Newsletters", color: "#007AFF", routine: true),
            "L3": .init(path: "Clients", color: nil),
        ]
        let row = ThreadRow(id: "t", subject: "", snippet: "", lastMessageAt: 0, messageCount: 1, unreadCount: 0,
                            hasAttachments: false, isStarred: false, participants: [], labelIds: ["L1", "L2", "L3"],
                            replied: false)
        let shown = ThreadRowView.chips(for: row, labels: chips, excluding: nil)
        #expect(shown.map(\.path) == ["Kaluta/1-Newsletters", "Archive/Old", "Clients"])
        #expect(RoutineEditorColors.hex("blue") == "#007AFF" && RoutineEditorColors.hex("nope") == "#8E8E93")
    }
}
