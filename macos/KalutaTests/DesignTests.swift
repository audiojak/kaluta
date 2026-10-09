import AppKit
import Testing
@testable import OpenAGC

struct DesignTokenTests {
    @Test func spacingIsOneIncreasingScale() {
        let scale = [Space.hair, Space.xs, Space.s, Space.m, Space.l, Space.xl, Space.xxl, Space.xxxl, Space.page]
        #expect(scale == [2, 4, 6, 8, 12, 16, 20, 24, 32])
    }

    @Test func labelChipsTakeTheLabelsColourFaintly() throws {
        let red = try #require(Tone.chipFill(hex: "#ff0000").usingColorSpace(.sRGB))
        #expect(red.redComponent == 1 && red.greenComponent == 0)
        #expect(abs(red.alphaComponent - Tone.chipFillOpacity) < 0.001)
        let plain = Tone.chipFill(hex: nil)
        #expect(abs(plain.alphaComponent - Tone.chipFillOpacity) < 0.001, "a label without a colour still gets a faint chip")
    }
}

@MainActor
struct SenderLineTests {
    private func row(_ people: [(String?, String)], messages: UInt32 = 1) -> ThreadRow {
        ThreadRow(id: "t", subject: "s", snippet: "", lastMessageAt: 0, messageCount: messages, unreadCount: 0,
                  hasAttachments: false, isStarred: false,
                  participants: people.map { AddressInfo(name: $0.0, email: $0.1) }, labelIds: [], replied: false)
    }

    private let me: Set<String> = ["john@actual.ai", "john.kennedy@alias.example"]

    @Test func youAreLeftOutAndOthersNamedLikeMail() {
        #expect(ThreadRowView.senderLine(row([("John Kennedy", "John@Actual.ai"), ("Matthew Watts", "mw@x.com")], messages: 3),
                                         me: me) == "Matthew Watts")
        #expect(ThreadRowView.senderLine(row([("Jeffrey Priebe", "j@x.com"), ("Andre Corr", "a@x.com"),
                                              ("John Kennedy", "john.kennedy@alias.example")]), me: me) == "Jeffrey & Andre")
        #expect(ThreadRowView.senderLine(row([("Himanshi Verma", "h@x"), ("Le, Minh", "m@x"), (nil, "austin@x.com"),
                                              ("D P", "d@x")], messages: 21), me: me) == "Himanshi, Minh, austin …")
        #expect(ThreadRowView.senderLine(row([("John Kennedy", "john@actual.ai")], messages: 2), me: me) == "Me")
        #expect(ThreadRowView.senderLine(row([]), me: me) == "(unknown sender)")
    }

    @Test func theCountStandsApartAndOnlyForThreads() {
        #expect(ThreadRowView.countText(row([("A", "a@x")], messages: 3)) == "3")
        #expect(ThreadRowView.countText(row([("A", "a@x")])) == nil)
    }
}

struct DueDayTests {
    private let calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/Los_Angeles")!
        c.locale = Locale(identifier: "en_US")
        return c
    }()

    /// Monday 2026-09-28, late evening.
    private var now: Date { calendar.date(from: DateComponents(year: 2026, month: 9, day: 28, hour: 23))! }

    @Test func daysFallIntoTheListsGroups() {
        func group(_ day: String?) -> DueDay.Group { DueDay.group(day, now: now, calendar: calendar) }
        #expect(group("2026-09-27") == .overdue)
        #expect(group("2026-09-28") == .today)
        #expect(group("2026-09-29") == .thisWeek)
        #expect(group("2026-10-04") == .thisWeek)
        #expect(group("2026-10-05") == .later)
        #expect(group(nil) == .noDate)
        #expect(group("soon") == .noDate)
        #expect(DueDay.Group.allCases.map(\.title) == ["Overdue", "Today", "This Week", "Later", "No Date"])
    }

    @Test func labelsReadAsPeopleSayThem() {
        func label(_ day: String?) -> String { DueDay.label(day, now: now, calendar: calendar) }
        #expect(label("2026-09-28") == "Today")
        #expect(label("2026-09-29") == "Tomorrow")
        #expect(label("2026-09-27") == "Yesterday")
        #expect(label("2026-10-01") == "Thursday")
        #expect(label("2026-10-12") == "Oct 12")
        #expect(label("2027-01-03") == "Jan 3, 2027")
        #expect(label(nil) == "No date")
        #expect(DueDay.urgency("2026-09-20", now: now, calendar: calendar) == .overdue)
    }

    @Test func daysRoundTrip() throws {
        let date = try #require(DueDay.date("2026-02-03", calendar: calendar))
        #expect(DueDay.string(date, calendar: calendar) == "2026-02-03")
    }
}

struct CategoryToneTests {
    @Test func aCategoryKeepsItsColourAndNeverTakesAStatusColour() {
        #expect(Tone.category("Reply") == Tone.category("reply"))
        let status: [NSColor] = [.systemRed, .systemOrange, .systemYellow, .systemPink]
        for name in ["Reply", "Decide", "Gather Info", "Schedule", "Review", "Admin", "Follow Up", "x"] {
            #expect(!status.contains(Tone.category(name)))
        }
        let distinct = Set(["Reply", "Decide", "Gather Info", "Schedule", "Review", "Admin", "Follow Up"]
            .map { Tone.category($0) })
        #expect(distinct.count == 7, "the starting set is told apart by colour")
    }
}
