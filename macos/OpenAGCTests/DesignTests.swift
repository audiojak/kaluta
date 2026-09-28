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
