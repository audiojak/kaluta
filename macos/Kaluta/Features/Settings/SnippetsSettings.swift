import SwiftUI

/// Settings › Snippets (spec §14.5 amendment 2026-11-18): saved text to
/// put into a message by typing / at the start of a line. Every account
/// has the same snippets.
struct SnippetsSettings: View {
    @Environment(AppModel.self) private var model
    @State private var snippets: [SnippetInfo] = []
    @State private var selection: Int64?
    @State private var name = ""
    @State private var body_ = ""
    /// The selected snippet's text as loaded: its formatting is kept unless
    /// the text is changed here.
    @State private var loadedText = ""
    @State private var error: String?

    private var selected: SnippetInfo? { snippets.first { $0.id == selection } }

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Type / at the start of a line in a message to put one in. {first name} becomes the first recipient's first name.")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HSplitView {
                List(snippets, id: \.id, selection: $selection) { snippet in
                    Text(snippet.name).lineLimit(1)
                }
                .frame(minWidth: 180)
                editor.frame(minWidth: 280)
            }
            HStack(spacing: Space.m) {
                Button("New Snippet") { Task { await add() } }
                    .hoverHelp("Make a new snippet")
                Button("Delete") { Task { await delete() } }
                    .disabled(selected == nil)
                    .hoverHelp("Delete the selected snippet (⌘Z in the mail window does not bring it back)")
                Spacer(minLength: 0)
                if let error { Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure) }
            }
            .controlSize(.small)
        }
        .padding(Space.xl)
        .task { await load() }
        .onChange(of: selection) { show(selected) }
    }

    @ViewBuilder private var editor: some View {
        if selected != nil {
            VStack(alignment: .leading, spacing: Space.s) {
                TextField("Name", text: $name)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { Task { await save() } }
                TextEditor(text: $body_)
                    .font(TypeRole.body)
                    .scrollContentBackground(.hidden)
                    .padding(Space.xs)
                    .background(Tone.controlFill, in: RoundedRectangle(cornerRadius: Radius.control))
                    .accessibilityLabel("Text")
                HStack {
                    Spacer()
                    Button("Save") { Task { await save() } }
                        .keyboardShortcut("s")
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                        .hoverHelp("Save the name and text (⌘S)")
                }
            }
            .padding(.leading, Space.m)
        } else {
            ContentUnavailableView("No Snippet Selected", systemImage: "text.quote")
        }
    }

    private func load() async {
        snippets = (try? await model.core?.snippets()) ?? []
        if selection == nil { selection = snippets.first?.id }
        show(selected)
    }

    private func show(_ snippet: SnippetInfo?) {
        name = snippet?.name ?? ""
        let text = snippet.map { ComposerHTML.attributedString(fromHTML: $0.bodyHtml).string } ?? ""
        body_ = text
        loadedText = text
        error = nil
    }

    private func add() async {
        guard let core = model.core else { return }
        do {
            selection = try await core.saveSnippet(SnippetInfo(id: 0, name: "New snippet", bodyHtml: "", updatedAt: 0))
            await load()
        } catch {
            self.error = error.message
        }
    }

    private func save() async {
        guard let core = model.core, let snippet = selected else { return }
        // Formatting saved from a message is kept unless the text changed.
        let html = body_ == loadedText ? snippet.bodyHtml : SnippetText.html(fromPlain: body_)
        do {
            try await core.saveSnippet(SnippetInfo(id: snippet.id, name: name, bodyHtml: html, updatedAt: 0))
            await load()
        } catch {
            self.error = error.message
        }
    }

    private func delete() async {
        guard let core = model.core, let snippet = selected else { return }
        do {
            try await core.deleteSnippet(snippet.id)
            selection = nil
            await load()
        } catch {
            self.error = error.message
        }
    }
}
