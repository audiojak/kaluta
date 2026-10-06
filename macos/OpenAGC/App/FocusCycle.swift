import AppKit
import SwiftUI

/// The main window's keyboard loop (spec §14.3): Tab moves the keyboard
/// sidebar → list → reader (when a message is shown) → search → sidebar;
/// ⌥Tab, and ⇧Tab as on any Mac, go the other way. Each column registers
/// the AppKit view its keys land in, so the loop can tell where the
/// keyboard is; a column takes the keyboard only when the loop asks.
@MainActor
@Observable
final class FocusCycle {
    enum Region: CaseIterable, Equatable {
        case sidebar, list, reader, search
    }

    @ObservationIgnored weak var sidebarView: NSView?
    @ObservationIgnored weak var listView: NSView?
    /// The reader's web view: it scrolls itself once it has the keyboard.
    @ObservationIgnored weak var readerView: NSView?
    @ObservationIgnored private var monitor: Any?

    func register(_ view: NSView, as region: Region) {
        switch region {
        case .sidebar: sidebarView = view
        case .list: listView = view
        case .reader: readerView = view
        case .search: break
        }
    }

    /// The next stop, or the one before. The reader is skipped when no
    /// message is shown.
    static func next(from region: Region, backwards: Bool, readerShown: Bool) -> Region {
        let stops: [Region] = readerShown ? [.sidebar, .list, .reader, .search] : [.sidebar, .list, .search]
        let at = stops.firstIndex(of: region) ?? (backwards ? 0 : stops.count - 1)
        let count = stops.count
        return stops[((at + (backwards ? -1 : 1)) % count + count) % count]
    }

    /// Where the keyboard is, if in one of the loop's stops.
    func region(of responder: NSResponder?) -> Region? {
        guard let responder else { return nil }
        if let field = responder as? NSTextView, field.delegate is NSSearchField { return .search }
        if responder is NSSearchField { return .search }
        guard let view = responder as? NSView else { return nil }
        if let sidebarView, view === sidebarView || view.isDescendant(of: sidebarView) { return .sidebar }
        if let listView, view === listView || view.isDescendant(of: listView) { return .list }
        if let readerView, view === readerView || view.isDescendant(of: readerView) { return .reader }
        return nil
    }

    /// Whether `event` is the loop's Tab: Tab with no ⌘ or ⌃, in a window
    /// of ours, with the keyboard in one of the stops and no sheet up.
    func takes(_ event: NSEvent) -> Region? {
        guard event.keyCode == 48, event.modifierFlags.intersection([.command, .control]).isEmpty,
              let window = event.window, window.attachedSheet == nil,
              window === (listView?.window ?? sidebarView?.window) else { return nil }
        return region(of: window.firstResponder)
    }

    func start(model: AppModel) {
        stop()
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self, weak model] event in
            nonisolated(unsafe) let pressed = event
            let taken = MainActor.assumeIsolated {
                guard let self, let model, let from = self.takes(pressed) else { return false }
                let backwards = !pressed.modifierFlags.intersection([.option, .shift]).isEmpty
                self.move(to: Self.next(from: from, backwards: backwards, readerShown: model.readerShown), model: model)
                return true
            }
            return taken ? nil : event
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    func move(to region: Region, model: AppModel) {
        switch region {
        case .sidebar:
            if let sidebarView { sidebarView.window?.makeFirstResponder(sidebarView) }
        case .list: model.focusThreadList()
        case .reader:
            if let readerView { readerView.window?.makeFirstResponder(readerView) }
        case .search: model.focusSearch()
        }
    }
}

/// Registers the nearest table or scroll view a SwiftUI view is drawn in
/// as one of the loop's stops.
struct FocusRegionProbe: NSViewRepresentable {
    let cycle: FocusCycle
    let region: FocusCycle.Region

    func makeNSView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.cycle = cycle
        view.region = region
        return view
    }

    func updateNSView(_ view: ProbeView, context: Context) {
        view.cycle = cycle
        view.region = region
        view.find()
    }

    final class ProbeView: NSView {
        weak var cycle: FocusCycle?
        var region: FocusCycle.Region = .list

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            find()
        }

        func find() {
            var view = superview
            while let current = view, !(current is NSTableView || current is NSScrollView) { view = current.superview }
            if let found = view { MainActor.assumeIsolated { cycle?.register(found, as: region) } }
        }
    }
}
