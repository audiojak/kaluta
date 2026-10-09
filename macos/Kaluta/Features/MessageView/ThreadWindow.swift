import SwiftUI

/// A thread opened in a window of its own (Return or double-click in the
/// list, as in Apple Mail). It belongs to the account it was opened in.
struct ThreadWindowRequest: Codable, Hashable {
    var accountID: String
    var threadID: String
}

struct ThreadWindow: View {
    @Environment(AppModel.self) private var model
    let request: ThreadWindowRequest
    @State private var store: ReaderStore?

    var body: some View {
        Group {
            if model.openAccountID != request.accountID {
                ContentUnavailableView("Another Account Is Open", systemImage: "person.crop.circle",
                                       description: Text("This conversation belongs to an account that is not open now."))
            } else if let store {
                ThreadReaderView(store: store, threadID: request.threadID)
            } else {
                Color.clear
            }
        }
        .frame(minWidth: 480, minHeight: 360)
        .navigationTitle(store?.detail.map { $0.thread.subject.isEmpty ? "(no subject)" : $0.thread.subject } ?? "Message")
        .task {
            guard store == nil else { return }
            let made = ReaderStore(core: model.core)
            made.ownAddresses = model.reader.ownAddresses
            store = made
        }
    }
}
