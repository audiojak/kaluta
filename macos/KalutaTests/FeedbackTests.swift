import AppKit
import Foundation
import Testing
@testable import Kaluta

/// Help › Report a Bug… and Suggest a Feature… (spec §17.1).
@MainActor
struct FeedbackTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        return model
    }

    /// A picture of one colour, `width` by `height` pixels.
    private func picture(width: Int, height: Int, gray: CGFloat = 1) throws -> CGImage {
        let context = try #require(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                             space: CGColorSpaceCreateDeviceRGB(),
                                             bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(gray: gray, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        return try #require(context.makeImage())
    }

    /// The red component at a pixel counted from the top left.
    private func red(_ image: CGImage, x: Int, y: Int) throws -> UInt8 {
        let rep = NSBitmapImageRep(cgImage: image)
        let color = try #require(rep.colorAt(x: x, y: y))
        return UInt8((color.redComponent * 255).rounded())
    }

    // MARK: The message

    @Test func theBodyIsTheDescriptionThenTheBlocksTheHandlerReads() {
        let context = FeedbackReport.context(app: "0.1.0", build: "1", core: "0.1.0", macOS: "26.1.0", account: "Gmail")
        let body = FeedbackReport.body(description: "  The list jumps.\n", context: context, log: "WARN a\nERROR b\n")
        #expect(body == """
        The list jumps.

        --- Kaluta report ---
        App: 0.1.0 (1)
        Core: 0.1.0
        macOS: 26.1.0
        Account: Gmail
        --- end ---

        --- Kaluta log ---
        WARN a
        ERROR b
        --- end ---

        """)
    }

    @Test func aBlockWithNothingInItIsLeftOut() {
        #expect(FeedbackReport.body(description: "Snooze until Monday.", context: [], log: " \n") == "Snooze until Monday.\n")
        #expect(!FeedbackReport.body(description: "", context: ["App: 1 (1)"], log: "").contains(FeedbackReport.logMarker))
    }

    @Test func theKindChoosesTheAddress() {
        #expect(FeedbackKind.bug.address == "bug@support.kaluta.org")
        #expect(FeedbackKind.idea.address == "idea@support.kaluta.org")
    }

    @Test func theAccountIsNamedByKindOnly() {
        #expect(FeedbackReport.accountLabel(.gmail, demo: false) == "Gmail")
        #expect(FeedbackReport.accountLabel(.agent, demo: false) == "Agent mailbox")
        #expect(FeedbackReport.accountLabel(.archive, demo: false) == "Imported mailbox")
        #expect(FeedbackReport.accountLabel(.gmail, demo: true) == "Demo")
    }

    // MARK: The picture

    @Test func aCoverReplacesThePixelsUnderIt() throws {
        let white = try picture(width: 100, height: 80)
        // The top-left quarter.
        let covered = try #require(FeedbackReport.covered(white, by: [CGRect(x: 0, y: 0, width: 0.5, height: 0.5)]))
        #expect(covered.width == 100 && covered.height == 80)
        #expect(try red(covered, x: 10, y: 10) == 0, "covered")
        #expect(try red(covered, x: 49, y: 39) == 0, "covered to its edge")
        #expect(try red(covered, x: 60, y: 10) == 255, "beside it")
        #expect(try red(covered, x: 10, y: 60) == 255, "below it")
    }

    @Test func coversOutsideThePictureAreIgnoredAndNoCoversChangeNothing() throws {
        let white = try picture(width: 40, height: 40)
        #expect(FeedbackReport.covered(white, by: []) === white)
        let covered = try #require(FeedbackReport.covered(white, by: [CGRect(x: 2, y: 2, width: 1, height: 1),
                                                                    CGRect(x: 0.9, y: 0.9, width: 0.5, height: 0.5)]))
        #expect(try red(covered, x: 5, y: 5) == 255)
        #expect(try red(covered, x: 38, y: 38) == 0, "the part inside is covered")
    }

    @Test func aDragBecomesACoverInsideThePicture() {
        let size = CGSize(width: 200, height: 100)
        let cover = PictureCovers.cover(from: CGPoint(x: 150, y: 80), to: CGPoint(x: 50, y: 20), in: size)
        #expect(cover == CGRect(x: 0.25, y: 0.2, width: 0.5, height: 0.6))
        let clipped = PictureCovers.cover(from: CGPoint(x: 100, y: 50), to: CGPoint(x: 400, y: 400), in: size)
        #expect(clipped == CGRect(x: 0.5, y: 0.5, width: 0.5, height: 0.5))
        #expect(PictureCovers.cover(from: CGPoint(x: 10, y: 10), to: CGPoint(x: 11, y: 40), in: size) == nil, "a slip")
    }

    @Test func aPictureIsSentNoLargerThanTheLimit() throws {
        let size = WindowCapture.fitted(width: 4800, height: 2400)
        #expect(size.width == WindowCapture.longestSide && size.height == WindowCapture.longestSide / 2)
        #expect(WindowCapture.fitted(width: 1200, height: 760) == (1200, 760))
        let big = try picture(width: 3000, height: 600)
        let scaled = WindowCapture.scaledToFit(big)
        #expect(scaled.width == 2400 && scaled.height == 480)
        let small = try picture(width: 300, height: 200)
        #expect(WindowCapture.scaledToFit(small) === small)
        let png = try #require(WindowCapture.png(small))
        #expect(png.starts(with: [0x89, 0x50, 0x4E, 0x47]))
    }

    /// White, with a line of black text in the middle.
    private func pictureOfText(_ text: String) throws -> CGImage {
        let width = 600, height = 200
        let context = try #require(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                             space: CGColorSpaceCreateDeviceRGB(),
                                             bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let graphics = NSGraphicsContext(cgContext: context, flipped: false)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = graphics
        (text as NSString).draw(at: CGPoint(x: 40, y: 80), withAttributes: [
            .font: NSFont.systemFont(ofSize: 32), .foregroundColor: NSColor.black,
        ])
        NSGraphicsContext.restoreGraphicsState()
        return try #require(context.makeImage())
    }

    /// The darkest red component anywhere in the picture.
    private func darkest(_ image: CGImage) throws -> UInt8 {
        let rep = NSBitmapImageRep(cgImage: image)
        var least: CGFloat = 1
        for y in stride(from: 0, to: rep.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: rep.pixelsWide, by: 2) {
                least = min(least, try #require(rep.colorAt(x: x, y: y)).redComponent)
            }
        }
        return UInt8((least * 255).rounded())
    }

    @Test func textInThePictureIsFoundAndPaintedOver() async throws {
        let shot = try pictureOfText("alex.rivera@example.org")
        #expect(try darkest(shot) < 40, "the text is there")
        let areas = await TextHider.textAreas(in: shot)
        #expect(!areas.isEmpty)
        let hidden = try #require(FeedbackReport.covered(shot, by: [], hiding: areas))
        let grey = Int((FeedbackReport.hiddenTextGray * 255).rounded())
        #expect(abs(Int(try darkest(hidden)) - grey) <= 3, "nothing darker than the paint is left: no letter shows")
        let blank = try picture(width: 200, height: 100)
        #expect(await TextHider.textAreas(in: blank).isEmpty)
    }

    @Test func aTextAreaIsMeasuredFromTheTopAndReachesPastTheText() {
        // Vision: 10% up from the bottom, 20% tall.
        let area = TextHider.area(for: CGRect(x: 0.2, y: 0.1, width: 0.4, height: 0.2))
        #expect(abs(area.minY - 0.65) < 0.0001 && abs(area.maxY - 0.95) < 0.0001)
        #expect(abs(area.minX - 0.15) < 0.0001 && abs(area.width - 0.5) < 0.0001)
        let edge = TextHider.area(for: CGRect(x: 0, y: 0.9, width: 1, height: 0.1))
        #expect(edge.minX == 0 && edge.minY == 0 && edge.maxX == 1, "kept inside the picture")
    }

    @Test func hiddenTextIsPaintedGreyUnderTheBlackCovers() throws {
        let white = try picture(width: 100, height: 100)
        let out = try #require(FeedbackReport.covered(white, by: [CGRect(x: 0, y: 0, width: 0.5, height: 0.5)],
                                                      hiding: [CGRect(x: 0, y: 0, width: 1, height: 0.2)]))
        #expect(try red(out, x: 10, y: 10) == 0, "a cover wins")
        #expect(abs(Int(try red(out, x: 80, y: 10)) - Int((FeedbackReport.hiddenTextGray * 255).rounded())) <= 3)
        #expect(try red(out, x: 80, y: 60) == 255)
    }

    @Test func aDroppedPictureIsReadForTextAgain() async throws {
        let model = try await demo()
        let draft = model.makeFeedbackDraft(.bug, picture: try picture(width: 60, height: 40))
        await draft.findText()
        #expect(draft.textAreas == [])
        let file = CoreClient.testScratch().appending(path: "drop.png")
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try #require(WindowCapture.png(try pictureOfText("Onboarding plan"))).write(to: file)
        #expect(draft.usePicture(at: file))
        #expect(draft.textAreas == nil && draft.waitingForText)
        await draft.findText()
        #expect(draft.textAreas?.isEmpty == false)
    }

    // MARK: The sheet's draft

    @Test func aBugOffersThePictureAndAnIdeaDoesNot() async throws {
        let model = try await demo()
        let shot = try picture(width: 60, height: 40)
        let bug = model.makeFeedbackDraft(.bug, picture: shot)
        #expect(bug.includePicture && !bug.includeLog)
        #expect(bug.cannotSend == nil && bug.accountID == AppModel.demoAccountID)
        #expect(bug.context.contains("Account: Demo"))
        #expect(!bug.canSend, "no title yet")
        bug.title = "  Archive jumps "
        #expect(bug.hideText && bug.waitingForText && !bug.canSend, "not before its text is hidden")
        #expect(bug.pictureToSend == nil, "never sent unhidden in the meantime")
        await bug.findText()
        #expect(bug.textAreas != nil && bug.canSend)
        bug.hideText = false
        #expect(bug.canSend && bug.pictureToSend === shot, "as it is, once the user says so")
        let idea = model.makeFeedbackDraft(.idea, picture: shot)
        #expect(!idea.includePicture && idea.pictureToSend == nil && idea.log.isEmpty)
        #expect(model.makeFeedbackDraft(.bug, picture: nil).includePicture == false)
    }

    @Test func theLogIsSentOnlyWhenAskedFor() async throws {
        let model = try await demo()
        let draft = FeedbackDraft(kind: .bug, accountID: "demo", sender: "me@example.com", cannotSend: nil,
                                  context: ["App: 0.1.0 (1)"], log: "WARN x", picture: nil)
        draft.details = "It broke."
        #expect(!draft.body.contains("WARN x"))
        draft.includeLog = true
        #expect(draft.body.contains("--- Kaluta log ---\nWARN x\n--- end ---"))
        _ = model
    }

    @Test func aReportIsSentAsMailWithItsPictureAndLandsInSent() async throws {
        let model = try await demo()
        let draft = model.makeFeedbackDraft(.bug, picture: try picture(width: 60, height: 40))
        draft.title = "Feedback test"
        draft.details = "The list jumps when I archive."
        draft.covers = [CGRect(x: 0, y: 0, width: 0.5, height: 0.5)]
        await draft.findText()
        model.feedbackDraft = draft
        await model.sendFeedback(draft)
        #expect(draft.error == nil)
        #expect(model.feedbackDraft == nil, "the sheet closes")
        #expect(try await model.core!.drafts().isEmpty)
        let sent = try await model.core!.threads(in: "SENT", limit: 50).rows
        // Sent's first copy lists no attachments; that a draft's files and
        // a report's lines reach the mail is checked where it is built
        // (mail-mime's build tests).
        #expect(sent.contains { $0.subject == "Feedback test" })
    }

    @Test func anImportedMailboxCannotSendAndSaysWhereToWrite() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        let draft = model.makeFeedbackDraft(.idea, picture: nil)
        #expect(draft.cannotSend?.contains("idea@support.kaluta.org") == true)
        draft.title = "Snooze"
        #expect(!draft.canSend)
        await model.sendFeedback(draft)
        #expect(draft.error == nil && !draft.sending)
    }
}
