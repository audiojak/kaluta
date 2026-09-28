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
                  participants: people.map { AddressInfo(name: $0.0, email: $0.1) }, labelIds: [])
    }

    private let me: Set<String> = ["john@actual.ai", "john.kennedy@alias.example"]

    @Test func youAreLeftOutAndOthersNamedLikeMail() {
        #expect(ThreadRowView.senderLine(row([("John Kennedy", "John@Actual.ai"), ("Matthew Watts", "mw@x.com")], messages: 3),
                                         me: me) == "Matthew Watts (3)")
        #expect(ThreadRowView.senderLine(row([("Jeffrey Priebe", "j@x.com"), ("Andre Corr", "a@x.com"),
                                              ("John Kennedy", "john.kennedy@alias.example")]), me: me) == "Jeffrey & Andre")
        #expect(ThreadRowView.senderLine(row([("Himanshi Verma", "h@x"), ("Le, Minh", "m@x"), (nil, "austin@x.com"),
                                              ("D P", "d@x")], messages: 21), me: me) == "Himanshi, Minh, austin … (21)")
        #expect(ThreadRowView.senderLine(row([("John Kennedy", "john@actual.ai")], messages: 2), me: me) == "Me (2)")
        #expect(ThreadRowView.senderLine(row([]), me: me) == "(unknown sender)")
    }
}
