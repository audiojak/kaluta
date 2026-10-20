import SwiftUI

/// One key on the key list: the key as drawn, and what it does here.
struct WorkflowKey: Equatable {
    let key: String
    let title: String
}

struct WorkflowKeyGroup: Equatable, Identifiable {
    let title: String
    let keys: [WorkflowKey]
    var id: String { title }
}

/// The key list (spec §14.13, ?): built from the commands the screen
/// offers, so it lists what works here and names each as its button does.
enum WorkflowKeys {
    private static let conversation: [WorkflowCommand] = [.conversation, .previousMessage, .nextMessage]
    private static let email: [WorkflowCommand] = [.openInFull, .openInGmail, .replyAll, .forward]

    static func groups(_ actions: [WorkflowAction], writing: Bool, loadImages: Bool) -> [WorkflowKeyGroup] {
        var decide: [WorkflowKey] = []
        var mail: [WorkflowKey] = []
        var talk: [WorkflowKey] = []
        var seen = Set<String>()
        for action in actions where action.unavailable == nil {
            guard let key = keys(action.command), seen.insert(key).inserted else { continue }
            let row = WorkflowKey(key: key, title: action.title)
            if conversation.contains(action.command) {
                talk.append(row)
            } else if email.contains(action.command) {
                mail.append(row)
            } else if action.command != .back && action.command != .showKeys {
                decide.append(row)
            }
        }
        if loadImages { mail.append(WorkflowKey(key: "⇧⌘I", title: "Load images")) }
        // In the order the conversation's line shows them.
        talk.sort { a, b in order(a.key) < order(b.key) }
        let back = actions.first { $0.command == .back }?.title ?? (writing ? "Cancel" : "Back to Mail")
        let around = [WorkflowKey(key: "Esc", title: back), WorkflowKey(key: "?", title: "Show or hide these keys")]
        return [WorkflowKeyGroup(title: "Decide", keys: decide), WorkflowKeyGroup(title: "The email", keys: mail),
                WorkflowKeyGroup(title: "The conversation", keys: talk),
                WorkflowKeyGroup(title: "Getting around", keys: around)]
            .filter { !$0.keys.isEmpty }
    }

    /// Reply is Return and R.
    private static func keys(_ command: WorkflowCommand) -> String? {
        command == .replyNow ? "Return, R" : command.keyLabel
    }

    private static func order(_ key: String) -> Int { ["C", "P", "N"].firstIndex(of: key) ?? 3 }
}

/// The key list as a sheet: a column per pair of groups. Esc, ? or Done
/// closes it.
struct WorkflowKeysView: View {
    let groups: [WorkflowKeyGroup]
    let kind: WorkflowKind
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            HStack {
                Text("Keys in \(kind.title)").font(TypeRole.workflowHeading)
                Spacer()
                Button("Done", action: close)
                    .keyboardShortcut(.cancelAction)
                    .hoverHelp("Close (Esc or ?)")
            }
            LazyVGrid(columns: [GridItem(.flexible(), alignment: .top), GridItem(.flexible(), alignment: .top)],
                      alignment: .leading, spacing: Space.xl) {
                ForEach(groups) { group in
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text(group.title)
                            .font(TypeRole.workflowContext).fontWeight(.semibold)
                            .foregroundStyle(.secondary)
                        ForEach(group.keys, id: \.key) { row in
                            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                                Keycap(key: row.key)
                                    .frame(minWidth: Self.keyColumn, alignment: .leading)
                                Text(row.title)
                            }
                            .accessibilityElement(children: .combine)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .font(TypeRole.workflowContext)
        .padding(Space.xxxl)
        .frame(width: Self.width)
        .onKeyPress("?") {
            close()
            return .handled
        }
    }

    static let width: CGFloat = 560
    static let keyColumn: CGFloat = 72
}
