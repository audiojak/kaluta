import SwiftUI

/// The task list (spec §14.8): tasks grouped by when they are due, in the
/// thread list's calm style. Choosing one shows its email in the reader.
/// Keys: `↩` edit, `r` `a` `f` answer the email, `e` done, `c` category,
/// `⌫` delete, `j` `k` move.
struct TaskListView: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        let sections = model.tasks.sections()
        List(selection: Binding(get: { model.tasks.selectedID }, set: { model.selectTask($0) })) {
            ForEach(sections) { section in
                Section {
                    ForEach(section.tasks, id: \.id) { task in
                        TaskRow(task: task)
                            .tag(task.id)
                            .contextMenu { TaskMenu(task: task) }
                    }
                } header: {
                    if !model.tasks.showsDone {
                        Text(section.group.title)
                            .foregroundStyle(section.group == .overdue ? Tone.caution : .secondary)
                    }
                }
            }
        }
        .listStyle(.inset)
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(characters: .init(charactersIn: "raefcjk"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            return handle(press.characters) ? .handled : .ignored
        }
        .popover(isPresented: Bindable(model.tasks).choosingCategory, arrowEdge: .trailing) {
            CategoryChooser()
        }
        .onKeyPress(.return) { edit() }
        .onKeyPress(.delete) { delete() }
        .onKeyPress(.deleteForward) { delete() }
        .overlay {
            if model.tasks.loaded, sections.isEmpty {
                if model.tasks.showsDone {
                    ContentUnavailableView("No Finished Tasks", systemImage: "checkmark.circle")
                } else {
                    ContentUnavailableView("No Tasks", systemImage: "checklist",
                                           description: Text("Press t on an email to make a task of it"))
                }
            }
        }
    }

    private func handle(_ key: String) -> Bool {
        guard model.tasks.selected != nil || "jk".contains(key) else { return false }
        switch key {
        case "r": model.reply(all: false)
        case "a": model.reply(all: true)
        case "f": model.forward()
        case "e": Task { await model.toggleSelectedTaskDone() }
        case "c": model.tasks.choosingCategory = true
        case "j": move(1)
        case "k": move(-1)
        default: return false
        }
        return true
    }

    private func edit() -> KeyPress.Result {
        guard model.tasks.selected != nil else { return .ignored }
        Task { await model.editSelectedTask() }
        return .handled
    }

    private func delete() -> KeyPress.Result {
        guard model.tasks.selected != nil else { return .ignored }
        Task { await model.deleteSelectedTask() }
        return .handled
    }

    private func move(_ delta: Int) {
        let ids = model.tasks.sections().flatMap(\.tasks).map(\.id)
        guard !ids.isEmpty else { return }
        let index = model.tasks.selectedID.flatMap { ids.firstIndex(of: $0) } ?? (delta > 0 ? -1 : ids.count)
        model.selectTask(ids[min(max(index + delta, 0), ids.count - 1)])
    }
}

/// One task: its title and due day, then its category and the email.
private struct TaskRow: View {
    @Environment(AppModel.self) private var model
    let task: TaskItem

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                if task.done {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
                Text(task.title)
                    .font(Font(TypeRole.rowSender(unread: !task.done)))
                    .strikethrough(task.done, color: .secondary)
                    .lineLimit(2)
                Spacer(minLength: Space.m)
                if let day = task.dueDay, !task.done {
                    Text(DueDay.label(day))
                        .font(Font(TypeRole.rowSecondary))
                        .foregroundStyle(dueStyle(day))
                }
            }
            HStack(spacing: Space.s) {
                CategoryChip(name: task.category)
                Text(emailLine)
                    .font(Font(TypeRole.rowSecondary))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, Space.xs)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
        .accessibilityAction(named: task.done ? "Reopen" : "Done") {
            model.selectTask(task.id)
            Task { await model.toggleSelectedTaskDone() }
        }
    }

    private var emailLine: String {
        let who = task.senderName ?? task.senderEmail
        return [who, task.subject.isEmpty ? nil : task.subject].compactMap { $0 }.joined(separator: " · ")
    }

    private func dueStyle(_ day: String) -> AnyShapeStyle {
        switch DueDay.urgency(day) {
        case .overdue: AnyShapeStyle(Tone.caution)
        case .today: AnyShapeStyle(Tone.unread)
        default: AnyShapeStyle(.secondary)
        }
    }

    private var accessibilityText: String {
        var parts = [task.title, task.category]
        if task.done { parts.insert("Done", at: 0) }
        if let day = task.dueDay, !task.done {
            parts.append(DueDay.urgency(day) == .overdue ? "overdue, due \(DueDay.label(day))" : "due \(DueDay.label(day))")
        }
        parts.append(emailLine)
        return parts.joined(separator: ", ")
    }
}

/// `c`: the categories, each with its number key.
private struct CategoryChooser: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text("Category").font(TypeRole.groupLabel)
            ForEach(Array(model.tasks.categories.prefix(9).enumerated()), id: \.element) { index, name in
                Button {
                    model.tasks.choosingCategory = false
                    Task { await model.setSelectedTaskCategory(name) }
                } label: {
                    HStack(spacing: Space.m) {
                        Text("\(index + 1)").font(TypeRole.meta.monospacedDigit()).foregroundStyle(.secondary)
                        CategoryChip(name: name, selected: name == model.tasks.selected?.category)
                        Spacer(minLength: 0)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: [])
                .hoverHelp("Move the task to \(name) (\(index + 1))")
            }
        }
        .padding(Space.l)
        .frame(minWidth: Self.width)
    }

    private static let width: CGFloat = 180
}

/// A task's context menu: the same actions as its keys.
struct TaskMenu: View {
    @Environment(AppModel.self) private var model
    let task: TaskItem

    var body: some View {
        Button("Edit Task…") { select(); Task { await model.editSelectedTask() } } // no-help: menu
        Divider() // menu
        Button("Reply") { select(); model.reply(all: false) } // no-help: menu
        Button("Reply All") { select(); model.reply(all: true) } // no-help: menu
        Button("Forward") { select(); model.forward() } // no-help: menu
        Divider() // menu
        Button(task.done ? "Mark as Not Done" : "Mark as Done") { // no-help: menu
            select()
            Task { await model.toggleSelectedTaskDone() }
        }
        Menu("Category") { // no-help: context menu
            ForEach(model.tasks.categories, id: \.self) { name in
                Button(name) { select(); Task { await model.setSelectedTaskCategory(name) } } // no-help: menu
            }
        }
        Divider() // menu
        Button("Delete Task") { select(); Task { await model.deleteSelectedTask() } } // no-help: menu
    }

    private func select() {
        if model.tasks.selectedID != task.id { model.selectTask(task.id) }
    }
}
