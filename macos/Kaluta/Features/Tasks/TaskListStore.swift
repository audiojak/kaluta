import Foundation
import Observation

/// The task list (spec §14.8): the open account's tasks, grouped by when
/// they are due, or the finished ones.
@MainActor
@Observable
final class TaskListStore {
    struct Section: Identifiable, Equatable {
        let group: DueDay.Group
        let tasks: [TaskItem]
        var id: Int { group.rawValue }
    }

    private(set) var tasks: [TaskItem] = []
    /// Finished tasks instead of open ones.
    var showsDone = false {
        didSet { if showsDone != oldValue { Task { await load() } } }
    }
    var selectedID: Int64?
    /// Open tasks due today or overdue: the sidebar's badge.
    private(set) var dueCount = 0
    private(set) var openCount = 0
    /// The account's categories, for the menus and the `c` chooser.
    private(set) var categories: [String] = []
    /// `c`: the category chooser is open.
    var choosingCategory = false
    private(set) var loaded = false
    private(set) var error: String?

    @ObservationIgnored private let core: CoreClient?
    /// Loads are numbered; one finishing after a newer one was applied is
    /// dropped, so older data never replaces newer.
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var applied = 0

    init(core: CoreClient?) {
        self.core = core
    }

    func load() async {
        guard let core else { return }
        generation += 1
        let mine = generation
        let done = showsDone
        do {
            let all = try await core.listTasks(includeDone: done)
            let found = try await core.taskCategories()
            guard mine > applied, done == showsDone else { return }
            applied = mine
            let open = all.filter { !$0.done }
            let shown = done ? all.filter(\.done) : open
            if shown != tasks { tasks = shown }
            dueCount = Self.dueCount(open)
            categories = found
            openCount = open.count
            error = nil
        } catch let error as CoreClientError {
            self.error = error.message
        } catch {
            self.error = error.localizedDescription
        }
        loaded = true
        if let selectedID, !tasks.contains(where: { $0.id == selectedID }) { self.selectedID = nil }
    }

    var selected: TaskItem? { selectedID.flatMap { id in tasks.first { $0.id == id } } }

    /// Open tasks under Overdue, Today, This Week, Later and No Date; done
    /// ones in one section, latest first.
    func sections(now: Date = .now) -> [Section] {
        if showsDone { return tasks.isEmpty ? [] : [Section(group: .noDate, tasks: tasks)] }
        let grouped = Dictionary(grouping: tasks) { DueDay.group($0.dueDay, now: now) }
        return DueDay.Group.allCases.compactMap { group in
            grouped[group].map { Section(group: group, tasks: $0) }
        }
    }

    /// The next task to select after `id` leaves the list.
    func neighbour(of id: Int64) -> Int64? {
        guard let index = tasks.firstIndex(where: { $0.id == id }) else { return nil }
        if index + 1 < tasks.count { return tasks[index + 1].id }
        return index > 0 ? tasks[index - 1].id : nil
    }

    /// Open tasks due today or overdue (the sidebar's badge).
    static func dueCount(_ tasks: [TaskItem], now: Date = .now) -> Int {
        tasks.filter { !$0.done && [.overdue, .today].contains(DueDay.group($0.dueDay, now: now)) }.count
    }
}
