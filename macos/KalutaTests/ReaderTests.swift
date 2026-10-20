import Foundation
import Testing
@testable import Kaluta

struct EmailDocumentTests {
    private func message(_ id: String, read: Bool = true, html: String? = "<p>x</p>", from: String = "Ann") -> EmailDocument.Message {
        EmailDocument.Message(id: id, fromName: from, fromEmail: "ann@example.com", recipients: ["Me"],
                              date: Date(timeIntervalSince1970: 0), snippet: "snip", isRead: read, html: html)
    }

    @Test func documentCarriesTheCSPAndEscapesHeaderText() {
        let html = EmailDocument.thread([message("1", from: "<script>alert(1)</script>")], isDark: false)
        #expect(html.contains(#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'"#))
        #expect(!html.contains("<script>alert(1)</script>"))
        #expect(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"))
    }

    @Test func latestAndUnreadMessagesStartExpanded() {
        let html = EmailDocument.thread([message("a"), message("b", read: false), message("c"), message("d")], isDark: false)
        #expect(html.contains(#"<details class="msg" id="m-a">"#))
        #expect(html.contains(#"<details class="msg" open id="m-b""#))
        #expect(html.contains(#"<details class="msg" id="m-c""#))
        #expect(html.contains(#"<details class="msg" open id="m-d""#))
    }

    @Test func theComposersViewOpensOnlyTheLatestAndLeavesOutDrafts() {
        var draft = message("e")
        draft.isDraft = true
        let shown = ComposerThreadPane.shown([message("a"), message("b", read: false), message("c"), draft])
        #expect(shown.map(\.id) == ["a", "b", "c"], "this reply's own draft is not part of the conversation")
        let html = EmailDocument.thread(shown, isDark: false, onlyLatestOpen: true)
        #expect(html.contains(#"<details class="msg" id="m-a">"#))
        #expect(html.contains(#"<details class="msg" id="m-b">"#), "even unread, an earlier message is a row")
        #expect(html.contains(#"<details class="msg" open id="m-c""#))
    }

