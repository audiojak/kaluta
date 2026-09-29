import SwiftUI

/// Settings › Tasks (spec §14.8): the open account's task categories, in
/// order. Claude picks from them; tasks keep a category's name when it is
/// removed.
struct TaskSettings: View {
    @Environment(AppModel.self) private var model
    @State private var editor = TaskCategoryEditor()

    var body: some View {
        Form {
            Section {
                List {
                    ForEach($editor.names, id: \.self) { $name in
                        HStack(spacing: Space.m) {
                            CategoryChip(name: name)
                            Spacer(minLength: 0)
                            Button("Remove", systemImage: "minus.circle") { editor.remove(name) }
                                .labelStyle(.iconOnly)
                                .buttonStyle(.borderless)
                                .disabled(editor.names.count == 1)
                                .hoverHelp("Remove \(name); tasks in it keep the name")
                        }
                    }
                    .onMove { editor.move(from: $0, to: $1) }
                }
                .frame(minHeight: Self.listHeight)
                HStack(spacing: Space.m) {
                    TextField("New category", text: $editor.newName)
                        .onSubmit { editor.add() }
                    Button("Add") { editor.add() }
                        .disabled(!editor.canAdd)
                        .hoverHelp("Add this category at the end")
                }
                if let error = editor.error {
                    Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure)
                }
            } header: {
                Text("Task Categories")
            } footer: {
                Text("Claude chooses one of these for each task. Drag to reorder.")
            }
            Section {
                Button("Reset to the Starting Set") { editor.reset() }
                    .hoverHelp("Reply, Decide, Gather Info, Schedule, Review, Admin and Follow Up")
            }
        }
        .formStyle(.grouped)
        .task(id: model.openAccountID) { await editor.load(core: model.core) }
        .onChange(of: editor.names) { _, names in editor.saveSoon(names) }
    }

    private static let listHeight: CGFloat = 200
}

/// The categories being edited: saved to the core as they change.
@MainActor
@Observable
final class TaskCategoryEditor {
    var names: [String] = []
    var newName = ""
    private(set) var error: String?
    @ObservationIgnored private var core: CoreClient?
    @ObservationIgnored private var saved: [String] = []

    func load(core: CoreClient?) async {
        self.core = core
        let loaded = (try? await core?.taskCategories()) ?? []
        saved = loaded
        names = loaded
    }

    var canAdd: Bool {
        let name = newName.trimmingCharacters(in: .whitespaces)
        return !name.isEmpty && !names.contains { $0.caseInsensitiveCompare(name) == .orderedSame }
    }

    func add() {
        guard canAdd else { return }
        names.append(newName.trimmingCharacters(in: .whitespaces))
        newName = ""
    }

    func remove(_ name: String) {
        guard names.count > 1 else { return }
        names.removeAll { $0 == name }
    }

    func move(from: IndexSet, to: Int) {
        names.move(fromOffsets: from, toOffset: to)
    }

    func reset() {
        Task {
            guard let core else { return }
            do {
                let fresh = try await core.resetTaskCategories()
                saved = fresh
                names = fresh
                error = nil
            } catch let error as CoreClientError {
                self.error = error.message
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    func saveSoon(_ names: [String]) {
        guard names != saved, let core else { return }
        Task { await save(names, core: core) }
    }

    func save(_ names: [String], core: CoreClient) async {
        do {
            let stored = try await core.setTaskCategories(names)
            saved = stored
            if stored != self.names { self.names = stored }
            error = nil
        } catch let error as CoreClientError {
            self.error = error.message
        } catch {
            self.error = error.localizedDescription
        }
    }
}
