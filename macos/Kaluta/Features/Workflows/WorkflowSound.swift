import AudioToolbox
import AVFoundation
import os

/// The end of a set's reward (spec §14.13): a G major chord, G3 B3 D4 G4,
/// strummed over about 0.1 s and fading over about a second. It plays as
/// a system sound, so macOS keeps it silent when *Play user interface
/// sound effects* is off, at the alert volume; Kaluta's *Play sounds*
/// setting silences it too.
@MainActor
enum WorkflowSound {
    /// Kaluta's *Play sounds* (Settings › General › Workflows), on unless
    /// turned off.
    static let playSoundsKey = "playSounds"

    static let frequencies: [Double] = [196.00, 246.94, 293.66, 392.00]
    static let strum = 0.1
    static let seconds = 1.3
    static let sampleRate = 44_100.0
    static let peak: Float = 0.5

    private static var soundID: SystemSoundID?
    private static let log = Logger(subsystem: "org.kaluta.Kaluta", category: "workflows")

    static func playChord(defaults: UserDefaults) {
        guard defaults.object(forKey: playSoundsKey) as? Bool ?? true else { return }
        guard let id = soundID ?? register() else { return }
        AudioServicesPlaySystemSound(id)
    }

    /// The chord rendered once into a temporary file, registered as a
    /// system sound.
    private static func register() -> SystemSoundID? {
        let url = FileManager.default.temporaryDirectory.appending(path: "kaluta-workflow-chord.caf")
        do {
            let samples = chord()
            guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
                  let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)),
                  let channel = buffer.floatChannelData?[0] else { return nil }
            buffer.frameLength = AVAudioFrameCount(samples.count)
            samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
            try? FileManager.default.removeItem(at: url)
            let file = try AVAudioFile(forWriting: url, settings: format.settings)
            try file.write(from: buffer)
        } catch {
            log.error("chord: \(error.localizedDescription, privacy: .public)")
            return nil
        }
        var id: SystemSoundID = 0
        guard AudioServicesCreateSystemSoundID(url as CFURL, &id) == kAudioServicesNoError else { return nil }
        soundID = id
        return id
    }

    /// The chord's samples: each note plucked (a fast attack, then an
    /// exponential fade, its overtones fading faster), one after another
    /// across `strum`; scaled to `peak` and ending in silence.
    static func chord() -> [Float] {
        let count = Int(seconds * sampleRate)
        var out = [Double](repeating: 0, count: count)
        let step = strum / Double(frequencies.count - 1)
        for (i, f) in frequencies.enumerated() {
            let start = Int(Double(i) * step * sampleRate)
            for n in start..<count {
                let t = Double(n - start) / sampleRate
                let attack = min(1, t / 0.004)
                let w = 2 * Double.pi * f * t
                let tone = sin(w) + 0.3 * sin(2 * w) * exp(-t * 4) + 0.12 * sin(3 * w) * exp(-t * 7)
                out[n] += attack * exp(-t * 3.5) * tone
            }
        }
        let largest = out.map(abs).max() ?? 1
        let fade = Int(0.08 * sampleRate)
        return out.enumerated().map { n, s in
            let tail = min(1, Double(count - 1 - n) / Double(fade))
            return Float(s / largest * tail) * peak
        }
    }
}