    @Test func styledMailGetsPaperInDarkModeOnly() {
        let styled = message("s", html: ##"<table bgcolor="#fff"><tr><td>x</td></tr></table>"##)
        #expect(EmailDocument.thread([styled], isDark: true).contains(#"class="body paper""#))
        #expect(!EmailDocument.thread([styled], isDark: false).contains("paper\""))
        #expect(!EmailDocument.thread([message("u")], isDark: true).contains(#"class="body paper""#))
    }

    @Test func cardsCarryInitialsAColourPerSenderAndAShortToLine() {
        #expect(EmailDocument.initials(name: "Darshan Patel", email: "d@x") == "DP")
        #expect(EmailDocument.initials(name: "Le, Minh", email: "m@x") == "ML")
        #expect(EmailDocument.initials(name: "Cher", email: "c@x") == "C")
        #expect(EmailDocument.initials(name: "billing@zoom.us", email: "billing@zoom.us") == "B")
        #expect(EmailDocument.avatarColor("A@x.com") == EmailDocument.avatarColor("a@x.com"), "stable per sender")
        #expect(EmailDocument.shortRecipients(["Me"]) == "Me")
        #expect(EmailDocument.shortRecipients(["Me", "Sam"]) == "Me & Sam")
        #expect(EmailDocument.shortRecipients(["Me", "Sam", "Ann", "Bo"]) == "Me, Sam + 2")
        let many = EmailDocument.Message(id: "1", fromName: "Ann", fromEmail: "ann@x.com",
                                         recipients: ["Me", "Sam", "Ann", "Bo"], date: Date(timeIntervalSince1970: 0),
                                         snippet: "", isRead: true, html: "<p>x</p>")
        let html = EmailDocument.thread([many], isDark: false)
        #expect(html.contains(#"title="Me, Sam, Ann, Bo">To: Me, Sam + 2</div>"#), "the full list on hover")
        #expect(html.contains(#"class="avatar""#))
    }

    /// Each open message answers itself: Reply, Reply all when someone
    /// else is on it, Forward; drafts and the composer's view have none.
    @Test func eachMessageHasItsOwnReplyAndForward() {
        var everyone = message("a/1")
        everyone.offersReplyAll = true
        var draft = message("d")
        draft.isDraft = true
        let html = EmailDocument.thread([everyone, message("b"), draft], isDark: false, actions: true)
        #expect(html.contains(#"href="kaluta-action:reply/a%2F1">Reply<kbd>⌘R</kbd></a>"#))
        #expect(html.contains(#"href="kaluta-action:reply-all/a%2F1">Reply all<kbd>⇧⌘R</kbd></a>"#))
        #expect(html.contains(#"href="kaluta-action:forward/b">Forward<kbd>⇧⌘F</kbd></a>"#))
        #expect(!html.contains("kaluta-action:reply-all/b"), "no one else on it")
        #expect(!html.contains("kaluta-action:reply/d"), "a draft is edited, not answered")
        #expect(!EmailDocument.thread([message("b")], isDark: false).contains("kaluta-action"))
        // The keys answer the latest until another is clicked: its links say so.
        #expect(html.contains(#"<details class="msg current" id="m-b""#))
        #expect(html.contains("Reply<kbd>⌘R</kbd>") && html.contains("Forward<kbd>⇧⌘F</kbd>"))
        let parsed = EmailDocument.MessageAction.parse(URL(string: "kaluta-action:reply-all/a%2F1")!)
        #expect(parsed?.0 == .replyAll && parsed?.1 == "a/1")
        #expect(EmailDocument.MessageAction.parse(URL(string: "kaluta-action:delete/x")!) == nil)
        #expect(EmailDocument.MessageAction.parse(URL(string: "https://example.com/reply/x")!) == nil)
    }

    @MainActor @Test func replyAllIsOfferedWhenItReachesMoreThanReply() {
        func address(_ email: String) -> AddressInfo { AddressInfo(name: nil, email: email) }
        let own: Set<String> = ["john@actual.ai"]
        let toMe = MessageInfo.sample(from: address("harish@example.com"), to: [address("john@actual.ai")], cc: [])
        #expect(!ReaderStore.offersReplyAll(toMe, own: own))
        let copied = MessageInfo.sample(from: address("harish@example.com"), to: [address("john@actual.ai")],
                                        cc: [address("simon@example.com")])
        #expect(ReaderStore.offersReplyAll(copied, own: own))
        var mine = MessageInfo.sample(from: address("john@actual.ai"), to: [address("brian@example.com")], cc: [])
        mine.isSentByMe = true
        #expect(!ReaderStore.offersReplyAll(mine, own: own), "Reply already reaches Brian")
        mine.cc = [address("austin@example.com")]
        #expect(ReaderStore.offersReplyAll(mine, own: own))
    }

    @Test func missingBodyShowsAPlaceholder() {
        #expect(EmailDocument.thread([message("p", html: nil)], isDark: false).contains("Downloading message"))
    }
}

struct LinkSafetyTests {
    @Test func flagsLinksWhoseTextNamesAnotherHost() {
        let html = #"<a href="https://evil.example.net/login" target="_blank">https://bank.example.com/login</a>"#
        #expect(LinkSafety.mismatchedLinks(in: html) == ["https://evil.example.net/login": "https://bank.example.com/login"])
    }

    @Test func allowsMatchingHostsSubdomainsAndOrdinaryText() {
        let html = """
        <a href="https://www.example.com/a">example.com</a>
        <a href="https://mail.example.com/x">example.com</a>
        <a href="https://example.org/y">Read more</a>
        <a href="https://example.org/z"><b>example.org</b></a>
        """
        #expect(LinkSafety.mismatchedLinks(in: html).isEmpty)
    }

    @Test func decodesEntitiesInHrefs() {
        let html = #"<a href="https://evil.example.net/?a=1&amp;b=2">paypal.example.com</a>"#
        #expect(LinkSafety.mismatchedLinks(in: html).keys.first == "https://evil.example.net/?a=1&b=2")
    }
}

@MainActor
struct MessageAttachmentTests {
    /// Each email's files are on its own card: listed when collapsed, as
    /// links the app opens (Quick Look) when open; the link's scheme is the
    /// app's, which mail cannot make.
    @Test func eachEmailCarriesItsOwnFiles() {
        var withFiles = EmailDocument.Message(id: "m2", fromName: "Jenn", fromEmail: "jenn@example.com", recipients: [],
                                              date: .now, snippet: "attached", isRead: true, html: "<p>see attached</p>")
        withFiles.attachments = [EmailDocument.Attachment(id: "a1", filename: "Q3 P&L.pdf", size: 13_000)]
        let plain = EmailDocument.Message(id: "m1", fromName: "Ann", fromEmail: "ann@example.com", recipients: [],
                                          date: .now, snippet: "hi", isRead: true, html: "<p>hi</p>")
        let page = EmailDocument.thread([plain, withFiles], isDark: false)
        #expect(page.contains(#"href="kaluta-attachment:a1""#))
        #expect(page.contains("Q3 P&amp;L.pdf"), "escaped")
        #expect(page.components(separatedBy: "kaluta-attachment:").count == 2, "only on the email that has it")
        #expect(page.contains(#"<div class="clip""#))
    }
}

@MainActor
struct RemoteImagePolicyTests {
    /// Images load by themselves from senders always allowed and, unless
    /// turned off, from people the user has written to.
    @Test func imagesLoadFromPeopleTheUserHasWrittenTo() {
        let defaults = UserDefaults(suiteName: "kaluta-images-\(UUID().uuidString)")!
        let known: Set<String> = ["ann@example.com"]
        #expect(ReaderStore.imagesAllowed(for: ["ann@example.com"], known: known, defaults: defaults), "on by default")
        #expect(!ReaderStore.imagesAllowed(for: ["news@example.com"], known: known, defaults: defaults))
        #expect(!ReaderStore.imagesAllowed(for: ["ann@example.com", "news@example.com"], known: known, defaults: defaults),
                "every sender in it")
        defaults.set(["news@example.com"], forKey: ReaderStore.allowedSendersKey)
        #expect(ReaderStore.imagesAllowed(for: ["ann@example.com", "news@example.com"], known: known, defaults: defaults))
        defaults.set(false, forKey: ReaderStore.knownSendersKey)
        #expect(!ReaderStore.imagesAllowed(for: ["ann@example.com"], known: known, defaults: defaults), "turned off")
        #expect(!ReaderStore.imagesAllowed(for: [], known: known, defaults: defaults))
    }
}

