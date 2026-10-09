import Foundation

/// Reads of a store's state, one at a time. A read asked for while one is
/// running makes it read once more, and returns only once that read is
/// applied, so code after `await load()` always sees fresh state (two
/// overlapping reads used to drop the newer one, or return before it).
@MainActor
final class SerialReads {
    private var reading: Task<Void, Never>?
    private var again = false

    func run(_ read: @escaping @MainActor () async -> Void) async {
        if let reading {
            again = true
            await reading.value
            return
        }
        let task = Task { @MainActor in
            repeat {
                again = false
                await read()
            } while again
            // Cleared here: a read asked for after the last one starts anew.
            reading = nil
        }
        reading = task
        await task.value
    }
}
