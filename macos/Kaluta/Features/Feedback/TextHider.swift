import CoreGraphics
import Vision

/// Finds the text in a report's picture so it can be painted over (spec
/// §17.1): names, addresses, subjects and mail are all text. The reading
/// is done on the Mac by the system's text recognition; nothing leaves it.
enum TextHider {
    /// Text this small is still found: a caption in a window pictured at
    /// one pixel per point is about 1% of its height.
    static let minimumTextHeight: Float = 0.006
    /// How far a painted area reaches past the text it covers, as a
    /// fraction of the text's height: ascenders, descenders and soft edges.
    static let margin: CGFloat = 0.25

    /// Where text is, as fractions of the picture measured from its top
    /// left, each a little larger than the text.
    static func textAreas(in image: CGImage) async -> [CGRect] {
        await Task.detached(priority: .userInitiated) {
            // Two readers: the recogniser finds lines of words; the
            // rectangle detector also finds what is not a word (a lone
            // digit, a count, initials).
            let words = VNRecognizeTextRequest()
            words.recognitionLevel = .accurate
            words.usesLanguageCorrection = false
            words.minimumTextHeight = minimumTextHeight
            let shapes = VNDetectTextRectanglesRequest()
            do {
                try VNImageRequestHandler(cgImage: image).perform([words, shapes])
            } catch {
                return []
            }
            return (words.results ?? []).map { area(for: $0.boundingBox) }
                + (shapes.results ?? []).map { area(for: $0.boundingBox) }
        }.value
    }

    /// Vision measures from the bottom left; the sheet and the painter
    /// measure from the top left.
    static func area(for box: CGRect) -> CGRect {
        let grow = box.height * margin
        return CGRect(x: box.minX - grow, y: 1 - box.maxY - grow, width: box.width + grow * 2,
                      height: box.height + grow * 2)
            .intersection(CGRect(x: 0, y: 0, width: 1, height: 1))
    }
}
