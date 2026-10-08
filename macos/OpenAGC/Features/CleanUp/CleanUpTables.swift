import AppKit
import SwiftUI

// Clean Up's two long lists (spec §14.12), backed by `NSTableView` like
// the thread list (spec §13 rule 2): a mailbox can hold thousands of
// groups and tens of thousands of messages in them.

/// The groups column: a checkbox, the title, the address and other names
/// on a second line, and the count. Highlighting works as in the mail
/// lists (click, arrows, ⇧ and ⌘, ⌘A); ticks are separate, made by the
/// checkbox or Space, and are what actions act on.
struct CleanUpGroupList: NSViewRepresentable {
    @Environment(AppModel.self) private var model

    func makeCoordinator() -> Coordinator { Coordinator(store: model.cleanUp) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = CleanUpGroupTable()
        table.store = model.cleanUp
        table.headerView = nil
        table.style = .inset
        table.usesAutomaticRowHeights = false
        table.intercellSpacing = .zero
        table.allowsMultipleSelection = true
        table.allowsEmptySelection = true
        table.backgroundColor = .clear
        let column = NSTableColumn(identifier: CleanUpGroupRowView.identifier)
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.setAccessibilityLabel("Groups")
        context.coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let store = model.cleanUp
        context.coordinator.update(groups: store.groups, view: store.view, ticked: store.ticked)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        private let store: CleanUpStore
        private var groups: [CleanupGroup] = []
        private var view: CleanUpViewKind?
        private var ticked: Set<String> = []
        weak var table: NSTableView?

        init(store: CleanUpStore) { self.store = store }

        func update(groups newGroups: [CleanupGroup], view newView: CleanUpViewKind, ticked newTicked: Set<String>) {
            guard let table else { return }
            if newGroups != groups || newView != view {
                // Keep the highlight on the same groups across a reload.
                let highlighted = Set(table.selectedRowIndexes.compactMap { groups.indices.contains($0) ? groups[$0].key : nil })
                let sameView = newView == view
                groups = newGroups
                view = newView
                ticked = newTicked
                table.rowHeight = CleanUpGroupRowView.height(detailLine: newView.hasDetailLine)
                table.reloadData()
                let rows = IndexSet(groups.indices.filter { sameView && highlighted.contains(groups[$0].key) })
                table.selectRowIndexes(rows, byExtendingSelection: false)
                if !sameView { table.scrollRowToVisible(0) }
            } else if newTicked != ticked {
                ticked = newTicked
                let visible = table.rows(in: table.visibleRect)
                for row in visible.location..<(visible.location + visible.length) {
                    (table.view(atColumn: 0, row: row, makeIfNecessary: false) as? CleanUpGroupRowView)?
                        .setTicked(ticked.contains(groups[row].key))
                }
            }
        }

        /// The highlighted groups' keys, for Space.
        var highlightedKeys: [String] {
            table?.selectedRowIndexes.compactMap { groups.indices.contains($0) ? groups[$0].key : nil } ?? []
        }

        func numberOfRows(in tableView: NSTableView) -> Int { groups.count }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            let cell = tableView.makeView(withIdentifier: CleanUpGroupRowView.identifier, owner: nil) as? CleanUpGroupRowView
                ?? CleanUpGroupRowView()
            let group = groups[row]
            let store = store
            cell.configure(group, ticked: ticked.contains(group.key)) { store.toggle(group.key) }
            return cell
        }
    }
}

/// The groups table's keys (docs/keyboard.md, Clean Up): Space ticks the
/// highlighted groups; `e`, `⌫` (or `#`) and `!` act on the ticked ones;
/// `j` and `k` move like the arrows.
final class CleanUpGroupTable: NSTableView {
    weak var store: CleanUpStore?

