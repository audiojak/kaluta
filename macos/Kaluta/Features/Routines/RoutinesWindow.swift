import ServiceManagement
import SwiftUI

/// The Routines window (spec §11.5): routines on the left, the structured
/// editor on the right.
struct RoutinesWindow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let store = model.routines
        NavigationSplitView {
            RoutineList()
                .navigationSplitViewColumnWidth(min: 220, ideal: 260)
        } detail: {
            if store.draft != nil {
                RoutineEditor()
            } else {
                ContentUnavailableView {
                    Label("No Routine Selected", systemImage: "clock.arrow.2.circlepath")
                } description: {
                    Text("A routine files automated mail into labels on a schedule, so the inbox keeps only mail a person needs to answer.")
                } actions: {
                    NewRoutineMenu()
                }
            }
        }
        .frame(minWidth: 860, minHeight: 600)
        .task { await store.load() }
        .onChange(of: model.routinesRevision) { Task { await store.load() } }
    }
}

private struct NewRoutineMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Menu("New Routine") {
            ForEach(RoutineRunner.allCases) { runner in
                Button("Sort important mail — \(runner.title)") {
                    Task { await model.routines.create(runner: runner) }
                }
            }
        }
        .hoverHelp("Create a routine that sorts important mail on a schedule")
        .fixedSize()
    }
}

private struct RoutineList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var store = model.routines
        List(selection: $store.selectedID) {
            ForEach(store.routines, id: \.id) { routine in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: Space.hair) {
                        Text(routine.name).font(TypeRole.heading)
                        HStack(spacing: Space.s) {
                            Text(RoutineRunner(rawValue: routine.runner)?.badge ?? routine.runner)
                            if routine.changedSincePublish { Text("· unpublished changes").foregroundStyle(Tone.caution) }
                        }
                        .font(TypeRole.caption)
                        .foregroundStyle(.secondary)
                        Text(RoutinesStore.activity(store.latestRuns[routine.id]))
                            .font(TypeRole.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Toggle("Enabled", isOn: Binding(
                        get: { routine.enabled },
                        set: { on in Task { await store.setEnabled(routine.id, on) } }))
                        .labelsHidden()
                        .toggleStyle(.switch)
                        .controlSize(.mini)
                        .hoverHelp("\(routine.enabled ? "Pause" : "Resume") this routine's schedule")
                }
                .padding(.vertical, Space.hair)
                .tag(routine.id)
            }
        }
        .toolbar {
            ToolbarItem { NewRoutineMenu() }
        }
    }
}

private struct RoutineEditor: View {
    @Environment(AppModel.self) private var model
    @State private var editingPrompt = false
    /// Deleting a routine cannot be undone: asked first.
    @State private var confirmingDelete = false
    @State private var expanded: Set<String> = []

    var body: some View {
        @Bindable var store = model.routines
        if let binding = Binding($store.draft) {
            Form {
                header(binding)
                schedule(binding)
                scope(binding)
                leaveAlone(binding)
                buckets(binding)
                reportAndAdvanced(binding)
                if let preview = store.preview { previewSection(preview) }
                runsSection()
            }
            .formStyle(.grouped)
            .disabled(store.busy != nil && store.previewSession == nil)
            .safeAreaInset(edge: .bottom) { actionBar() }
            .sheet(isPresented: $editingPrompt) { PromptEditor(draft: binding) }
            .sheet(item: $store.handoff) { handoff in HandoffSheet(handoff: handoff) }
        }
    }

    // MARK: Sections

