import AppKit
import ScreenCaptureKit

/// A picture of one of Kaluta's own windows, as the user sees it, for a
/// bug report (spec §17.1). ScreenCaptureKit hands a process its own
/// windows without Screen Recording permission, so nothing is asked and
/// nothing outside Kaluta can be in the picture. If that fails, the window
/// draws itself (which leaves some SwiftUI surfaces blank on macOS 26).
@MainActor
enum WindowCapture {
    enum Failure: Error {
        case noWindow
    }

    /// The longest side a report's picture is sent at, in pixels: readable,
    /// and a PNG of a mail window stays well under the mail's size limits.
    static let longestSide = 2400

    static func image(of window: NSWindow) async -> CGImage? {
        if let image = try? await composited(window) { return image }
        return drawn(window)
    }

    /// The window as the window server composites it: glass, SwiftUI and
    /// web content included.
    static func composited(_ window: NSWindow) async throws -> CGImage {
        let content = try await SCShareableContent.currentProcess
        let id = CGWindowID(window.windowNumber)
        guard let target = content.windows.first(where: { $0.windowID == id }) else { throw Failure.noWindow }
        let filter = SCContentFilter(desktopIndependentWindow: target)
        let scale = CGFloat(filter.pointPixelScale)
        let size = fitted(width: filter.contentRect.width * scale, height: filter.contentRect.height * scale)
        let configuration = SCStreamConfiguration()
        configuration.width = size.width
        configuration.height = size.height
        configuration.showsCursor = false
        configuration.ignoreShadowsSingleWindow = true
        return try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
    }

    /// The window's views drawn into a bitmap by AppKit.
    static func drawn(_ window: NSWindow) -> CGImage? {
        guard let view = window.contentView?.superview ?? window.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return nil }
        view.cacheDisplay(in: view.bounds, to: rep)
        return rep.cgImage
    }

    /// A size no longer than `longestSide` on either side, same shape.
    static func fitted(width: CGFloat, height: CGFloat) -> (width: Int, height: Int) {
        let longest = max(width, height)
        let factor = longest > CGFloat(longestSide) ? CGFloat(longestSide) / longest : 1
        return (max(1, Int((width * factor).rounded())), max(1, Int((height * factor).rounded())))
    }

    /// The picture no larger than a report sends (a dropped file can be
    /// any size).
    static func scaledToFit(_ image: CGImage) -> CGImage {
        let size = fitted(width: CGFloat(image.width), height: CGFloat(image.height))
        guard size.width < image.width,
              let context = CGContext(data: nil, width: size.width, height: size.height, bitsPerComponent: 8,
                                      bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return image }
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: size.width, height: size.height))
        return context.makeImage() ?? image
    }

    static func png(_ image: CGImage) -> Data? {
        NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
    }
}