    override func keyDown(with event: NSEvent) {
        guard let store, event.modifierFlags.intersection([.command, .control, .option]).isEmpty else {
            super.keyDown(with: event)
            return
        }
        switch event.charactersIgnoringModifiers {
        case " ":
            let keys = (delegate as? CleanUpGroupList.Coordinator)?.highlightedKeys ?? []
            if keys.isEmpty { NSSound.beep() } else { store.toggleTicks(keys) }
        case "e": act(.archive)
        case "#": act(.trash)
        case "!": act(.spam)
        case "j": move(by: 1)
        case "k": move(by: -1)
        default:
            if event.keyCode == 51 || event.keyCode == 117 { // delete, forward delete
                act(.trash)
            } else {
                super.keyDown(with: event)
            }
        }
    }

    private func act(_ action: CleanupAction) {
        guard let store, store.canAct else {
            NSSound.beep()
            return
        }
        Task { await store.apply(action) }
    }

    private func move(by delta: Int) {
        guard numberOfRows > 0 else { return }
        let current = selectedRow < 0 ? (delta > 0 ? -1 : numberOfRows) : selectedRow
        let next = min(max(current + delta, 0), numberOfRows - 1)
        selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
        scrollRowToVisible(next)
    }
}

/// One group: checkbox, title, count; under the title, in the secondary
/// style, the address and the other names it went by.
final class CleanUpGroupRowView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("CleanUpGroupRow")

    static func height(detailLine: Bool) -> CGFloat { detailLine ? 48 : 32 }

    private let checkbox = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    private let title = CleanUpGroupRowView.label(TypeRole.rowSender(unread: false))
    private let detail = CleanUpGroupRowView.label(TypeRole.rowSecondary)
    private let count = CleanUpGroupRowView.label(TypeRole.rowCount)
    private let separator = NSView()
    private var onToggle: (() -> Void)?

    private static let padding: CGFloat = 12
    private static let checkboxWidth: CGFloat = 18
    private static let lineHeight: CGFloat = 17

    override init(frame: NSRect) {
        super.init(frame: frame)
        identifier = Self.identifier
        checkbox.target = self
        checkbox.action = #selector(toggled)
        checkbox.setAccessibilityLabel("Tick")
        detail.textColor = .secondaryLabelColor
        count.alignment = .right
        count.textColor = .secondaryLabelColor
        separator.wantsLayer = true
        for view in [checkbox, title, detail, count, separator] { addSubview(view) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { separator.isHidden = backgroundStyle == .emphasized }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateSeparatorColor()
    }

    private func updateSeparatorColor() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            separator.layer?.backgroundColor = NSColor.separatorColor.cgColor
        }
    }

    /// The second line: "billing@example.net · aka Example Billing, Billing
    /// Team", led by "Unsubscribed" once the user unsubscribed from it here.
    static func detailLine(_ group: CleanupGroup) -> String {
        var parts: [String] = group.unsubscribed ? ["Unsubscribed"] : []
        if let address = group.detail, !address.isEmpty, address != group.title { parts.append(address) }
        if !group.aka.isEmpty { parts.append("aka " + group.aka.joined(separator: ", ")) }
        return parts.joined(separator: " · ")
    }

    func configure(_ group: CleanupGroup, ticked: Bool, onToggle: @escaping () -> Void) {
        self.onToggle = onToggle
        title.stringValue = group.title
        detail.stringValue = Self.detailLine(group)
        detail.isHidden = detail.stringValue.isEmpty
        count.stringValue = Int(group.count).formatted()
        setTicked(ticked)
        updateSeparatorColor()
        let messages = group.count == 1 ? "1 message" : "\(Int(group.count).formatted()) messages"
        setAccessibilityLabel([group.title, detail.isHidden ? nil : detail.stringValue, messages]
            .compactMap { $0 }.joined(separator: ", "))
        toolTip = group.aka.isEmpty ? nil : "Also sent as " + group.aka.joined(separator: ", ")
        needsLayout = true
    }

    func setTicked(_ ticked: Bool) {
        checkbox.state = ticked ? .on : .off
        checkbox.setAccessibilityValue(ticked ? "Ticked" : "Not ticked")
    }

    @objc private func toggled() { onToggle?() }

    override func layout() {
        super.layout()
        let p = Self.padding
        let w = bounds.width
        let h = bounds.height
        let line = Self.lineHeight
        let textX = p + Self.checkboxWidth + 6
        let countWidth = ceil((count.stringValue as NSString).size(withAttributes: [.font: TypeRole.rowCount]).width) + 4
        // Centred as one block: the title alone, or the title over the detail.
        let block = detail.isHidden ? line : 2 * line - 1
        let top = (h + block) / 2
        checkbox.frame = NSRect(x: p, y: top - line, width: Self.checkboxWidth, height: line)
        count.frame = NSRect(x: w - p - countWidth, y: top - line, width: countWidth, height: line)
        title.frame = NSRect(x: textX, y: top - line, width: max(0, w - textX - countWidth - p - 6), height: line)
        detail.frame = NSRect(x: textX, y: top - 2 * line + 1, width: max(0, w - textX - p), height: line)
        let hairline = 1 / max(window?.backingScaleFactor ?? 2, 1)
        separator.frame = NSRect(x: textX, y: 0, width: max(0, w - textX), height: hairline)
    }

    private static func label(_ font: NSFont) -> NSTextField {
        let field = NSTextField(labelWithString: "")
        field.font = font
        field.lineBreakMode = .byTruncatingTail
        field.maximumNumberOfLines = 1
        field.isSelectable = false
        return field
    }
}