@MainActor
struct SchemeHandlerTests {
    @Test func remoteURLsAreUnwrappedOnlyForHTTP() {
        #expect(RemoteImageSchemeHandler.remoteURL(from: URL(string: "kaluta-remote:https://img.example.com/a.png?x=1"))
            == URL(string: "https://img.example.com/a.png?x=1"))
        #expect(RemoteImageSchemeHandler.remoteURL(from: URL(string: "kaluta-remote:file:///etc/passwd")) == nil)
        #expect(RemoteImageSchemeHandler.remoteURL(from: URL(string: "kaluta-remote:javascript:alert(1)")) == nil)
        // Load Images asks again under other addresses: WebKit kept the blank
        // pixels the first load got and answered with them.
        #expect(RemoteImageSchemeHandler.loading(#"<img src="kaluta-remote:https://a.example/x.png">"#)
            == #"<img src="kaluta-remote-loaded:https://a.example/x.png">"#)
        #expect(RemoteImageSchemeHandler.remoteURL(from: URL(string: "kaluta-remote-loaded:https://a.example/x.png"))
            == URL(string: "https://a.example/x.png"))
        #expect(EmailDocument.contentSecurityPolicy.contains("kaluta-remote-loaded:"))
        #expect(RemoteImageSchemeHandler.remoteURL(from: URL(string: "https://img.example.com/a.png")) == nil)
    }
}

@MainActor
struct ReaderStoreTests {
    @Test func loadsAThreadWithBodiesAndDecidesRemoteImages() async throws {
        let dir = CoreClient.testScratch()
        let model = AppModel(core: try CoreClient(dataDirectory: dir))
        await model.start(openDemo: true)
        let first = try #require(model.threads.rows.first)
        await model.reader.show(threadID: first.id)
        let detail = try #require(model.reader.detail)
        #expect(detail.thread.id == first.id)
        #expect(model.reader.bodies.count == detail.messages.count)
        #expect(model.reader.documentMessages.allSatisfy { $0.html != nil })
        #expect(!model.reader.hasRemoteImages, "demo mail has no remote images")
        #expect(!model.reader.remoteImagesAllowedForThread)
        model.reader.loadRemoteImagesForThread()
        #expect(model.reader.allowsRemoteImages)
        await model.reader.show(threadID: nil)
        #expect(model.reader.detail == nil)
    }
}

struct DraftCardTests {
    @Test func aDraftInTheThreadIsMarkedAsOne() {
        let sent = EmailDocument.Message(id: "1", fromName: "Ann", fromEmail: "ann@x.com", recipients: ["Me"],
                                         date: .now, snippet: "hi", isRead: true, html: "<p>hi</p>")
        let draft = EmailDocument.Message(id: "2", fromName: "Me", fromEmail: "me@x.com", recipients: ["Ann"],
                                          date: .now, snippet: "reply", isRead: true, html: "<p>reply</p>",
                                          isDraft: true)
        let html = EmailDocument.thread([sent, draft], isDark: false)
        #expect(html.contains("<details class=\"msg draft\" open id=\"m-2\">"))
        #expect(html.contains("<span class=\"badge\">Draft</span>"))
        #expect(html.contains("Saved "))
        #expect(html.contains("<details class=\"msg\" open id=\"m-1\">") == false, "the sent one is collapsed, read")
        #expect(html.components(separatedBy: "class=\"badge\"").count == 2, "only the draft is marked")
    }
}
