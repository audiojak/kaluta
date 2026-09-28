import SwiftUI

struct SettingsView: View {
    var body: some View {
        TabView {
            Tab("General", systemImage: "gearshape") {
                GeneralSettings()
            }
            Tab("Accounts", systemImage: "person.crop.circle") {
                AccountSettings()
            }
            Tab("Agents", systemImage: "sparkles") {
                AgentSettings()
            }
            Tab("Permissions", systemImage: "hand.raised") {
                AgentPermissionsSettings()
            }
            Tab("Routines", systemImage: "clock.arrow.2.circlepath") {
                RoutineSettings()
            }
            Tab("Privacy", systemImage: "lock.shield") {
                PrivacySettings()
            }
        }
        .frame(width: 640, height: 520)
    }
}

private struct GeneralSettings: View {
    @AppStorage(NewMailNotifier.notifyKey, store: CoreClient.appDefaults()) private var notify = true
    @AppStorage(NewMailNotifier.badgeKey, store: CoreClient.appDefaults()) private var badge = true
    @AppStorage(Updater.betaKey, store: CoreClient.appDefaults()) private var betas = false
    @Environment(AppModel.self) private var model
    @Environment(Updater.self) private var updater

    var body: some View {
        Form {
            Section("Sending") {
                Picker("Undo send", selection: Binding(
                    get: { model.undoSendSeconds }, set: { model.undoSendSeconds = $0 })) {
                    ForEach(AppModel.undoSendChoices, id: \.self) { seconds in
                        Text(seconds == 0 ? "Off" : "\(seconds) seconds").tag(seconds)
                    }
                }
                .help("How long a sent message waits so you can take it back")
                Text("Messages wait this long before they go, so you can take one back with Undo (⌘Z). Quitting sends them at once.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("New Mail") {
                Toggle("Notify me about new mail in the Inbox", isOn: $notify)
                    .help("Show a notification when new mail arrives in the Inbox")
                Toggle("Show unread count on the Dock icon", isOn: $badge)
                    .help("Badge the Dock icon with the Inbox's unread count")
                    .onChange(of: badge) { model.updateBadge() }
            }
            Section("Updates") {
                if updater.isConfigured {
                    Toggle("Check for updates automatically", isOn: Binding(
                        get: { updater.automaticallyChecks }, set: { updater.automaticallyChecks = $0 }))
                        .help("Look for new versions of OpenAGC in the background")
                    Toggle("Include beta versions", isOn: $betas)
                        .help("Also offer beta versions when checking for updates")
                    Button("Check Now") { updater.checkForUpdates() }
                        .help("Look for a new version of OpenAGC now")
                        .disabled(!updater.canCheckForUpdates)
                } else {
                    Text("This build does not update itself. Official releases do.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
    }
}