/// The messages column: every message in the ticked groups, newest first,
/// fetched a page at a time as rows come into view.
struct CleanUpMessageList: NSViewRepresentable {
    @Environment(AppModel.self) private var model

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
        table.headerView = nil
        table.style = .inset
        table.rowHeight = CleanUpMessageRowView.height
        table.usesAutomaticRowHeights = false
        table.intercellSpacing = .zero
        table.allowsMultipleSelection = false
        table.allowsEmptySelection = true
        table.backgroundColor = .clear
        let column = NSTableColumn(identifier: CleanUpMessageRowView.identifier)
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.target = context.coordinator
        table.doubleAction = #selector(Coordinator.openClicked(_:))
        table.setAccessibilityLabel("Messages")
        context.coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let store = model.cleanUp
        context.coordinator.update(count: store.messageCount, generation: store.messageGeneration,
                                   revision: store.pageRevision, showsSize: store.view.showsSize)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        private let model: AppModel
        private var count = 0
        private var generation = -1
        private var revision = -1
        private var showsSize = false
        weak var table: NSTableView?

        init(model: AppModel) { self.model = model }

        func update(count newCount: Int, generation newGeneration: Int, revision newRevision: Int, showsSize newShowsSize: Bool) {
            guard let table else { return }
            if newGeneration != generation || newCount != count || newShowsSize != showsSize {
                let restart = newGeneration != generation
                count = newCount
                generation = newGeneration
                revision = newRevision
                showsSize = newShowsSize
                table.reloadData()
                if restart, count > 0 { table.scrollRowToVisible(0) }
            } else if newRevision != revision {
                revision = newRevision
                // A page arrived: fill the rows that were waiting for it.
                let visible = table.rows(in: table.visibleRect)
                guard visible.length > 0 else { return }
                table.reloadData(forRowIndexes: IndexSet(integersIn: visible.location..<(visible.location + visible.length)),
                                 columnIndexes: [0])
            }
        }

        func numberOfRows(in tableView: NSTableView) -> Int { count }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            let cell = tableView.makeView(withIdentifier: CleanUpMessageRowView.identifier, owner: nil) as? CleanUpMessageRowView
                ?? CleanUpMessageRowView()
            cell.configure(model.cleanUp.message(at: row), showsSize: showsSize)
            return cell
        }

        /// A double-click opens the message's conversation in a window.
        @objc func openClicked(_ sender: NSTableView) {
            guard let account = model.cleanUp.accountID, let message = model.cleanUp.message(at: sender.clickedRow) else { return }
            model.openThreadWindow?(ThreadWindowRequest(accountID: account, threadID: message.threadId))
        }
    }
}

