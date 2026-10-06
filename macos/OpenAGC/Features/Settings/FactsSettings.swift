import SwiftUI

/// Settings › Facts (spec §14.11, ADR 0012): the global facts, which every
/// account's AI drafts may use, with the same editing as the Facts tab.
struct FactsSettings: View {
    @Environment(AppModel.self) private var model
    @State private var store: FactsStore?
    @State private var editing: FactSheet?

    private enum FactSheet: Identifiable {
        case fact(FactInfo?)
        case category
        case categories
        var id: String {
            switch self {
            case let .fact(f): "fact-\(f.map { FactsStore.tag($0) } ?? "new")"
            case .category: "category"
            case .categories: "categories"
            }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Global facts: every account's AI drafts may use them. A fact an account has its own version of uses that instead. Make a fact global from Analysis › Facts.")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let store {
                HSplitView {
                    FactsList(store: store)
                        .frame(minWidth: Self.listWidth)
                    Group {
                        if let fact = store.selected {
                            FactDetail(fact: fact, store: store) { editing = .fact($0) }
                        } else {
                            ContentUnavailableView("No Fact Selected", systemImage: "globe")
                        }
                    }
                    .frame(minWidth: Self.listWidth)
                }
            }
            HStack(spacing: Space.m) {
                Button("Add Fact…") { editing = .fact(nil) }
                    .hoverHelp("Write a fact every account's drafts may use")
                Button("Add Category…") { editing = .category }
                    .hoverHelp("A category of your own for global facts")
                Button("Categories…") { editing = .categories }
                    .hoverHelp("Hide built-in categories, rename or delete your own")
                Spacer(minLength: 0)
            }
            .controlSize(.small)
        }
        .padding(Space.xl)
        .task {
            if store == nil { store = FactsStore(core: model.core, scope: .global) }
            await store?.load()
        }
        .sheet(item: $editing) { sheet in
            if let store {
                switch sheet {
                case let .fact(f):
                    FactEditor(fact: f, scope: .global, categories: store.categories) {
                        editing = nil
                        Task { await store.load() }
                    }
                case .category:
                    NewFactCategorySheet(scope: .global) {
                        editing = nil
                        Task { await store.load() }
                    }
                case .categories:
                    FactCategoriesSheet(store: store) { editing = nil }
                }
            }
        }
    }

    private static let listWidth: CGFloat = 260
}
