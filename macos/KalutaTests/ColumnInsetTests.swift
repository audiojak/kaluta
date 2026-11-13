import CoreGraphics
import Testing
@testable import Kaluta

/// The decision bar starts at the reading column's left edge, and the
/// inset that puts it there is never part of the width the bar needs.
struct ColumnInsetTests {
    @Test func withRoomTheContentStartsAtTheColumnsEdge() {
        #expect(ColumnInset.inset(width: 1260, column: 900, needed: 700) == 180)
    }

    @Test func noWiderThanTheColumnThereIsNoInset() {
        #expect(ColumnInset.inset(width: 900, column: 900, needed: 700) == 0)
        #expect(ColumnInset.inset(width: 640, column: 900, needed: 700) == 0)
    }

    @Test func manyButtonsTakeTheInsetBeforeTheyAskForAWiderWindow() {
        // Triage's bar in a 1200-point window: 1136 points of room.
        #expect(ColumnInset.inset(width: 1136, column: 900, needed: 1100) == 36)
        #expect(ColumnInset.inset(width: 1136, column: 900, needed: 1136) == 0)
    }
}