/// One message, in the thread row's calm style: the sender and the date,
/// the subject under them, and its size in the Size view.
final class CleanUpMessageRowView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("CleanUpMessageRow")
    static let height: CGFloat = 48

    private let sender = CleanUpMessageRowView.label(TypeRole.rowSender(unread: false))
    private let date = CleanUpMessageRowView.label(TypeRole.rowSecondary)
    private let subject = CleanUpMessageRowView.label(TypeRole.rowSubject(unread: false))
    private let size = CleanUpMessageRowView.label(TypeRole.rowSecondary)
    private let separator = NSView()

    private static let padding: CGFloat = 12
    private static let lineHeight: CGFloat = 17

    private static let sizes: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    override init(frame: NSRect) {
        super.init(frame: frame)
        identifier = Self.identifier
        date.alignment = .right
        date.textColor = .secondaryLabelColor
        size.alignment = .right
        size.textColor = .secondaryLabelColor
        separator.wantsLayer = true
        for view in [sender, date, subject, size, separator] { addSubview(view) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { separator.isHidden = backgroundStyle == .emphasized }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateSeparatorColor()
    }

    private func updateSeparatorColor() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            separator.layer?.backgroundColor = NSColor.separatorColor.cgColor
        }
    }

    /// "Billing Team", else the address; "(unknown sender)" with neither.
    static func senderText(_ message: CleanupMessage) -> String {
        guard let from = message.from else { return "(unknown sender)" }
        let name = from.name?.trimmingCharacters(in: .whitespaces) ?? ""
        return name.isEmpty ? from.email : name
    }

    static func sizeText(_ bytes: UInt64) -> String { sizes.string(fromByteCount: Int64(clamping: bytes)) }

    /// `nil`: the row's page is still loading; the row stays blank.
    func configure(_ message: CleanupMessage?, showsSize: Bool) {
        updateSeparatorColor()
        guard let message else {
            for field in [sender, date, subject, size] { field.stringValue = "" }
            setAccessibilityLabel("Loading")
            return
        }
        sender.stringValue = Self.senderText(message)
        date.stringValue = RowDateFormatter.string(forMillis: message.date)
        subject.stringValue = message.subject.isEmpty ? "(no subject)" : message.subject
        size.stringValue = showsSize ? Self.sizeText(message.size) : ""
        size.isHidden = !showsSize
        setAccessibilityLabel([sender.stringValue, subject.stringValue, date.stringValue,
                               showsSize ? size.stringValue : nil].compactMap { $0 }.joined(separator: ", "))
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let p = Self.padding
        let w = bounds.width
        let line = Self.lineHeight
        let top = (bounds.height + 2 * line - 1) / 2
        let width = { (field: NSTextField) in
            ceil((field.stringValue as NSString).size(withAttributes: [.font: field.font ?? TypeRole.rowSecondary]).width) + 4
        }
        let dateWidth = width(date)
        let sizeWidth = size.isHidden ? 0 : width(size)
        date.frame = NSRect(x: w - p - dateWidth, y: top - line, width: dateWidth, height: line)
        sender.frame = NSRect(x: p, y: top - line, width: max(0, w - 2 * p - dateWidth - 6), height: line)
        size.frame = NSRect(x: w - p - sizeWidth, y: top - 2 * line + 1, width: sizeWidth, height: line)
        subject.frame = NSRect(x: p, y: top - 2 * line + 1, width: max(0, w - 2 * p - sizeWidth - (size.isHidden ? 0 : 6)),
                               height: line)
        let hairline = 1 / max(window?.backingScaleFactor ?? 2, 1)
        separator.frame = NSRect(x: p, y: 0, width: max(0, w - p), height: hairline)
    }

    private static func label(_ font: NSFont) -> NSTextField {
        let field = NSTextField(labelWithString: "")
        field.font = font
        field.lineBreakMode = .byTruncatingTail
        field.maximumNumberOfLines = 1
        field.isSelectable = false
        return field
    }
}
