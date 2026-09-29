import SwiftUI

/// List filters (spec §14.3 amendment 2026-09-28, filters): narrow the
/// current mailbox or search, combinable, per window and not remembered.
/// A listing gets them as narrowings (`INBOX+@unread`), a search as the
/// matching operators.
enum ListFilter: String, CaseIterable, Identifiable {
    case unread = "@unread"
    case starred = "@starred"
    case attachments = "@attachments"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .unread: "Unread"
        case .starred: "Starred"
        case .attachments: "With Attachments"
        }
    }

    /// The same filter as a search operator (spec §8).
    var searchOperator: String {
        switch self {
        case .unread: "is:unread"
        case .starred: "is:starred"
        case .attachments: "has:attachment"
        }
    }

    /// In a stable order, so the same filters make the same listing id.
    static func ordered(_ filters: Set<ListFilter>) -> [ListFilter] {
        allCases.filter(filters.contains)
    }
}

/// The filter button in the list column's header, as in Mail: filled
/// while a filter is on.
struct ListFilterMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let active = !model.listFilters.isEmpty
        Menu {
            ForEach(ListFilter.allCases) { filter in
                Toggle(filter.title, isOn: Binding(
                    get: { model.listFilters.contains(filter) },
                    set: { on in
                        if on { model.listFilters.insert(filter) } else { model.listFilters.remove(filter) }
                    }))
            }
            if active {
                Divider() // menu
                Button("Clear Filters") { model.listFilters = [] }
            }
        } label: {
            Label("Filter", systemImage: active ? "line.3.horizontal.decrease.circle.fill"
                                                : "line.3.horizontal.decrease.circle")
        }
        .labelStyle(.iconOnly)
        .menuIndicator(.hidden)
        .help(ToolbarHelp.text(for: "Filter", model: model) ?? "") // toolbar
        .accessibilityValue(active ? ListFilter.ordered(model.listFilters).map(\.title).joined(separator: ", ") : "None")
    }
}

/// View options for the list, in the title bar as in Mail: Important
/// Only in the Inbox, and Show Categories when the account uses them.
struct ListViewOptionsMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        Menu {
            Toggle("Important Only", isOn: $model.inboxImportantOnly)
            Toggle("Hide Emails with Tasks", isOn: $model.inboxHidesTasks)
            if InboxCategories.inUse(model.inboxCategoryCounts) {
                Toggle("Show Categories", isOn: $model.showCategories)
            }
        } label: {
            Label("View Options", systemImage: "ellipsis.circle")
        }
        .labelStyle(.iconOnly)
        .menuIndicator(.hidden)
        .help(ToolbarHelp.text(for: "View Options", model: model) ?? "") // toolbar
    }
}
