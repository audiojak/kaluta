import SwiftUI

/// Audience groups (spec §14.9, D1): inferred from mail and confirmed by
/// the user, who can rename, merge, reject or add them; fewer than five
/// are filled from the obvious gaps, marked suggested. A recipient's
/// confirmed group scopes guidelines when AI drafts to them.
struct GuideAudiences: View {
    @Environment(AppModel.self) private var model
    @State private var error: String?

    var body: some View {
        let groups = model.guide.groups
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Audiences").font(TypeRole.groupLabel)
            Text("Who you write to differently. Guidelines can apply to one audience; drafts to someone in it follow them.")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(groups.filter { $0.status != .rejected }, id: \.id) { group in
                AudienceRow(group: group, others: groups.filter { $0.id != group.id && $0.status != .rejected },
                            error: $error)
            }
            HStack(spacing: Space.m) {
                Button("Add Audience") { Task { await add() } }
                    .hoverHelp("Add an audience of your own")
                if groups.filter({ $0.status != .rejected }).count < 5 {
                    Button("Suggest More") { Task { await fill() } }
                        .hoverHelp("Suggest common audiences until there are five")
                }
            }
            .controlSize(.small)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.caption).foregroundStyle(Tone.failure)
            }
        }
    }

    private func add() async {
        guard let core = model.core else { return }
        let taken = Set(model.guide.groups.map { $0.name.lowercased() })
        let name = (1...).lazy.map { $0 == 1 ? "New audience" : "New audience \($0)" }.first { !taken.contains($0.lowercased()) }!
        _ = try? await core.saveAudienceGroup(AudienceGroup(id: 0, name: name, status: .confirmed, description: "", members: []))
        await model.guide.load()
    }

    private func fill() async {
        _ = try? await model.core?.fillAudienceGroups()
        await model.guide.load()
    }
}

private struct AudienceRow: View {
    @Environment(AppModel.self) private var model
    let group: AudienceGroup
    let others: [AudienceGroup]
    @Binding var error: String?
    @State private var name = ""
    @State private var members = ""

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.s) {
                TextField("Name", text: $name)
                    .textFieldStyle(.plain)
                    .font(TypeRole.heading)
                    .onSubmit { Task { await rename() } }
                if group.status == .suggested {
                    Text("Suggested")
                        .font(Font(TypeRole.chip))
                        .padding(.horizontal, Space.xs)
                        .padding(.vertical, Space.hair)
                        .background(Tone.highlight, in: .rect(cornerRadius: Radius.chip))
                }
                Spacer(minLength: 0)
                if group.status == .suggested {
                    Button("Confirm") { Task { await save(status: .confirmed) } }
                        .hoverHelp("Keep this audience")
                }
                Menu("Merge Into") {
                    ForEach(others, id: \.id) { other in
                        Button(other.name) { Task { await merge(into: other) } } // no-help: menu
                    }
                }
                .fixedSize()
                .disabled(others.isEmpty)
                .hoverHelp("Combine this audience with another")
                if group.status == .confirmed, !model.guideRunActive {
                    Button("Improve") { Task { await model.improveGuide(group.name) } }
                        .hoverHelp("Analyse the sent messages to \(group.name) most likely to show how you write to them")
                }
                Button("Remove") { Task { await save(status: .rejected) } }
                    .hoverHelp("Remove this audience; it will not be suggested again")
            }
            .controlSize(.small)
            if !group.description.isEmpty {
                Text(group.description).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            TextField("Addresses or @domains, separated by commas", text: $members)
                .textFieldStyle(.roundedBorder)
                .onSubmit { Task { await save(status: group.status) } }
        }
        .padding(Space.l)
        .card(.neutral)
        .onAppear {
            name = group.name
            members = group.members.joined(separator: ", ")
        }
        .onChange(of: group.name) { _, new in name = new }
    }

    private func save(status: AudienceStatus) async {
        guard let core = model.core else { return }
        let list = members.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        do {
            _ = try await core.saveAudienceGroup(AudienceGroup(id: group.id, name: group.name, status: status,
                                                               description: group.description, members: list))
            error = nil
        } catch {
            self.error = error.message
        }
        await model.guide.load()
    }

    private func rename() async {
        guard let core = model.core, name != group.name else { return }
        do {
            _ = try await core.renameAudienceGroup(group.id, to: name)
            error = nil
        } catch {
            self.error = error.message
            name = group.name
        }
        await model.guide.load()
    }

    private func merge(into other: AudienceGroup) async {
        do {
            _ = try await model.core?.mergeAudienceGroups(into: other.id, from: group.id)
            error = nil
        } catch {
            self.error = error.message
        }
        await model.guide.load()
    }
}