    private func header(_ d: Binding<RoutineDefinition>) -> some View {
        Section {
            TextField("Name", text: d.name)
            Picker("Runs on", selection: d.runner) {
                ForEach(RoutineRunner.allCases) { Text($0.title).tag($0.rawValue) }
            }
            .hoverHelp("Where the routine runs: here on this Mac, or in Claude's or ChatGPT's cloud")
            Text(model.routines.runner.explanation)
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if model.routines.runner == .local {
                Picker("Agent", selection: Binding(get: { d.wrappedValue.agent ?? "claude-code" }, set: { d.wrappedValue.agent = $0 })) {
                    Text("Claude").tag("claude-code")
                    Text("Codex").tag("codex")
                }
                .hoverHelp("Which agent runs this routine on this Mac")
                LaunchAtLoginToggle()
            }
            if let url = d.wrappedValue.cloud.routineUrl, let link = URL(string: url) {
                Link("Open at claude.ai", destination: link)
            }
            if d.wrappedValue.advancedPrompt != nil {
                Label("The prompt was edited by hand, so the settings below no longer change it.", systemImage: "pencil.and.outline")
                    .foregroundStyle(Tone.caution)
            }
        }
    }

    private func schedule(_ d: Binding<RoutineDefinition>) -> some View {
        let parsed = SchedulePreset.parse(d.wrappedValue.schedule.rrule)
        let set = { (preset: SchedulePreset, hour: Int, minute: Int, day: String) in
            if let rule = SchedulePreset.rule(preset, hour: hour, minute: minute, weekday: day) {
                d.wrappedValue.schedule.rrule = rule
            }
        }
        return Section("Schedule") {
            Picker("Repeat", selection: Binding(get: { parsed.0 }, set: { set($0, parsed.hour, parsed.minute, parsed.weekday) })) {
                ForEach(SchedulePreset.allCases) { Text($0.title).tag($0) }
            }
            .hoverHelp("How often the routine runs")
            switch parsed.0 {
            case .hourly:
                Stepper("At minute \(parsed.minute)", value: Binding(get: { parsed.minute }, set: { set(.hourly, parsed.hour, $0, parsed.weekday) }), in: 0...59)
            case .daily, .weekdays, .weekly:
                if parsed.0 == .weekly {
                    Picker("Day", selection: Binding(get: { parsed.weekday }, set: { set(.weekly, parsed.hour, parsed.minute, $0) })) {
                        ForEach(["MO", "TU", "WE", "TH", "FR", "SA", "SU"], id: \.self) { day in
                            Text(Calendar.current.weekdaySymbols[(["SU", "MO", "TU", "WE", "TH", "FR", "SA"].firstIndex(of: day) ?? 0)]).tag(day)
                        }
                    }
                    .hoverHelp("The day of the week it runs")
                }
                DatePicker("Time", selection: Binding(
                    get: { Calendar.current.date(bySettingHour: parsed.hour, minute: parsed.minute, second: 0, of: .now) ?? .now },
                    set: {
                        let c = Calendar.current.dateComponents([.hour, .minute], from: $0)
                        set(parsed.0, c.hour ?? 8, c.minute ?? 0, parsed.weekday)
                    }), displayedComponents: .hourAndMinute)
            case .custom:
                TextField("RRULE", text: d.schedule.rrule).font(TypeRole.code)
            }
            LabeledContent("Runs") { Text(model.core?.describeSchedule(d.wrappedValue.schedule.rrule) ?? "") }
            if let note = model.routines.runner.scheduleNote {
                Text(note).font(TypeRole.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func scope(_ d: Binding<RoutineDefinition>) -> some View {
        Section("Which mail") {
            TextField("Search", text: d.scope)
                .font(TypeRole.code)
                .onSubmit { Task { await model.routines.countScope() } }
            if let count = model.routines.scopeCount {
                Text(count >= 500 ? "Matches 500 or more threads right now" : "Matches \(count) thread\(count == 1 ? "" : "s") right now")
                    .font(TypeRole.caption).foregroundStyle(.secondary)
            }
            TextField("Parent label", text: d.parentLabel)
            Stepper("At most \(d.wrappedValue.limits.maxThreadsPerRun) threads per run", value: d.limits.maxThreadsPerRun, in: 10...500, step: 10)
        }
    }

    private func leaveAlone(_ d: Binding<RoutineDefinition>) -> some View {
        Section {
            Toggle("Conversations with real people", isOn: d.leaveAlone.humanThreads)
                .hoverHelp("Never file threads with a person on the other end")
            Toggle("Threads you have replied to", isOn: d.leaveAlone.repliedByMe)
                .hoverHelp("Never file threads you've taken part in")
            Toggle("Starred threads", isOn: d.leaveAlone.starred)
                .hoverHelp("Never file starred threads")
            Toggle("Spam and Trash", isOn: d.leaveAlone.spamTrash)
                .hoverHelp("Never touch mail in Spam or Trash")
            LinesField(title: "Other rules, one per line", lines: d.leaveAlone.custom)
        } header: {
            Text("Leave alone")
        } footer: {
            Text("Wrongly deferring a real conversation is much worse than leaving one extra message in the inbox.")
                .foregroundStyle(.secondary)
        }
    }

    private func buckets(_ d: Binding<RoutineDefinition>) -> some View {
        Section {
            ForEach(d.buckets) { $bucket in
                let index = d.wrappedValue.buckets.firstIndex { $0.id == bucket.id } ?? 0
                DisclosureGroup(isExpanded: Binding(
                    get: { expanded.contains(bucket.id) },
                    set: { if $0 { expanded.insert(bucket.id) } else { expanded.remove(bucket.id) } })) {
                    BucketEditor(bucket: $bucket, others: d.wrappedValue.buckets.filter { $0.id != bucket.id })
                    HStack {
                        Button("Move Up") { d.wrappedValue.buckets.swapAt(index, index - 1) }.disabled(index == 0)
                            .hoverHelp("Check this bucket earlier; a thread gets the first bucket that fits")
                        Button("Move Down") { d.wrappedValue.buckets.swapAt(index, index + 1) }
                            .hoverHelp("Check this bucket later")
                            .disabled(index == d.wrappedValue.buckets.count - 1)
                        Spacer()
                        Button("Remove", role: .destructive) { d.wrappedValue.buckets.removeAll { $0.id == bucket.id } }
                            .hoverHelp("Delete this bucket from the routine")
                    }
                    .controlSize(.small)
                } label: {
                    HStack {
                        Circle().fill(RoutineEditorColors.color(bucket.color)).frame(width: 10, height: 10)
                        Text(bucket.labelName).font(TypeRole.code)
                        Text(bucket.title).foregroundStyle(.secondary)
                        Spacer()
                        Text(bucket.cadence.capitalized).font(TypeRole.caption).foregroundStyle(.secondary)
                    }
                }
            }
            Button("Add Bucket") {
                let bucket = RoutineDefinition.Bucket.new(order: d.wrappedValue.buckets.count + 1)
                d.wrappedValue.buckets.append(bucket)
                expanded.insert(bucket.id)
            }
            .hoverHelp("Add another label the routine can file mail into")
        } header: {
            Text("Buckets")
        } footer: {
            Text("Each thread gets at most one bucket's label, in this order, under “\(d.wrappedValue.parentLabel)”.")
                .foregroundStyle(.secondary)
        }
    }

    private func reportAndAdvanced(_ d: Binding<RoutineDefinition>) -> some View {
        Section("Report and prompt") {
            Toggle("Count what went where", isOn: d.report.counts)
                .hoverHelp("Include how many threads went to each bucket in the report")
            Stepper("At most \(d.wrappedValue.report.maxLines) lines", value: d.report.maxLines, in: 5...50)
            Toggle("Leave unmatched automated mail and list it", isOn: Binding(
                get: { d.wrappedValue.unmatched.kind == "leave_and_report" },
                set: { d.wrappedValue.unmatched = $0 ? .init(kind: "leave_and_report", label: nil) : .init(kind: "apply_label", label: "Other") }))
                .hoverHelp("Keep automated mail that fits no bucket in the Inbox and list it in the report, instead of labelling it Other")
            Button(d.wrappedValue.advancedPrompt == nil ? "Edit Prompt…" : "Edit Hand-Written Prompt…") { editingPrompt = true }
                .hoverHelp("See or hand-edit the prompt the agent gets")
        }
    }

    private func previewSection(_ rows: [RoutinePreviewRow]) -> some View {
        Section("Preview — nothing was changed") {
            if rows.isEmpty {
                Text("No threads to sort right now.").foregroundStyle(.secondary)
            }
            ForEach(rows, id: \.threadId) { row in
                PreviewRowView(row: row, bucket: model.routines.draft?.buckets.first { $0.id == row.bucketId })
            }
        }
    }

    private func runsSection() -> some View {
        let store = model.routines
        return Section {
            if store.runs.isEmpty {
                Text("No runs yet.").foregroundStyle(.secondary)
            }
            ForEach(store.runs, id: \.runId) { run in
                RunRow(run: run)
            }
        } header: {
            HStack {
                Text("Recent runs")
                Spacer()
                if store.runner == .claudeCloud, store.draft?.cloud.triggerId != nil {
                    Button("Check Claude") { Task { await store.refreshCloudRuns() } }.controlSize(.small)
                        .hoverHelp("Fetch this routine's latest runs from claude.ai")
                }
            }
        }
    }

    private func actionBar() -> some View {
        let store = model.routines
        return VStack(spacing: Space.s) {
            if let error = store.error {
                Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Tone.failure).font(TypeRole.meta)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if let message = store.message {
                Label(message, systemImage: "checkmark.circle").foregroundStyle(.secondary).font(TypeRole.meta)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                Button("Delete", role: .destructive) { confirmingDelete = true }
                    .hoverHelp("Delete this routine; mail it filed keeps its labels")
                    .confirmationDialog("Delete this routine?", isPresented: $confirmingDelete) {
                        Button("Delete Routine", role: .destructive) { Task { await store.delete() } } // no-help: dialog
                    } message: {
                        Text("Its schedule and history go. Mail it filed keeps its labels. This cannot be undone.")
                    }
                Spacer()
                if let busy = store.busy {
                    ProgressView().controlSize(.small)
                    Text(busy).foregroundStyle(.secondary)
                }
                Button("Preview") { Task { await store.startPreview() } }
                    .hoverHelp("Classify the current mail without changing anything (runs here, with your agent)")
                Button("Run Now") { Task { await store.runNow(model: model) } }
                    .hoverHelp("Run the routine once now")
                    .disabled(store.runner == .chatGptCloud || store.runner == .claudeDesktop
                        || (store.runner == .claudeCloud && store.draft?.cloud.triggerId == nil))
                Button(publishTitle(store.runner)) { Task { await store.publish() } }
                    .disabled(store.runner == .local || model.isArchive)
                    .hoverHelp(model.isArchive ? "An imported mailbox lives only on this Mac; run the routine locally." : "")
                Button("Revert") { store.revert() }.disabled(!store.hasUnsavedChanges)
                    .hoverHelp("Throw away unsaved changes")
                Button("Save") { Task { await store.save() } }
                    .hoverHelp("Save the routine (⌘S)")
                    .keyboardShortcut("s")
                    .buttonStyle(.borderedProminent)
                    .disabled(!store.hasUnsavedChanges)
            }
        }
        .padding(Space.l)
        .background(.bar)
    }

    private func publishTitle(_ runner: RoutineRunner) -> String {
        switch runner {
        case .claudeCloud: "Publish to Claude"
        case .chatGptCloud: "Copy for ChatGPT…"
        case .claudeDesktop: "Copy for Claude Desktop…"
        case .local: "Publish"
        }
    }
}

private struct BucketEditor: View {
    @Binding var bucket: RoutineDefinition.Bucket
    let others: [RoutineDefinition.Bucket]

    var body: some View {
        TextField("Label", text: $bucket.labelName).font(TypeRole.code)
        TextField("Title", text: $bucket.title)
        Picker("Color", selection: $bucket.color) {
            ForEach(RoutineDefinition.Bucket.colors, id: \.self) { name in
                Label(name.capitalized, systemImage: "circle.fill").tag(name)
            }
        }
        .hoverHelp("The label's colour in Gmail and Kaluta")
        Picker("Review", selection: $bucket.cadence) {
            ForEach(RoutineDefinition.Bucket.cadences, id: \.self) { Text($0.capitalized).tag($0) }
        }
        .hoverHelp("How often you plan to look at this bucket; the agent uses it to judge urgency")
        VStack(alignment: .leading) {
            Text("What belongs here").font(TypeRole.caption).foregroundStyle(.secondary)
            TextEditor(text: $bucket.description).frame(minHeight: 60).font(TypeRole.body)
        }
        LinesField(title: "Examples, one per line", lines: $bucket.positiveExamples)
        LinesField(title: "Not this (e.g. “Failed payments belong in 1-Daily.”), one per line", lines: $bucket.negativeExamples)
        TextField("When it's ambiguous", text: Binding(get: { bucket.priorityWhenAmbiguous ?? "" },
                                                       set: { bucket.priorityWhenAmbiguous = $0.isEmpty ? nil : $0 }))
        Toggle("List each thread in the report", isOn: $bucket.listIndividuallyInReport)
            .hoverHelp("Name every thread filed here in the report, not just the count")
    }
}

/// A list of strings edited as lines of text.
private struct LinesField: View {
    let title: String
    @Binding var lines: [String]

    var body: some View {
        VStack(alignment: .leading) {
            Text(title).font(TypeRole.caption).foregroundStyle(.secondary)
            TextEditor(text: Binding(
                get: { lines.joined(separator: "\n") },
                set: { lines = $0.split(separator: "\n", omittingEmptySubsequences: false).map(String.init).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty } }))
                .frame(minHeight: 44)
        }
    }
}

private struct PreviewRowView: View {
    let row: RoutinePreviewRow
    let bucket: RoutineDefinition.Bucket?
    @Environment(AppModel.self) private var model
    @State private var subject: String?

    var body: some View {
        HStack {
            VStack(alignment: .leading) {
                Text(subject ?? row.threadId).lineLimit(1)
                Text(row.reason).font(TypeRole.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            Spacer()
            if let bucket {
                Text(bucket.labelName).font(TypeRole.codeCaption)
                    .padding(.horizontal, Space.s).padding(.vertical, Space.hair)
                    .background(RoutineEditorColors.color(bucket.color).opacity(0.2), in: .capsule)
            } else {
                Text("Leave in inbox").font(TypeRole.caption).foregroundStyle(.secondary)
            }
        }
        .task { subject = try? await model.core?.thread(row.threadId)?.thread.subject }
        .onTapGesture { model.selectedThreadID = row.threadId }
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { model.selectedThreadID = row.threadId }
    }
}

enum RoutineEditorColors {
    static func color(_ name: String) -> Color {
        switch name {
        case "red": .red
        case "orange": .orange
        case "yellow": .yellow
        case "green": .green
        case "teal": .teal
        case "blue": .blue
        case "purple": .purple
        case "pink": .pink
        case "brown": .brown
        default: .gray
        }
    }
}

private struct RunRow: View {
    let run: RoutineRunInfo
    @Environment(AppModel.self) private var model
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            if let report = run.reportText, !report.isEmpty {
                // Plain text: a cloud log can quote mail; it is shown, never acted on.
                Text(report).font(TypeRole.meta.monospaced()).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text("No report.").foregroundStyle(.secondary)
            }
            if run.threadCount > 0, run.status != "undone" {
                Button("Undo This Run") { Task { await model.routines.undo(run) } }
                    .hoverHelp("Move these threads back to the inbox and remove the labels (threads changed since are left alone)")
            }
        } label: {
            HStack {
                Text(Date(timeIntervalSince1970: TimeInterval(run.startedAt) / 1000), format: .dateTime.month().day().hour().minute())
                Text(run.status.capitalized).foregroundStyle(run.status == "failed" ? .red : .secondary)
                if run.inferred { Text("(seen in Gmail)").foregroundStyle(.secondary) }
                Spacer()
                if run.threadCount > 0 { Text("\(run.threadCount) threads").foregroundStyle(.secondary) }
            }
            .font(TypeRole.meta)
        }
    }
}

/// Advanced › Edit prompt (spec §11.5): editing freezes generation until
/// reset; removing the safety lines shows a warning.
private struct PromptEditor: View {
    @Binding var draft: RoutineDefinition
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""

    var body: some View {
        Dialog(title: "Prompt",
               message: "This is what the agent is told. Editing it by hand stops the settings from changing it until you reset.",
               width: nil) {
            TextEditor(text: $text).font(TypeRole.code).frame(minWidth: 640, minHeight: 420)
            let missing = PromptSafety.missing(text)
            if !missing.isEmpty {
                Label("This prompt no longer says: \(missing.joined(separator: ", ")).", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Tone.caution)
            }
        } leading: {
            Button("Reset to Generated") {
                draft.advancedPrompt = nil
                dismiss()
            }
            .hoverHelp("Go back to the prompt built from the settings")
            .disabled(draft.advancedPrompt == nil)
        } buttons: {
            CancelButton(help: "Close without changing the prompt (Esc)") { dismiss() }
            Button("Use This Prompt") {
                draft.advancedPrompt = text
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
            .hoverHelp("Use your edited prompt; the settings will no longer change it (Return)")
        }
        .task {
            if let custom = draft.advancedPrompt {
                text = custom
            } else if let id = model.routines.selectedID {
                text = (try? await model.core?.routinePrompt(id)) ?? ""
            }
        }
    }
}

enum PromptSafety {
    /// Mirrors the core's check (spec §11.8).
    static func missing(_ prompt: String) -> [String] {
        let lower = prompt.lowercased()
        var out: [String] = []
        if !(lower.contains("never trash") || lower.contains("never delete")) { out.append("never trash or delete") }
        if !lower.contains("spam") { out.append("never mark as spam") }
        if !lower.contains("never send") { out.append("never send") }
        return out
    }
}

/// The paste hand-off (spec §11.5).
private struct HandoffSheet: View {
    let handoff: RoutineHandoff
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var pasted = ""

    private var isChatGPT: Bool { handoff.url.contains("chatgpt") }

    var body: some View {
        Dialog(title: isChatGPT ? "Set up in ChatGPT" : "Set up at claude.ai", width: Self.width) {
            if isChatGPT {
                Text("1. Copy the prompt and open ChatGPT.\n2. Start a new chat, paste the prompt and ask ChatGPT to “create a scheduled task” with it, \(handoff.scheduleText.lowercased()).\n3. Make sure the Gmail app is connected in ChatGPT.")
            } else {
                Text("1. Copy the prompt and open your routines at claude.ai.\n2. Create a routine, paste the prompt, add the Gmail connector, and set the schedule to \(handoff.cronUtc.map { "the cron `\($0)` (UTC)" } ?? handoff.scheduleText).\n3. Paste the new routine's link below so Kaluta can show its runs.")
            }
            ScrollView {
                Text(handoff.prompt).font(TypeRole.codeCaption).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: 220)
            .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: Radius.control))
            if !isChatGPT {
                TextField("https://claude.ai/code/routines/trig_…", text: $pasted)
            }
        } buttons: {
            CancelButton(title: "Close", help: "Close; you can set it up later (Esc)") { dismiss() }
            if !isChatGPT {
                Button("Link Routine") { Task { await model.routines.attach(url: pasted) } }
                    .hoverHelp("Connect the routine you created at claude.ai so Kaluta shows its runs")
                    .disabled(pasted.isEmpty)
            }
            Button("Copy Prompt and Open") { model.routines.copyAndOpen(handoff) }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Copy the routine's prompt and open the site to paste it (Return)")
        }
    }

    private static let width: CGFloat = 620
}

extension RoutineHandoff: @retroactive Identifiable {
    public var id: String { url + prompt.prefix(32) }
}

/// Offered with the local runner (spec §11.7).
private struct LaunchAtLoginToggle: View {
    @State private var enabled = SMAppService.mainApp.status == .enabled
    @State private var error: String?

    var body: some View {
        Toggle("Open Kaluta at login", isOn: Binding(get: { enabled }, set: { on in
            do {
                if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
                enabled = on
                error = nil
            } catch {
                self.error = error.localizedDescription
            }
        }))
        .hoverHelp("Start Kaluta when you log in, so routines on this Mac run on time")
        if let error {
            Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure)
        }
    }
}
