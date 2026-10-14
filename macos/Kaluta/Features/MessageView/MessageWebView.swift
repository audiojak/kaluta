import AppKit
import SwiftUI
import WebKit

/// The locked-down web view that renders a thread (spec §14.4): no
/// JavaScript, no persistent storage, no navigation, links opened in the
/// user's browser after a phishing check, images only through our schemes.
struct MessageWebView: NSViewRepresentable {
    let html: String
    let allowRemoteImages: Bool
    /// The web view once made; the main window's Tab loop registers it.
    var onCreated: ((WKWebView) -> Void)?
    /// Inline images by content id; the page reloads when more arrive.
    var inlineImages: [String: InlineImage] = [:]
    /// Open on the last message (the composer's view of a thread) rather
    /// than the top. The page itself still runs no script: this is the
    /// app's own call, after each load.
    var scrollToLatest = false
    /// The page's height once laid out (and again as images arrive), for a
    /// view that grows to fit the email instead of scrolling it: a
    /// workflow's item, where the workflow keeps the one scroll.
    var onContentHeight: ((CGFloat) -> Void)?
    /// An attachment's link on its message's card was clicked: its id.
    var onAttachment: ((String) -> Void)?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.preferences.isElementFullscreenEnabled = false
        config.setURLSchemeHandler(context.coordinator.remoteImages, forURLScheme: RemoteImageSchemeHandler.scheme)
        config.setURLSchemeHandler(context.coordinator.remoteImages, forURLScheme: RemoteImageSchemeHandler.loadedScheme)
        config.setURLSchemeHandler(context.coordinator.inlineImages, forURLScheme: CidSchemeHandler.scheme)
        // Sized to its content, it hands the scroll wheel to the view around it.
        let webView: WKWebView
        if onContentHeight == nil {
            webView = WKWebView(frame: .zero, configuration: config)
        } else {
            let fitted = FittedWebView(frame: .zero, configuration: config)
            // A new width reflows the email: measured again.
            fitted.widthChanged = { [weak coordinator = context.coordinator, weak fitted] in
                if let fitted { coordinator?.measure(fitted, after: [0.05]) }
            }
            webView = fitted
        }
        onCreated?(webView)
        webView.navigationDelegate = context.coordinator
        webView.uiDelegate = context.coordinator
        webView.allowsBackForwardNavigationGestures = false
        webView.allowsMagnification = true
        webView.underPageBackgroundColor = .textBackgroundColor
        #if DEBUG
        webView.isInspectable = true
        #else
        webView.isInspectable = false
        #endif
        webView.setAccessibilityLabel("Message")
        return webView
    }

    func updateNSView(_ webView: WKWebView, context: Context) {
        let coordinator = context.coordinator
        coordinator.onContentHeight = onContentHeight
        coordinator.onAttachment = onAttachment
        let remoteChanged = coordinator.remoteImages.allowRemote != allowRemoteImages
        coordinator.remoteImages.allowRemote = allowRemoteImages
        let images = inlineImages
        coordinator.inlineImages.provider = { cid in images[cid].map { ($0.data, $0.mimeType) } }
        let imagesChanged = coordinator.loadedInlineImages != Set(images.keys)
        coordinator.loadedInlineImages = Set(images.keys)
        guard html != coordinator.loadedHTML || remoteChanged || imagesChanged else { return }
        coordinator.scrollToLatest = scrollToLatest
        coordinator.loadedHTML = html
        coordinator.suspiciousLinks = LinkSafety.mismatchedLinks(in: html)
        // A fixed, opaque base URL: nothing in the document can resolve
        // relative to a real origin.
        let shown = allowRemoteImages ? RemoteImageSchemeHandler.loading(html) : html
        webView.loadHTMLString(shown, baseURL: URL(string: "about:blank"))
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        let remoteImages = RemoteImageSchemeHandler()
        let inlineImages = CidSchemeHandler()
        var loadedHTML = ""
        var loadedInlineImages: Set<String> = []
        var suspiciousLinks: [String: String] = [:]
        var scrollToLatest = false
        var onContentHeight: ((CGFloat) -> Void)?
        var onAttachment: ((String) -> Void)?

        func measure(_ webView: WKWebView, after delays: [Double]) {
            guard onContentHeight != nil else { return }
            for delay in delays {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self, weak webView] in
                    webView?.evaluateJavaScript("document.documentElement.scrollHeight") { value, _ in
                        if let height = value as? Double { self?.onContentHeight?(CGFloat(height)) }
                    }
                }
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            // Measured by the app (the page runs no script), now and as its
            // images arrive.
            measure(webView, after: [0.0, 0.3, 1.0, 2.5, 5.0])
            // Each load (bodies and images arriving reload the page) starts
            // at the top again.
            guard scrollToLatest else { return }
            // A short message sits at the bottom with the earlier rows above it;
            // a long one starts at its top.
            webView.evaluateJavaScript("""
                var m = document.querySelectorAll('details.msg');
                if (m.length) { var e = m[m.length - 1]; e.scrollIntoView(e.offsetHeight + 24 > window.innerHeight); }
                """)
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            // Our own loadHTMLString is the only navigation allowed.
            if action.navigationType == .other, action.request.url?.absoluteString == "about:blank" {
                return .allow
            }
            if action.navigationType == .linkActivated, let url = action.request.url {
                if url.scheme == EmailDocument.attachmentScheme {
                    onAttachment?(String(url.absoluteString.dropFirst(EmailDocument.attachmentScheme.count + 1)))
                } else {
                    open(url)
                }
            }
            return .cancel
        }

        /// `target=_blank` links arrive here; open them externally too.
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                     for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = action.request.url { open(url) }
            return nil
        }

        private func open(_ url: URL) {
            guard let scheme = url.scheme?.lowercased(), ["http", "https", "mailto"].contains(scheme) else { return }
            if let shown = suspiciousLinks[url.absoluteString] {
                let alert = NSAlert()
                alert.messageText = "This link may not go where it says"
                alert.informativeText = "The link text shows “\(shown)”, but it opens \(url.host() ?? url.absoluteString)."
                alert.alertStyle = .warning
                alert.addButton(withTitle: "Don’t Open")
                alert.addButton(withTitle: "Open Anyway")
                guard alert.runModal() == .alertSecondButtonReturn else { return }
            }
            NSWorkspace.shared.open(url)
        }
    }
}

/// A web view that is as tall as its page: the scroll wheel goes to the
/// scroll view it sits in.
final class FittedWebView: WKWebView {
    var widthChanged: (() -> Void)?
    private var lastWidth: CGFloat = 0

    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }

    /// ↑ ↓, Space, Page Up and Down, Home and End scroll what it sits in;
    /// with nothing of its own to scroll, it would swallow them.
    override func keyDown(with event: NSEvent) {
        let reading: Set<UInt16> = [123, 124, 125, 126, 49, 116, 121, 115, 119]
        if reading.contains(event.keyCode), event.modifierFlags.intersection([.command, .control, .option]).isEmpty {
            nextResponder?.keyDown(with: event)
        } else {
            super.keyDown(with: event)
        }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        if abs(newSize.width - lastWidth) > 1 {
            lastWidth = newSize.width
            widthChanged?()
        }
    }
}
