import Foundation
import OpenAGCCore
import os
import PDFKit

/// The app's handle on the Rust core. This is the only file that imports
/// `OpenAGCCore` (spec §14.2); everything else talks to `CoreClient`.
final class CoreClient: Sendable {
    private let core: Core

    /// Events from the core, already coalesced in Rust (spec §4.3). One
    /// consumer; stores fan out on the main actor.
    /// Core events with the account each is about (`nil`: app-wide).
    let events: AsyncStream<CoreClientEvent.Tagged>

    convenience init(dataDirectory: URL, logDirectory: URL? = nil,
                     secrets: KeychainSecretStore = CoreClient.defaultSecrets()) throws(CoreClientError) {
        try self.init(dataDirectoryPath: dataDirectory.path, logDirectoryPath: logDirectory?.path, secrets: secrets)
    }

    init(dataDirectoryPath: String, logDirectoryPath: String? = nil,
         secrets: KeychainSecretStore = CoreClient.defaultSecrets()) throws(CoreClientError) {
        let (stream, continuation) = AsyncStream.makeStream(of: CoreClientEvent.Tagged.self, bufferingPolicy: .unbounded)
        events = stream
        do {
            core = try Core(config: CoreConfig(dataDir: dataDirectoryPath, logDir: logDirectoryPath),
                            secrets: SecretBridge(secrets),
                            listener: EventBridge(continuation))
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
        core.setTextExtractor(extractor: PDFTextExtractor())
        let bundle = Bundle.main
        core.configureAgents(
            shimPath: bundle.bundleURL.appending(path: "Contents/MacOS/openagc-mcp").path,
            systemPromptPath: bundle.url(forResource: "agent-system-prompt", withExtension: "md")?.path ?? "")
        // Agents outside the app reach agent mailboxes through it while it
        // runs (spec §10.1, openagc-mcp --mailbox); tests never serve them.
        if !Self.isRunningTests {
            do { try core.serveOutsideAgents() } catch {
                Logger(subsystem: "ai.actual.openagc", category: "agent").warning("outside agents: \(String(describing: error), privacy: .public)")
            }
            // Push what changed while closed to the rules servers agent
            // mailboxes publish to (spec §10.6).
            core.resumeRulesPublishing()
        }
        // Tests never run the user's real agent CLIs (which would use their
        // account); neither do UI runs that ask for fakes.
        if UserDefaults.standard.bool(forKey: "OpenAGCFakeAgents") || Self.isRunningTests {
            core.debugUseFakeAgents()
        }
        // Agent mailboxes (spec §7.9): tests and fake-agent runs never
        // create a real account at the service; each sign-up is a real one.
        if Self.usesFakeAgentMail {
            core.debugUseFakeAgentMail(enabled: true)
        }
    }

    /// Agent mailboxes are created against an in-memory service: always in
    /// tests, and in scratch runs that ask for fakes. Never on the real data
    /// directory, where a fake mailbox would mix with real ones.
    static var usesFakeAgentMail: Bool {
        isRunningTests || (isScratchRun && (UserDefaults.standard.bool(forKey: "OpenAGCFakeAgents")
            || UserDefaults.standard.bool(forKey: "OpenAGCFakeAgentMail")))
    }

    /// The app's Keychain items, or a separate service when hosting tests
    /// or on a scratch data directory, so neither can read the user's
    /// sign-ins. Everything that touches the Keychain defaults to this.
    static func defaultSecrets() -> KeychainSecretStore {
        if isRunningTests { return KeychainSecretStore(service: testSecretsService) }
        return KeychainSecretStore(service: isScratchRun ? "ai.actual.openagc.scratch" : "ai.actual.openagc")
    }

    /// The test host's Keychain service; emptied when the host starts and
    /// quits (`AppDelegate`).
    static let testSecretsService = "ai.actual.openagc.tests"

    /// Where the app's tests put scratch data: one directory per test-host
    /// process, which scripts/test-macos.sh removes after the run (and the
    /// sweeper after an hour), so tests need not clean up one by one.
    static let testScratchRoot = FileManager.default.temporaryDirectory
        .appending(path: "openagc-apptests-\(ProcessInfo.processInfo.processIdentifier)", directoryHint: .isDirectory)

    /// A fresh scratch directory for a test.
    static func testScratch() -> URL {
        testScratchRoot.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    }

    /// The app's preferences, or a throwaway suite when the app is hosting
    /// tests or running on a scratch data directory (snapshots,
    /// automation): those must never write the real app's preferences
    /// (the test host and snapshots share its bundle id).
    /// One suite per process, shared by everything that remembers a
    /// setting, so a test run or snapshot is consistent with itself.
    /// Launch arguments (`-OpenAGC…`) are still read from `.standard`:
    /// reading the argument domain writes nothing.
    static func appDefaults() -> UserDefaults { sharedDefaults }

    nonisolated(unsafe) private static let sharedDefaults: UserDefaults = {
        guard isRunningTests || isScratchRun else { return .standard }
        return UserDefaults(suiteName: "openagc-scratch-\(UUID().uuidString)") ?? .standard
    }()

    /// Pointed at a throwaway data directory (snapshots, automation).
    static var isScratchRun: Bool {
        !(UserDefaults.standard.string(forKey: "OpenAGCDataDirectory") ?? "").isEmpty
    }

    static var isRunningTests: Bool {
        let env = ProcessInfo.processInfo.environment
        return env["XCTestConfigurationFilePath"] != nil || env["XCTestBundlePath"] != nil
            || env["XCTestSessionIdentifier"] != nil || NSClassFromString("XCTestCase") != nil
    }

    var version: String { core.version() }
    /// Where this core keeps its accounts.
    var dataDirectory: String { core.dataDir() }

    func ping(_ message: String) -> String {
        core.ping(message: message)
    }

    func pingAsync(_ message: String) async throws(CoreClientError) -> String {
        try await call { try await core.pingAsync(message: message) }
    }

    // MARK: Account and mail reads (SQLite only; never the network)

    func openAccount(_ accountID: String) async throws(CoreClientError) {
        try await call { try await core.openAccount(accountId: accountID) }
    }

    /// The user's accounts in their order (spec §7.7).
    func accounts() async throws(CoreClientError) -> [AccountSummary] {
        try await call { try await core.listAccounts() }
    }

    func setCurrentAccount(_ accountID: String) async throws(CoreClientError) {
        try await call { try await core.setCurrentAccount(accountId: accountID) }
    }

    func removeAccount(_ accountID: String) async throws(CoreClientError) {
        try await call { try await core.removeAccount(accountId: accountID) }
    }

    // MARK: Archive accounts (spec §7.8)

    func scanMailbox(_ path: String) async throws(CoreClientError) -> MailboxScan {
        try await call { try await core.scanMailbox(path: path) }
    }

    /// Start importing; returns the archive account's id. Progress arrives
    /// as `importProgress` events tagged with it.
    func startImport(path: String, name: String, myAddresses: [String], into accountID: String? = nil) async throws(CoreClientError) -> String {
        try await call { try await core.startImport(path: path, name: name, myAddresses: myAddresses, accountId: accountID) }
    }

    func reimportArchive(_ accountID: String) async throws(CoreClientError) -> String {
        try await call { try await core.reimportArchive(accountId: accountID) }
    }

    func cancelImport(_ accountID: String) { core.cancelImport(accountId: accountID) }

    /// Ask Gmail for `query` and download missing matches; returns how many
    /// arrived (0 for accounts without a server).
    func searchServer(_ query: String, limit: UInt32) async throws(CoreClientError) -> UInt32 {
        try await call { try await core.searchServer(query: query, limit: limit) }
    }

    /// Some mail is stored with headers only (tiered download).
    func hasHeaderOnlyMail() async -> Bool {
        (try? await call { try await core.hasHeaderOnlyMail() }) ?? false
    }

    /// Download header-only bodies now (tiered download, spec §7.4); on
    /// failure the core queues them first instead.
    func ensureBodies(_ ids: [String]) async {
        _ = try? await call { try await core.ensureBodies(messageIds: ids) }
    }

    /// Download these messages' bodies next (opened with headers only).
    func prioritizeMessages(_ ids: [String]) async {
        try? await call { try await core.prioritizeMessages(messageIds: ids) }
    }

    /// Re-download an account's mail so labels and bodies match Gmail.
    func refreshFromServer(_ accountID: String) async throws(CoreClientError) -> UInt64 {
        try await call { try await core.refreshFromServer(accountId: accountID) }
    }

    /// How an account's backfill fetches bodies ("rest", "imap", …).
    func backfillStatus(_ accountID: String) async -> BackfillStatus {
        await core.backfillStatus(accountId: accountID)
    }

    /// Which transport serves each sync job and why, the breaker, recent
    /// operations (the Sync Debugger, the sync footer).
    func syncDiagnostics(_ accountID: String) async -> SyncDiagnostics {
        await core.syncDiagnostics(accountId: accountID)
    }

    /// Time each sync job over IMAP and over the API (the Sync Debugger).
    /// Reads only; changes no mail.
    func compareTransports(_ accountID: String) async throws(CoreClientError) -> [TransportComparison] {
        try await call { try await core.compareTransports(accountId: accountID) }
    }

    func isArchive(_ accountID: String) -> Bool { core.accountIsArchive(accountId: accountID) }

    // MARK: Agent mailboxes (spec §7.9)

    /// Create a mailbox for an agent. Accepts the service's terms: only
    /// from the user's Agree and Create.
    /// `requestID` is the sheet's own: a retry returns the same mailbox.
    /// `humanEmail` is the user's email: AgentMail needs it, Primitive
    /// ignores it.
    func createAgentMailbox(service: AgentService, name: String, humanEmail: String? = nil,
                            requestID: String) async throws(CoreClientError) -> AgentMailboxCreated {
        try await call {
            try await core.createAgentMailbox(service: service, name: name, humanEmail: humanEmail,
                                              requestId: requestID)
        }
    }

    func agentMailboxPlan(_ accountID: String) async throws(CoreClientError) -> AgentMailboxPlan {
        try await call { try await core.agentMailboxPlan(accountId: accountID) }
    }

    func startAgentMailboxVerification(_ accountID: String, email: String) async throws(CoreClientError) -> AgentVerification {
        try await call { try await core.startAgentMailboxVerification(accountId: accountID, email: email) }
    }

    func verifyAgentMailbox(_ accountID: String, code: String) async throws(CoreClientError) -> AgentMailboxPlan {
        try await call { try await core.verifyAgentMailbox(accountId: accountID, code: code) }
    }

    /// The verification code, once it is in the user's account `inAccount`.
    func findAgentMailboxCode(_ accountID: String, in inAccount: String) async -> String? {
        (try? await call { try await core.findAgentMailboxCode(accountId: accountID, inAccountId: inAccount) }) ?? nil
    }

    func agentMailboxAPIKey(_ accountID: String) throws(CoreClientError) -> String {
        try callSync { try core.agentMailboxApiKey(accountId: accountID) }
    }

    func agentServiceTermsURL(_ service: AgentService) -> URL? {
        URL(string: core.agentServiceTermsUrl(service: service))
    }

    func agentServiceDashboardURL(_ service: AgentService) -> URL? {
        URL(string: core.agentServiceDashboardUrl(service: service))
    }

    func isAgent(_ accountID: String) -> Bool { core.accountIsAgent(accountId: accountID) }

    /// Where an agent mailbox may send now (kind: any_recipient,
    /// managed_zone, your_domain, address).
    func agentSendRules(_ accountID: String) async throws(CoreClientError) -> [AgentSendRule] {
        try await call { try await core.agentSendRules(accountId: accountID) }
    }

    /// The user's own domains on an agent mailbox's account.
    func agentDomains(_ accountID: String) async throws(CoreClientError) -> [AgentDomain] {
        try await call { try await core.agentDomains(accountId: accountID) }
    }

    func addAgentDomain(_ accountID: String, domain: String) async throws(CoreClientError) -> AgentDomain {
        try await call { try await core.addAgentDomain(accountId: accountID, domain: domain) }
    }

    func checkAgentDomain(_ accountID: String, domainID: String) async throws(CoreClientError) -> AgentDomain {
        try await call { try await core.checkAgentDomain(accountId: accountID, domainId: domainID) }
    }

    func agentDomainZoneFile(_ accountID: String, domainID: String) async throws(CoreClientError) -> String {
        try await call { try await core.agentDomainZoneFile(accountId: accountID, domainId: domainID) }
    }

    /// Send and receive as `address` (the service's own, or on a verified domain).
    func setAgentAddress(_ accountID: String, _ address: String) async throws(CoreClientError) {
        try await call { try await core.setAgentAddress(accountId: accountID, address: address) }
    }

    /// Whether agents send from this mailbox without asking.
    func agentSendMode(_ accountID: String) -> AgentSendMode? {
        try? core.agentSendMode(accountId: accountID)
    }

    func setAgentSendMode(_ accountID: String, _ mode: AgentSendMode) throws(CoreClientError) {
        try callSync { try core.setAgentSendMode(accountId: accountID, mode: mode) }
    }

    /// What Connect an Agent… would write for this mailbox, and where
    /// (spec §10.1). Writes nothing.
    func agentConnection(_ accountID: String, client: AgentClient) throws(CoreClientError) -> AgentConnection {
        try callSync { try core.agentConnection(accountId: accountID, client: client) }
    }

    /// Write it, backing the file up first; the backup's path, if there was a file.
    func connectAgent(_ accountID: String, client: AgentClient) throws(CoreClientError) -> String? {
        try callSync { try core.connectAgent(accountId: accountID, client: client) }
    }

    // MARK: Rules server (spec §10.6)

    /// Exactly what publishing this agent mailbox sends now.
    func rulesPreview(_ accountID: String) async throws(CoreClientError) -> RulesPreview {
        try await call { try await core.rulesPreview(accountId: accountID) }
    }

    /// Register on the server (the first time) and publish now and on every change.
    func rulesPublishStart(_ accountID: String, serverURL: String,
                           registrationToken: String?) async throws(CoreClientError) -> RulesPublication {
        try await call {
            try await core.rulesPublishStart(accountId: accountID, serverUrl: serverURL,
                                             registrationToken: registrationToken)
        }
    }

    /// Stop publishing; with `removeFromServer`, the server forgets the mailbox.
    func rulesPublishStop(_ accountID: String, removeFromServer: Bool) async throws(CoreClientError) {
        try await call { try await core.rulesPublishStop(accountId: accountID, removeFromServer: removeFromServer) }
    }

    func rulesPublishStatus(_ accountID: String) -> RulesPublication? {
        core.rulesPublishStatus(accountId: accountID)
    }

    /// Push a new version now.
    func rulesPublishNow(_ accountID: String) async throws(CoreClientError) -> RulesPublication {
        try await call { try await core.rulesPublishNow(accountId: accountID) }
    }

    /// How agents reach the mailbox's rules server, and whether it signs
    /// claude.ai connectors in (Connect a Cloud Agent…).
    func rulesConnectInfo(_ accountID: String) async throws(CoreClientError) -> RulesConnectInfo {
        try await call { try await core.rulesConnectInfo(accountId: accountID) }
    }

    /// A one-time connect code for a claude.ai connector or cloud routine;
    /// shown once, never kept.
    func rulesConnectCodeMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesConnectCode {
        try await call { try await core.rulesConnectCodeMint(accountId: accountID, name: name) }
    }

    /// A static agent token for Claude Code, the Agent SDK or a script;
    /// shown once, never kept.
    func rulesAgentTokenMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesAgentToken {
        try await call { try await core.rulesAgentTokenMint(accountId: accountID, name: name) }
    }

    /// The agents connected to the mailbox on its rules server, revoked ones included.
    func rulesAgents(_ accountID: String) async throws(CoreClientError) -> [RulesAgent] {
        try await call { try await core.rulesAgents(accountId: accountID) }
    }

    /// Revoke an agent; a connector's sessions end at its next request.
    func rulesAgentRevoke(_ accountID: String, agentID: String) async throws(CoreClientError) {
        try await call { try await core.rulesAgentRevoke(accountId: accountID, agentId: agentID) }
    }

    /// Snapshots: a publishing status as if pushed; no server is contacted.
    func debugSetRulesPublication(_ accountID: String, serverURL: String, version: Int64, publishedAt: Int64,
                                  error: String?) throws(CoreClientError) {
        try callSync {
            try core.debugSetRulesPublication(accountId: accountID, serverUrl: serverURL, version: version,
                                              publishedAt: publishedAt, error: error)
        }
    }

    // MARK: Service accounts (spec §7.9, ADR 0015)

    /// The service accounts with their agents' account ids.
    func listServiceAccounts() async throws(CoreClientError) -> [ServiceAccountSummary] {
        try await call { try await core.listServiceAccounts() }
    }

    /// Add an agent to a service account: no sign-up, terms or code.
    func addAgent(toServiceAccount serviceAccountID: String, name: String, domain: String? = nil,
                  requestID: String = UUID().uuidString) async throws(CoreClientError) -> AgentAdded {
        try await call {
            try await core.addAgent(serviceAccountId: serviceAccountID, name: name, domain: domain,
                                    requestId: requestID)
        }
    }

    /// The service account an agent mailbox belongs to.
    func agentServiceAccount(_ accountID: String) -> String? {
        try? core.agentServiceAccount(accountId: accountID)
    }

    /// The service account's plan now, from the service.
    func serviceAccountPlan(_ serviceAccountID: String) async throws(CoreClientError) -> AgentMailboxPlan {
        try await call { try await core.serviceAccountPlan(serviceAccountId: serviceAccountID) }
    }

    /// What the service account's agents may do, in words (the agents'
    /// prompts say the same).
    func serviceAccountLimits(_ serviceAccountID: String) -> String? {
        try? core.serviceAccountLimits(serviceAccountId: serviceAccountID)
    }

    /// The same words for a service account not made yet.
    func agentServiceLimits(_ service: AgentService, verified: Bool, humanEmail: String?) -> String {
        core.agentServiceLimits(service: service, verified: verified, humanEmail: humanEmail)
    }

    /// The service account's key: it reaches every agent in it.
    func serviceAccountAPIKey(_ serviceAccountID: String) throws(CoreClientError) -> String {
        try callSync { try core.serviceAccountApiKey(serviceAccountId: serviceAccountID) }
    }

    /// AgentMail, once verified: a new key for this agent's inbox only.
    func agentInboxAPIKey(_ accountID: String) async throws(CoreClientError) -> String {
        try await call { try await core.agentInboxApiKey(accountId: accountID) }
    }

    /// Where the service account's agents may send now.
    func serviceAccountSendRules(_ serviceAccountID: String) async throws(CoreClientError) -> [AgentSendRule] {
        try await call { try await core.serviceAccountSendRules(serviceAccountId: serviceAccountID) }
    }

    /// The user's own domains on the service account.
    func serviceAccountDomains(_ serviceAccountID: String) async throws(CoreClientError) -> [AgentDomain] {
        try await call { try await core.serviceAccountDomains(serviceAccountId: serviceAccountID) }
    }

    /// Tests: deliver a message into a fake agent mailbox.
    func deliverToAgentMailbox(_ accountID: String, from: String, subject: String, body: String) throws(CoreClientError) {
        try callSync { try core.debugDeliverToAgentMailbox(accountId: accountID, from: from, subject: subject, body: body) }
    }

    /// Development/test hook: a listed account with a synthetic mailbox and
    /// no sign-in.
    func addDemoAccount(_ accountID: String, email: String, name: String? = nil, threads: UInt32 = 60) async throws(CoreClientError) {
        try await call {
            try await core.debugAddDemoAccount(accountId: accountID, email: email, displayName: name, threads: threads)
        }
    }

    /// Account stores no listed account owns (spec §7.7).
    func orphanedStores() async throws(CoreClientError) -> [OrphanedStore] {
        try await call { try await core.orphanedStores() }
    }

    func removeOrphanedStore(_ id: String) async throws(CoreClientError) {
        try await call { try await core.removeOrphanedStore(accountId: id) }
    }

    /// Rename an account; for Gmail an empty name goes back to the profile's.
    func renameAccount(_ accountID: String, to name: String) async throws(CoreClientError) {
        try await call { try await core.renameAccount(accountId: accountID, name: name) }
    }

    func moveAccount(_ accountID: String, to position: Int) async throws(CoreClientError) {
        try await call { try await core.moveAccount(accountId: accountID, position: UInt32(max(0, position))) }
    }

    var currentAccountID: String? { core.currentAccountId() }

    func mailboxes() async throws(CoreClientError) -> [MailboxInfo] {
        try await call { try await core.listMailboxes() }
    }

    func labels() async throws(CoreClientError) -> [LabelInfo] {
        try await call { try await core.listLabels() }
    }

    /// Create a label; a `/` path creates missing parents.
    func createLabel(_ path: String, color: String? = nil) async throws(CoreClientError) -> LabelInfo {
        try await call { try await core.createLabel(name: path, color: color) }
    }

    /// The Inbox's category tabs with their counts, Primary first.
    func inboxCategories(importantOnly: Bool, hiddenLabel: String? = nil) async throws(CoreClientError) -> [InboxCategory] {
        try await call { try await core.inboxCategories(importantOnly: importantOnly, hiddenLabel: hiddenLabel) }
    }

    func threads(in mailboxID: String, after cursor: String? = nil, limit: UInt32 = 100) async throws(CoreClientError) -> ThreadPage {
        try await call { try await core.listThreads(mailboxId: mailboxID, cursor: cursor, limit: limit) }
    }

    func search(_ query: String, after cursor: String? = nil, limit: UInt32 = 150) async throws(CoreClientError) -> ThreadPage {
        try await call { try await core.searchThreads(query: query, cursor: cursor, limit: limit) }
    }

    func thread(_ threadID: String) async throws(CoreClientError) -> ThreadDetail? {
        try await call { try await core.getThread(threadId: threadID) }
    }

    func renderedBody(_ messageID: String) async throws(CoreClientError) -> RenderedBody? {
        try await call { try await core.getRenderedBody(messageId: messageID) }
    }

    // MARK: Mutations (applied locally at once, then pushed to Gmail)

    // Each returns a token for undo (spec §14.6a), or nil if nothing changed.

    @discardableResult
    func archive(_ threadIDs: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.archive(threadIds: threadIDs) }
    }

    @discardableResult
    func moveToInbox(_ threadIDs: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.moveToInbox(threadIds: threadIDs) }
    }

    @discardableResult
    func setRead(_ threadIDs: [String], _ read: Bool) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.setRead(threadIds: threadIDs, read: read) }
    }

    @discardableResult
    func setStarred(_ threadIDs: [String], _ starred: Bool) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.setStarred(threadIds: threadIDs, starred: starred) }
    }

    @discardableResult
    func modifyLabels(_ threadIDs: [String], add: [String], remove: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.modifyLabels(threadIds: threadIDs, add: add, remove: remove) }
    }

    @discardableResult
    func trash(_ threadIDs: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.trash(threadIds: threadIDs) }
    }

    /// Mark as Junk: to Spam, out of the Inbox (spec §14.3 amendment, junk).
    func markJunk(_ threadIDs: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.markJunk(threadIds: threadIDs) }
    }

    /// Not Junk: out of Spam, into the Inbox.
    func notJunk(_ threadIDs: [String]) async throws(CoreClientError) -> UndoToken? {
        try await call { try await core.notJunk(threadIds: threadIDs) }
    }

    /// Reverse a recorded action exactly, in its own account.
    func undo(_ token: UndoToken) async throws(CoreClientError) {
        try await call { try await core.undoAction(token: token) }
    }

    func redo(_ token: UndoToken) async throws(CoreClientError) {
        try await call { try await core.redoAction(token: token) }
    }

    func outboxStatus() async throws(CoreClientError) -> (pending: UInt32, failed: UInt32) {
        let status = try await call { try await core.outboxStatus() }
        return (status.pending, status.failed)
    }

    func clearFailedChanges() async throws(CoreClientError) {
        try await call { try await core.clearFailedChanges() }
    }

    // MARK: Attachments

    struct AttachmentFile: Sendable, Equatable {
        let url: URL
        let filename: String
        let mimeType: String
        let contentID: String?
    }

    /// The local copy of an attachment, downloaded first if needed. New
    /// downloads are quarantined like any browser download (spec §15.3), so
    /// Gatekeeper checks them before anything opens.
    func attachmentFile(_ attachmentID: String) async throws(CoreClientError) -> AttachmentFile {
        let info = try await call { try await core.attachmentFile(attachmentId: attachmentID) }
        let url = URL(filePath: info.path)
        if info.downloaded { Self.quarantine(url) }
        return AttachmentFile(url: url, filename: info.filename, mimeType: info.mimeType, contentID: info.contentId)
    }

    static func quarantine(_ url: URL) {
        var values = URLResourceValues()
        values.quarantineProperties = [
            kLSQuarantineAgentNameKey as String: "OpenAGC",
            kLSQuarantineTypeKey as String: kLSQuarantineTypeOtherAttachment as String,
        ]
        var url = url
        do {
            try url.setResourceValues(values)
        } catch {
            Logger(subsystem: "ai.actual.openagc", category: "attachments")
                .error("quarantine failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    // MARK: Drafts and sending

    func accountAddress() async throws(CoreClientError) -> String {
        try await call { try await core.accountAddress() }
    }

    /// The open account's own addresses (aliases too), lowercased.
    func ownAddresses() async -> Set<String> {
        Set((try? await call { try await core.ownAddresses() }) ?? [])
    }

    func replyDraft(to messageID: String, all: Bool) async throws(CoreClientError) -> DraftInfo {
        try await call { try await core.replyDraft(messageId: messageID, replyAll: all) }
    }

    func forwardDraft(of messageID: String) async throws(CoreClientError) -> DraftInfo {
        try await call { try await core.forwardDraft(messageId: messageID) }
    }

    /// Insert or update a draft; returns its id.
    func saveDraft(_ draft: DraftInfo) async throws(CoreClientError) -> Int64 {
        try await call { try await core.saveDraft(draft: draft) }
    }

    func draft(_ id: Int64) async throws(CoreClientError) -> DraftInfo? {
        try await call { try await core.getDraft(id: id) }
    }

    func drafts() async throws(CoreClientError) -> [DraftInfo] {
        try await call { try await core.listDrafts() }
    }

    func deleteDraft(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.deleteDraft(id: id) }
    }

    /// Returns whether the send is held for Undo Send.
    @discardableResult
    func sendDraft(_ id: Int64) async throws(CoreClientError) -> Bool {
        try await call { try await core.sendDraft(id: id) }
    }

    /// Undo Send: take back a held send in `accountID`; false if it went.
    func cancelSend(_ draftID: Int64, in accountID: String) async -> Bool {
        let composer = composer(for: accountID)
        return (try? await CoreClient.bridge { try await composer.cancelSend(draftId: draftID) }) ?? false
    }

    /// The local draft that edits a draft from the Drafts mailbox (made on
    /// first open, with its attachments).
    func openDraft(_ messageID: String, in accountID: String) async throws(CoreClientError) -> DraftInfo {
        let composer = composer(for: accountID)
        return try await CoreClient.bridge { try await composer.openDraft(messageId: messageID) }
    }

    /// When the draft's send stops being held, if it is still held.
    func sendHeldUntil(_ draftID: Int64, in accountID: String) async -> Date? {
        let composer = composer(for: accountID)
        let until = try? await CoreClient.bridge { try await composer.sendHeldUntil(draftId: draftID) }
        return until.flatMap { $0 }.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) }
    }

    /// How long sends wait so they can be undone (0 = off).
    func setSendDelay(seconds: UInt32) { core.setSendDelay(seconds: seconds) }

    /// Quitting: send every held message now; waits up to `timeout` and
    /// returns how many are still going.
    func sendHeldNow(timeout: Duration) async -> UInt32 {
        await core.sendHeldNow(timeoutMs: UInt32(timeout.components.seconds * 1000))
    }

    func heldSendCount() async -> UInt32 { await core.heldSendCount() }

    /// Sends quitting should wait for (held, due or on their way),
    /// answered at once: the quit handler cannot wait to ask.
    func unsentSendCountNow() -> UInt32 { core.unsentSendCountNow() }

    /// Mirror edited drafts to Gmail now instead of at the next 30 s tick.
    func flushDrafts() { core.flushDrafts() }

    /// Synchronous, for the recipient token field's completion callback.
    func suggestContactsNow(_ text: String, limit: UInt32 = 8) -> [AddressInfo] {
        core.suggestContactsNow(text: text, limit: limit)
    }

    // MARK: Tasks (spec §14.8)

    func listTasks(includeDone: Bool = false) async throws(CoreClientError) -> [TaskItem] {
        try await call { try await core.listTasks(includeDone: includeDone) }
    }

    /// Adds the tasks and labels their threads `Task`.
    func createTasks(_ tasks: [NewTask]) async throws(CoreClientError) -> [TaskItem] {
        try await call { try await core.createTasks(new: tasks) }
    }

    func updateTask(_ id: Int64, _ edit: TaskEdit) async throws(CoreClientError) -> TaskItem {
        try await call { try await core.updateTask(id: id, edit: edit) }
    }

    func setTaskDone(_ id: Int64, _ done: Bool) async throws(CoreClientError) -> TaskItem {
        try await call { try await core.setTaskDone(id: id, done: done) }
    }

    /// Returns the task as it was, for `restoreTask` (undo).
    func deleteTask(_ id: Int64) async throws(CoreClientError) -> TaskItem {
        try await call { try await core.deleteTask(id: id) }
    }

    func restoreTask(_ task: TaskItem) async throws(CoreClientError) -> TaskItem {
        try await call { try await core.restoreTask(task: task) }
    }

    func threadsWithOpenTasks(_ threadIDs: [String]) async throws(CoreClientError) -> [String] {
        try await call { try await core.threadsWithOpenTasks(threadIds: threadIDs) }
    }

    func taskLabelID() async throws(CoreClientError) -> String? {
        try await call { try await core.taskLabelId() }
    }

    func taskCategories() async throws(CoreClientError) -> [String] {
        try await call { try await core.taskCategories() }
    }

    func setTaskCategories(_ names: [String]) async throws(CoreClientError) -> [String] {
        try await call { try await core.setTaskCategories(names: names) }
    }

    func resetTaskCategories() async throws(CoreClientError) -> [String] {
        try await call { try await core.resetTaskCategories() }
    }

    /// Claude's request for task suggestions about these threads, with
    /// `today` as `YYYY-MM-DD` in the user's calendar.
    func taskPrompt(_ threadIDs: [String], today: String) async throws(CoreClientError) -> String {
        try await call { try await core.taskPrompt(threadIds: threadIDs, today: today) }
    }

    func parseTaskSuggestions(_ text: String, threadIDs: [String]) async throws(CoreClientError) -> [TaskSuggestion] {
        try await call { try await core.parseTaskSuggestions(text: text, threadIds: threadIDs) }
    }

    // MARK: Writing guide (spec §14.9)

    func guideCategories() async throws(CoreClientError) -> [GuideCategoryInfo] {
        try await call { try await core.guideCategories() }
    }

    /// Entries with any of `statuses`, or every entry when empty.
    func guideEntries(_ statuses: [GuideStatus] = []) async throws(CoreClientError) -> [GuideEntry] {
        try await call { try await core.listGuideEntries(statuses: statuses) }
    }

    func guideEntry(_ id: Int64) async throws(CoreClientError) -> GuideEntry? {
        try await call { try await core.guideEntry(id: id) }
    }

    /// Edits applied as one change; its id undoes and redoes it.
    func applyGuideEdits(_ edits: [GuideEdit], reason: String) async throws(CoreClientError) -> GuideChange {
        try await call { try await core.applyGuideEdits(edits: edits, reason: reason) }
    }

    func undoGuideChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.undoGuideChange(changeId: id) }
    }

    func redoGuideChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.redoGuideChange(changeId: id) }
    }

    func guideVersion() async throws(CoreClientError) -> Int64 {
        try await call { try await core.guideVersion() }
    }

    func audienceGroups() async throws(CoreClientError) -> [AudienceGroup] {
        try await call { try await core.listAudienceGroups() }
    }

    func saveAudienceGroup(_ group: AudienceGroup) async throws(CoreClientError) -> [AudienceGroup] {
        try await call { try await core.saveAudienceGroup(group: group) }
    }

    func renameAudienceGroup(_ id: Int64, to name: String) async throws(CoreClientError) -> [AudienceGroup] {
        try await call { try await core.renameAudienceGroup(id: id, name: name) }
    }

    /// Returns the change that re-scoped entries, if any, for Undo.
    func mergeAudienceGroups(into: Int64, from: Int64) async throws(CoreClientError) -> Int64? {
        try await call { try await core.mergeAudienceGroups(into: into, from: from) }
    }

    /// Suggest groups for the obvious gaps until there are five.
    func fillAudienceGroups() async throws(CoreClientError) -> [AudienceGroup] {
        try await call { try await core.fillAudienceGroups() }
    }

    /// The confirmed groups these recipients belong to.
    func audienceFor(_ addresses: [String]) async throws(CoreClientError) -> [String] {
        try await call { try await core.audienceFor(addresses: addresses) }
    }

    func deleteAudienceGroup(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.deleteAudienceGroup(id: id) }
    }

    /// Markdown to read or share, or JSON to import or merge elsewhere.
    func exportGuide(json: Bool, withEvidence: Bool = false) async throws(CoreClientError) -> String {
        try await call { try await core.exportGuide(json: json, withEvidence: withEvidence) }
    }

    func readGuideExport(_ json: String) throws(CoreClientError) -> GuideImport {
        do { return try core.readGuideExport(json: json) } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    /// For the learning dialog: sent mail, analysed before, and what a run
    /// of `count` would analyse.
    func guideSampleInfo(count: UInt32, filter: GuideSampleFilter) async throws(CoreClientError) -> GuideSampleInfo {
        try await call { try await core.guideSampleInfo(count: count, filter: filter) }
    }

    /// The writing guide for a message being drafted (spec §14.9).
    func guideForMessage(recipients: [String], messageType: String?,
                         audiences: [String]?) async throws(CoreClientError) -> GuideRendered {
        try await call { try await core.guideForMessage(recipients: recipients, messageType: messageType, audiences: audiences) }
    }

    /// Check an AI draft's own text against the guide for its message.
    func checkGuideDraft(_ text: String, recipients: [String], messageType: String?,
                         audiences: [String]?) async throws(CoreClientError) -> [GuideCheckFailure] {
        try await call {
            try await core.checkGuideDraft(text: text, recipients: recipients, messageType: messageType, audiences: audiences)
        }
    }

    func setDraftGuideVersion(_ draftID: Int64, _ version: Int64) async throws(CoreClientError) {
        try await call { try await core.setDraftGuideVersion(draftId: draftID, version: version) }
    }

    func draftGuideVersion(_ draftID: Int64) async throws(CoreClientError) -> Int64? {
        try await call { try await core.draftGuideVersion(draftId: draftID) }
    }

    /// Keep what writing help wrote into a draft (spec §14.10); nothing is
    /// kept before the account's first finished learning run.
    func recordWritingHelp(draftID: Int64, agent: String, instruction: String, text: String, guideVersion: Int64?,
                           audiences: [String]) async throws(CoreClientError) {
        try await call {
            try await core.recordWritingHelp(draftId: draftID, agent: agent, instruction: instruction, aiText: text,
                                             guideVersion: guideVersion, audiences: audiences)
        }
    }

    func aiCompositions(limit: UInt32 = 50) async throws(CoreClientError) -> [AiCompositionInfo] {
        try await call { try await core.aiCompositions(limit: limit) }
    }

    /// Ask the agent how to change the guide; nothing changes until the
    /// user answers the questions.
    func proposeGuideChange(_ request: String, agent: String) async throws(CoreClientError) -> [GuideChangeQuestion] {
        try await call { try await core.proposeGuideChange(request: request, agent: agent) }
    }

    /// What merging a guide from another account or a file would do.
    func planGuideMerge(fromAccount: String?, json: String?, agent: String) async throws(CoreClientError) -> GuideMergePlan {
        try await call { try await core.planGuideMerge(fromAccount: fromAccount, json: json, agent: agent) }
    }

    /// The signature block the analysis found in sent mail, if any.
    func guideSignature() async throws(CoreClientError) -> String? {
        try await call { try await core.guideSignature() }
    }

    /// How many messages a run of this request would analyse.
    func guideRunPreview(_ request: GuideRunRequest) async throws(CoreClientError) -> UInt32 {
        try await call { try await core.guideRunPreview(request: request) }
    }

    func startGuideRun(_ request: GuideRunRequest) async throws(CoreClientError) -> GuideRunInfo {
        try await call { try await core.startGuideRun(request: request) }
    }

    func pauseGuideRun() async throws(CoreClientError) {
        try await call { try await core.pauseGuideRun() }
    }

    /// Resume a paused run, or one the app quit in the middle of.
    @discardableResult
    func resumeGuideRun() async throws(CoreClientError) -> GuideRunInfo? {
        try await call { try await core.resumeGuideRun() }
    }

    func cancelGuideRun() async throws(CoreClientError) {
        try await call { try await core.cancelGuideRun() }
    }

    func guideProgress() async throws(CoreClientError) -> GuideProgress {
        try await call { try await core.guideProgress() }
    }

    // MARK: Analysis (spec §14.10)

    /// Run Now: the day's review on demand.
    func startAnalysisRun(agent: String? = nil) async throws(CoreClientError) -> AnalysisRunInfo {
        try await call { try await core.startAnalysisRun(agent: agent) }
    }

    func pauseAnalysisRun() async throws(CoreClientError) {
        try await call { try await core.pauseAnalysisRun() }
    }

    func resumeAnalysisRun() async throws(CoreClientError) -> AnalysisRunInfo? {
        try await call { try await core.resumeAnalysisRun() }
    }

    func cancelAnalysisRun() async throws(CoreClientError) {
        try await call { try await core.cancelAnalysisRun() }
    }

    func analysisProgress() async throws(CoreClientError) -> AnalysisProgress {
        try await call { try await core.analysisProgress() }
    }

    func analysisQueue() async throws(CoreClientError) -> AnalysisQueue {
        try await call { try await core.analysisQueue() }
    }

    /// The user looked at a page's proposals: its dot clears.
    func analysisSeen(_ page: ProposalPage) async throws(CoreClientError) {
        try await call { try await core.analysisSeen(page: page) }
    }

    func analysisPairs(_ proposalID: Int64) async throws(CoreClientError) -> [AnalysisPairInfo] {
        try await call { try await core.analysisPairs(proposalId: proposalID) }
    }

    /// Accept or reject proposals as one change; undone with `undoGuideChange`.
    func decideAnalysisProposals(_ ids: [Int64], accept: Bool) async throws(CoreClientError) -> GuideChange {
        try await call { try await core.decideAnalysisProposals(ids: ids, accept: accept) }
    }

    func acceptAnalysisProposal(_ id: Int64, as fields: GuideEntryFields) async throws(CoreClientError) -> GuideChange {
        try await call { try await core.acceptAnalysisProposalEdited(id: id, fields: fields) }
    }

    func ignoreAnalysisPair(_ compositionID: Int64) async throws(CoreClientError) -> GuideChange {
        try await call { try await core.ignoreAnalysisPair(compositionId: compositionID) }
    }

    /// Accept or reject fact proposals; undone with `undoFactChange`.
    /// `uses`, by proposal id, is how freely drafts may use each accepted fact.
    func decideFactProposals(_ ids: [Int64], accept: Bool,
                             uses: [Int64: FactUse] = [:]) async throws(CoreClientError) -> FactChange {
        try await call { try await core.decideFactAnalysisProposals(ids: ids, accept: accept, uses: uses) }
    }

    func analysisFactsFrom() async throws(CoreClientError) -> FactsFrom {
        try await call { try await core.analysisFactsFrom() }
    }

    func setAnalysisFactsFrom(_ from: FactsFrom) async throws(CoreClientError) {
        try await call { try await core.setAnalysisFactsFrom(from: from) }
    }

    func analysisSettings() async throws(CoreClientError) -> AnalysisSettings {
        try await call { try await core.analysisSettings() }
    }

    func setAnalysisSettings(_ settings: AnalysisSettings) async throws(CoreClientError) {
        try await call { try await core.setAnalysisSettings(settings: settings) }
    }

    /// Accounts with Analysis proposals the user has not seen (menu dots).
    func accountsWithUnseenAnalysis() async -> [String] {
        await core.accountsWithUnseenAnalysis()
    }

    func analysisMetrics() async throws(CoreClientError) -> AnalysisMetrics {
        try await call { try await core.analysisMetrics() }
    }

    // MARK: Facts (spec §14.11)

    func facts(_ statuses: [FactStatus] = [.accepted]) async throws(CoreClientError) -> [FactInfo] {
        try await call { try await core.listFacts(statuses: statuses) }
    }

    func factCategories() async throws(CoreClientError) -> [FactCategoryInfo] {
        try await call { try await core.factCategories() }
    }

    func factStarterSets() -> [StarterSetInfo] { core.factStarterSets() }

    func similarFactCategory(_ name: String) async throws(CoreClientError) -> FactCategoryInfo? {
        try await call { try await core.similarFactCategory(name: name) }
    }

    /// Change facts as one change; undone with `undoFactChange`.
    func applyFactEdits(_ edits: [FactEdit], reason: String) async throws(CoreClientError) -> FactChange {
        try await call { try await core.applyFactEdits(edits: edits, reason: reason) }
    }

    func editFactCategories(_ edits: [CategoryEdit]) async throws(CoreClientError) -> FactChange {
        try await call { try await core.editFactCategories(edits: edits) }
    }

    func addFactStarterSet(_ set: StarterSet) async throws(CoreClientError) -> FactChange {
        try await call { try await core.addFactStarterSet(set: set) }
    }

    func undoFactChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.undoFactChange(changeId: id) }
    }

    func redoFactChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.redoFactChange(changeId: id) }
    }

    /// Markdown to read or share, or JSON to merge into another account.
    func exportFacts(json: Bool) async throws(CoreClientError) -> String {
        try await call { try await core.exportFacts(json: json) }
    }

    func mergeFacts(_ json: String) async throws(CoreClientError) -> FactMergeResult {
        try await call { try await core.mergeFacts(json: json) }
    }

    // Global facts (ADR 0012)

    func globalFacts(_ statuses: [FactStatus] = [.accepted]) async throws(CoreClientError) -> [FactInfo] {
        try await call { try await core.listGlobalFacts(statuses: statuses) }
    }

    func globalFactCategories() async throws(CoreClientError) -> [FactCategoryInfo] {
        try await call { try await core.globalFactCategories() }
    }

    func applyGlobalFactEdits(_ edits: [FactEdit], reason: String) async throws(CoreClientError) -> FactChange {
        try await call { try await core.applyGlobalFactEdits(edits: edits, reason: reason) }
    }

    func editGlobalFactCategories(_ edits: [CategoryEdit]) async throws(CoreClientError) -> FactChange {
        try await call { try await core.editGlobalFactCategories(edits: edits) }
    }

    func undoGlobalFactChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.undoGlobalFactChange(changeId: id) }
    }

    func redoGlobalFactChange(_ id: Int64) async throws(CoreClientError) {
        try await call { try await core.redoGlobalFactChange(changeId: id) }
    }

    /// Move a fact to every account, or back to this one; undone with
    /// `undoFactChange` (the account's stack).
    func makeFactGlobal(_ id: Int64) async throws(CoreClientError) -> FactChange {
        try await call { try await core.makeFactGlobal(id: id) }
    }

    func makeFactLocal(_ globalID: Int64) async throws(CoreClientError) -> FactChange {
        try await call { try await core.makeFactLocal(globalId: globalID) }
    }

    /// Snapshots: a few reviewed pairs and proposals on the demo account.
    func debugSeedAnalysis() async throws(CoreClientError) {
        try await call { try await core.debugSeedAnalysis() }
    }

    func debugSeedFactProposals() async throws(CoreClientError) {
        try await call { try await core.debugSeedFactProposals() }
    }

    func guideEntryHealth() async throws(CoreClientError) -> [GuideEntryHealth] {
        try await call { try await core.guideEntryHealth() }
    }

    /// Proposals ready to decide (from finished runs only).
    func guideDecisions() async throws(CoreClientError) -> [GuideEntry] {
        try await call { try await core.guideDecisions() }
    }

    // MARK: Agents

    func agentProviders(refresh: Bool = false) async -> [AgentProviderInfo] {
        await core.listAgentProviders(refresh: refresh)
    }

    /// Start a session; with `selection`, the agent sees only those threads.
    func startAgentSession(provider: String, selection: [String]? = nil,
                           resume: String? = nil) async throws(CoreClientError) -> String {
        try await call { try await core.startAgentSession(provider: provider, selection: selection, resume: resume) }
    }

    /// A session whose tools may only read: writing help and task
    /// suggestions, which must not change mail whatever the agent is told.
    func startReadOnlyAgentSession(provider: String, selection: [String]) async throws(CoreClientError) -> String {
        try await call { try await core.startReadOnlyAgentSession(provider: provider, selection: selection) }
    }

    func sendAgentPrompt(_ sessionID: String, _ prompt: String,
                         context: PromptContextInfo = .empty) async throws(CoreClientError) {
        try await call { try await core.sendAgentPrompt(sessionId: sessionID, prompt: prompt, context: context) }
    }

    func cancelAgentTurn(_ sessionID: String) async throws(CoreClientError) {
        try await call { try await core.cancelAgentTurn(sessionId: sessionID) }
    }

    func closeAgentSession(_ sessionID: String) async throws(CoreClientError) {
        try await call { try await core.closeAgentSession(sessionId: sessionID) }
    }

    func agentHistory(limit: UInt32 = 30) async throws(CoreClientError) -> [AgentSessionInfo] {
        try await call { try await core.listAgentHistory(limit: limit) }
    }

    func agentTranscript(_ sessionID: String) async throws(CoreClientError) -> [AgentTranscriptItem] {
        try await call { try await core.agentTranscript(sessionId: sessionID) }
    }

    /// Continue a stored conversation; returns its (unchanged) id.
    func resumeAgentSession(_ sessionID: String) async throws(CoreClientError) -> String {
        try await call { try await core.resumeAgentSession(sessionId: sessionID) }
    }

    /// Approve or reject an action the agent proposed.
    func resolveAgentAction(_ actionID: Int64, approve: Bool) throws(CoreClientError) {
        do {
            try core.resolveAgentAction(actionId: actionID, approve: approve)
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    func agentActions(limit: UInt32 = 200) async throws(CoreClientError) -> [AgentActionInfo] {
        try await call { try await core.listAgentActions(limit: limit) }
    }

    /// Reversible tools that need the user's approval (spec §10.3).
    func setAgentPolicy(_ approveTools: [String]) throws(CoreClientError) {
        do {
            try core.setAgentPolicy(approveTools: approveTools)
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    var agentPolicy: [String] { core.agentPolicy() }
    var configurableAgentTools: [String] { core.configurableAgentTools() }

    /// Development: scripted agents instead of the real CLIs.
    func useFakeAgents() { core.debugUseFakeAgents() }

    // MARK: Routines

    func routines() async throws(CoreClientError) -> [RoutineInfo] {
        try await call { try await core.listRoutines() }
    }

    func createRoutineFromTemplate(runner: String) async throws(CoreClientError) -> RoutineInfo {
        try await call { try await core.createRoutineFromTemplate(runner: runner) }
    }

    func saveRoutine(json: String) async throws(CoreClientError) -> RoutineInfo {
        try await call { try await core.saveRoutine(definitionJson: json) }
    }

    func deleteRoutine(_ id: String) async throws(CoreClientError) {
        try await call { try await core.deleteRoutine(id: id) }
    }

    func routinePrompt(_ id: String) async throws(CoreClientError) -> String {
        try await call { try await core.routinePrompt(id: id) }
    }

    func runRoutineNow(_ id: String) async throws(CoreClientError) -> String {
        try await call { try await core.runRoutineNow(id: id) }
    }

    func previewRoutine(_ id: String) async throws(CoreClientError) -> String {
        try await call { try await core.previewRoutine(id: id) }
    }

    func routinePreview(_ sessionID: String) -> [RoutinePreviewRow]? {
        core.routinePreview(sessionId: sessionID)
    }

    func routineRuns(_ id: String, limit: UInt32 = 20) async throws(CoreClientError) -> [RoutineRunInfo] {
        try await call { try await core.listRoutineRuns(id: id, limit: limit) }
    }

    /// Create or update the routine at claude.ai through the user's CLI.
    func publishRoutineToCloud(_ id: String) async throws(CoreClientError) -> RoutineInfo {
        try await call { try await core.publishRoutineToCloud(id: id) }
    }

    func setRoutineEnabled(_ id: String, _ enabled: Bool) async throws(CoreClientError) -> RoutineInfo {
        try await call { try await core.setRoutineEnabled(id: id, enabled: enabled) }
    }

    func runCloudRoutineNow(_ id: String) async throws(CoreClientError) -> String? {
        try await call { try await core.runCloudRoutineNow(id: id) }
    }

    func refreshCloudRuns(_ id: String) async throws(CoreClientError) {
        try await call { try await core.refreshCloudRuns(id: id) }
    }

    func routineHandoff(_ id: String) async throws(CoreClientError) -> RoutineHandoff {
        try await call { try await core.routineHandoff(id: id) }
    }

    func attachCloudRoutine(_ id: String, urlOrID: String) async throws(CoreClientError) -> RoutineInfo {
        try await call { try await core.attachCloudRoutine(id: id, urlOrId: urlOrID) }
    }

    /// Put a run's threads back in the inbox; returns how many.
    func undoRoutineRun(_ runID: Int64) async throws(CoreClientError) -> UInt32 {
        try await call { try await core.undoRoutineRun(runId: runID) }
    }

    func describeSchedule(_ rrule: String) -> String { core.describeSchedule(rrule: rrule) }
    func nextRunAt(_ rrule: String) -> Int64? { core.nextRunAt(rrule: rrule) }

    // MARK: Accounts and sync

    struct SignInStart: Sendable {
        let sessionID: String
        let authorizationURL: URL
    }

    struct ConnectedAccount: Sendable, Equatable {
        let accountID: String
        let email: String
    }

    /// Start Gmail sign-in; open the returned URL in the user's browser.
    /// `fullAccess` also asks Google for full mail access, which faster
    /// download over IMAP needs (spec §7.4).
    func beginGmailSignIn(clientID: String, clientSecret: String?, loginHint: String? = nil,
                          fullAccess: Bool = false) async throws(CoreClientError) -> SignInStart {
        let start = try await call {
            try await core.beginGmailSignIn(client: OAuthClientConfig(clientId: clientID, clientSecret: clientSecret),
                                            loginHint: loginHint, fullAccess: fullAccess)
        }
        guard let url = URL(string: start.authorizationUrl) else {
            throw CoreClientError(kind: .internalError, message: "invalid authorization URL")
        }
        return SignInStart(sessionID: start.sessionId, authorizationURL: url)
    }

    /// Wait for the browser to finish and create the account.
    func completeGmailSignIn(_ sessionID: String) async throws(CoreClientError) -> ConnectedAccount {
        let account = try await call { try await core.completeGmailSignIn(sessionId: sessionID) }
        return ConnectedAccount(accountID: account.accountId, email: account.email)
    }

    func cancelGmailSignIn(_ sessionID: String) {
        core.cancelGmailSignIn(sessionId: sessionID)
    }

    func accountHasCredentials(_ accountID: String) throws(CoreClientError) -> Bool {
        do {
            return try core.accountHasCredentials(accountId: accountID)
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    func startSync() throws(CoreClientError) {
        do {
            try core.startSync()
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    /// Start every signed-in account's sync in the background; returns the
    /// accounts whose sign-in could not be read (spec §7.7).
    func startAllSync() async throws(CoreClientError) -> [String] {
        try await call { try await core.startAllSync() }
    }

    func stopSync() { core.stopSync() }
    func setAppActive(_ active: Bool) { core.setAppActive(active: active) }
    func syncNow() { core.syncNow() }

    /// How far back mail is downloaded (spec §7.4).
    func syncWindow() async throws(CoreClientError) -> SyncWindow {
        try await call { try await core.syncWindow() }
    }

    func syncWindow(for accountID: String) async throws(CoreClientError) -> SyncWindow {
        try await call { try await core.syncWindowFor(accountId: accountID) }
    }

    func setSyncWindow(_ window: SyncWindow, for accountID: String) async throws(CoreClientError) {
        try await call { try await core.setSyncWindowFor(accountId: accountID, window: window) }
    }

    func setSyncWindow(_ window: SyncWindow) async throws(CoreClientError) {
        try await call { try await core.setSyncWindow(window: window) }
    }

    // MARK: Clean Up (spec §14.12)

    // Every call names its account: the window cleans one account,
    // whatever the main window shows.

    /// The groups of `view` in `scope`, filtered by `filter`, in the core's order.
    func cleanupGroups(accountID: String, view: CleanupView, scope: CleanupScope,
                       filter: String) async throws(CoreClientError) -> [CleanupGroup] {
        try await call { try await core.cleanupGroups(accountId: accountID, view: view, scope: scope, filter: filter) }
    }

    /// A page of the messages in the groups named by `keys`, newest first.
    func cleanupMessages(accountID: String, view: CleanupView, scope: CleanupScope, keys: [String],
                         offset: UInt32, limit: UInt32) async throws(CoreClientError) -> [CleanupMessage] {
        try await call {
            try await core.cleanupMessages(accountId: accountID, view: view, scope: scope, keys: keys,
                                           offset: offset, limit: limit)
        }
    }

    /// How many messages the groups named by `keys` hold now.
    func cleanupCount(accountID: String, view: CleanupView, scope: CleanupScope,
                      keys: [String]) async throws(CoreClientError) -> UInt64 {
        try await call { try await core.cleanupCount(accountId: accountID, view: view, scope: scope, keys: keys) }
    }

    /// Apply `action` to every message in the groups named by `keys`, as
    /// they are now: one undoable action (undone with `undo(_:)`).
    func cleanupApply(accountID: String, view: CleanupView, scope: CleanupScope, keys: [String],
                      action: CleanupAction) async throws(CoreClientError) -> CleanupResult {
        try await call {
            try await core.cleanupApply(accountId: accountID, view: view, scope: scope, keys: keys, action: action)
        }
    }

    /// What Unsubscribe would do for the ticked groups: one target per list.
    func cleanupUnsubscribeTargets(accountID: String, view: CleanupView, scope: CleanupScope,
                                   keys: [String]) async throws(CoreClientError) -> [CleanupUnsubscribeTarget] {
        try await call {
            try await core.cleanupUnsubscribeTargets(accountId: accountID, view: view, scope: scope, keys: keys)
        }
    }

    /// The one-click unsubscribes (RFC 8058) of the targets the user
    /// confirmed, as `cleanupUnsubscribeTargets` gave them: a list whose
    /// newest message changed since is not posted to. Mailto targets are
    /// the composer's.
    func cleanupUnsubscribe(accountID: String, view: CleanupView, scope: CleanupScope,
                            targets: [CleanupUnsubscribeTarget]) async throws(CoreClientError)
        -> [CleanupUnsubscribeResult] {
        try await call {
            try await core.cleanupUnsubscribe(accountId: accountID, view: view, scope: scope, targets: targets)
        }
    }

    /// The Inbox Zero card's numbers; also records today's count at
    /// midnight and, the first time, the baseline (spec §14.12).
    func cleanupProgress(accountID: String) async throws(CoreClientError) -> CleanupProgress {
        try await call { try await core.cleanupProgress(accountId: accountID) }
    }

    /// Snapshots only: the Inbox's daily counts, today's last, and the baseline.
    func debugSeedInboxHistory(accountID: String, counts: [UInt64], baseline: UInt64) async throws(CoreClientError) {
        try await call { try await core.debugSeedInboxHistory(accountId: accountID, counts: counts, baseline: baseline) }
    }

    /// Where the account's download stands, for loading every header when
    /// Clean Up opens (spec §14.12).
    func cleanupLoadStatus(accountID: String) async throws(CoreClientError) -> CleanupLoadStatus {
        try await call { try await core.cleanupLoadStatus(accountId: accountID) }
    }

    /// Set the account's sync window to Everything, keeping bodies where
    /// they were. `expectCheap`: widening without asking because headers
    /// were cheap; if they are not by now, nothing changes (`.needsAsk`).
    func cleanupLoadEveryHeader(accountID: String, expectCheap: Bool) async throws(CoreClientError)
        -> CleanupLoadOutcome {
        try await call { try await core.cleanupLoadEveryHeader(accountId: accountID, expectCheap: expectCheap) }
    }

    /// Without IMAP: how many messages loading all mail would download, and
    /// about how long it would take (while Clean Up's header load waits for
    /// IMAP, the messages still waiting).
    func cleanupLoadEstimate(accountID: String) async throws(CoreClientError) -> CleanupLoadEstimate {
        try await call { try await core.cleanupLoadEstimate(accountId: accountID) }
    }

    /// Load All Mail while Clean Up's header load waits for IMAP: the rest
    /// comes down whole over the API. Returns how many were queued.
    func cleanupLoadWaitingHeaders(accountID: String) async throws(CoreClientError) -> UInt64 {
        try await call { try await core.cleanupLoadWaitingHeaders(accountId: accountID) }
    }

    /// Which part of the download range gets full messages over IMAP.
    func bodyWindow(for accountID: String) async throws(CoreClientError) -> BodyWindow {
        try await call { try await core.bodyWindowFor(accountId: accountID) }
    }

    func setBodyWindow(_ window: BodyWindow, for accountID: String) async throws(CoreClientError) {
        try await call { try await core.setBodyWindowFor(accountId: accountID, bodyWindow: window) }
    }

    func signOut(_ accountID: String) async throws(CoreClientError) {
        try await call { try await core.signOut(accountId: accountID) }
    }

    /// Development hook: fill the open account with a synthetic mailbox.
    @discardableResult
    func seedDemoMailbox(threads: UInt32) async throws(CoreClientError) -> UInt32 {
        try await call { try await core.debugSeedDemoMailbox(threads: threads) }
    }

    /// Runs a core call, converting generated errors to `CoreClientError`.
    /// Compose operations pinned to one account (spec §7.7).
    nonisolated func composer(for accountID: String) -> AccountComposer {
        core.composerFor(accountId: accountID)
    }

    /// Map a core call's errors like `call` does, for handles other than
    /// `Core` (e.g. `AccountComposer`).
    static func bridge<T>(_ body: () async throws -> T) async throws(CoreClientError) -> T {
        do {
            return try await body()
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    private func callSync<T>(_ body: () throws -> T) throws(CoreClientError) -> T {
        do {
            return try body()
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    private func call<T>(_ body: () async throws -> T) async throws(CoreClientError) -> T {
        do {
            return try await body()
        } catch let error as CoreError {
            throw CoreClientError(error)
        } catch {
            throw CoreClientError(kind: .internalError, message: String(describing: error))
        }
    }

    /// Diagnostics hook: ask Rust to emit one ThreadsChanged per id.
    func debugEmitThreadsChanged(mailboxID: String, threadIDs: [String]) {
        core.debugEmitThreadsChanged(mailboxId: mailboxID, threadIds: threadIDs)
    }

    /// `~/Library/Logs/OpenAGC`, where the core writes `core.log`.
    static func defaultLogDirectory() -> URL {
        URL.libraryDirectory.appending(path: "Logs/OpenAGC", directoryHint: .isDirectory)
    }

    /// `~/Library/Application Support/OpenAGC`, created if missing.
    static func defaultDataDirectory() throws -> URL {
        let dir = URL.applicationSupportDirectory.appending(path: "OpenAGC", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }
}

/// Core errors as the app sees them, without exposing generated types.
struct CoreClientError: Error, Equatable {
    enum Kind: Equatable {
        case invalidInput, notFound, storage, network, auth, rateLimited
        case agent, permissionDenied, cancelled, internalError
    }

    let kind: Kind
    let message: String

    init(kind: Kind, message: String) {
        self.kind = kind
        self.message = message
    }

    fileprivate init(_ error: CoreError) {
        switch error {
        case let .Failed(kind, message):
            self.init(kind: Kind(kind), message: message)
        }
    }
}

private extension CoreClientError.Kind {
    init(_ kind: ErrorKind) {
        switch kind {
        case .invalidInput: self = .invalidInput
        case .notFound: self = .notFound
        case .storage: self = .storage
        case .network: self = .network
        case .auth: self = .auth
        case .rateLimited: self = .rateLimited
        case .agent: self = .agent
        case .permissionDenied: self = .permissionDenied
        case .cancelled: self = .cancelled
        case .internal: self = .internalError
        }
    }
}

// MARK: - Data records

// Plain data records generated from Rust (spec §4.2). Aliased here so the
// rest of the app can use them without importing OpenAGCCore; all calls
// into the core still go through CoreClient.
typealias AddressInfo = OpenAGCCore.AddressInfo
typealias AgentActionInfo = OpenAGCCore.AgentActionInfo
typealias AiCompositionInfo = OpenAGCCore.AiCompositionInfo
typealias AgentEventInfo = OpenAGCCore.AgentEventInfo
typealias AgentProviderInfo = OpenAGCCore.AgentProviderInfo
typealias AgentSessionInfo = OpenAGCCore.AgentSessionInfo
typealias AgentTranscriptItem = OpenAGCCore.AgentTranscriptItem
typealias AgentStatusInfo = OpenAGCCore.AgentStatusInfo
typealias AttachmentInfo = OpenAGCCore.AttachmentInfo
typealias PromptContextInfo = OpenAGCCore.PromptContextInfo

extension PromptContextInfo {
    /// No references at all: the agent works from the prompt alone.
    static let empty = PromptContextInfo(mailboxId: nil, listDescription: nil, visibleThreadIds: [],
                                         selectedThreadIds: [], searchQuery: nil)
}
typealias RoutineHandoff = OpenAGCCore.RoutineHandoff
typealias RoutineInfo = OpenAGCCore.RoutineInfo
typealias RoutinePreviewRow = OpenAGCCore.RoutinePreviewRow
typealias RoutineRunInfo = OpenAGCCore.RoutineRunInfo
typealias DraftAttachmentInfo = OpenAGCCore.DraftAttachmentInfo
typealias DraftInfo = OpenAGCCore.DraftInfo
typealias DraftStatus = OpenAGCCore.DraftStatus
typealias LabelInfo = OpenAGCCore.LabelInfo
typealias InboxCategory = OpenAGCCore.InboxCategory
typealias SyncDiagnostics = OpenAGCCore.SyncDiagnostics
typealias TransportOp = OpenAGCCore.TransportOp
typealias TransportComparison = OpenAGCCore.TransportComparison
typealias MailboxInfo = OpenAGCCore.MailboxInfo
typealias SyncWindow = OpenAGCCore.SyncWindow
typealias BodyWindow = OpenAGCCore.BodyWindow
typealias UndoToken = OpenAGCCore.UndoToken
typealias CleanupView = OpenAGCCore.CleanupView
typealias CleanupScope = OpenAGCCore.CleanupScope
typealias CleanupAction = OpenAGCCore.CleanupAction
typealias CleanupGroup = OpenAGCCore.CleanupGroup
typealias CleanupMessage = OpenAGCCore.CleanupMessage
typealias CleanupResult = OpenAGCCore.CleanupResult
typealias CleanupLoadStatus = OpenAGCCore.CleanupLoadStatus
typealias CleanupLoadEstimate = OpenAGCCore.CleanupLoadEstimate
typealias CleanupProgress = OpenAGCCore.CleanupProgress
typealias CleanupDay = OpenAGCCore.CleanupDay
typealias CleanupUnsubscribeTarget = OpenAGCCore.CleanupUnsubscribeTarget
typealias CleanupUnsubscribeMethod = OpenAGCCore.CleanupUnsubscribeMethod
typealias CleanupUnsubscribeResult = OpenAGCCore.CleanupUnsubscribeResult
typealias AccountSummary = OpenAGCCore.AccountSummary
typealias AccountKind = OpenAGCCore.AccountKind
typealias AgentService = OpenAGCCore.AgentService
typealias AgentSendMode = OpenAGCCore.AgentSendMode
typealias AgentClient = OpenAGCCore.AgentClient
typealias AgentConnection = OpenAGCCore.AgentConnection
typealias AgentDomain = OpenAGCCore.AgentDomain
typealias AgentSendRule = OpenAGCCore.AgentSendRule
typealias AgentDnsRecord = OpenAGCCore.AgentDnsRecord
typealias AgentMailboxPlan = OpenAGCCore.AgentMailboxPlan
typealias AgentMailboxCreated = OpenAGCCore.AgentMailboxCreated
typealias AgentVerification = OpenAGCCore.AgentVerification
typealias ServiceAccountSummary = OpenAGCCore.ServiceAccountSummary
typealias RulesPublication = OpenAGCCore.RulesPublication
typealias RulesPreview = OpenAGCCore.RulesPreview
typealias RulesPreviewEntry = OpenAGCCore.RulesPreviewEntry
typealias RulesPreviewFact = OpenAGCCore.RulesPreviewFact
typealias RulesPreviewAudience = OpenAGCCore.RulesPreviewAudience
typealias RulesConnectInfo = OpenAGCCore.RulesConnectInfo
typealias RulesConnectCode = OpenAGCCore.RulesConnectCode
typealias RulesAgentToken = OpenAGCCore.RulesAgentToken
typealias RulesAgent = OpenAGCCore.RulesAgent
typealias RulesAgentKind = OpenAGCCore.RulesAgentKind
typealias AgentAdded = OpenAGCCore.AgentAdded
typealias ImportStatus = OpenAGCCore.ImportStatus
typealias BackfillStatus = OpenAGCCore.BackfillStatus
typealias OrphanedStore = OpenAGCCore.OrphanedStore
typealias MailboxScan = OpenAGCCore.MailboxScan
typealias MailboxKind = OpenAGCCore.MailboxKind
typealias MessageInfo = OpenAGCCore.MessageInfo
typealias RenderedBody = OpenAGCCore.RenderedBody
typealias ThreadDetail = OpenAGCCore.ThreadDetail
typealias ThreadPage = OpenAGCCore.ThreadPage
typealias ThreadRow = OpenAGCCore.ThreadRow
typealias TaskItem = OpenAGCCore.TaskItem
typealias NewTask = OpenAGCCore.NewTask
typealias TaskEdit = OpenAGCCore.TaskEdit
typealias TaskAction = OpenAGCCore.TaskAction
typealias TaskSuggestion = OpenAGCCore.TaskSuggestion
typealias GuideCategoryInfo = OpenAGCCore.GuideCategoryInfo
typealias GuideEntry = OpenAGCCore.GuideEntry
typealias GuideEntryFields = OpenAGCCore.GuideEntryFields
typealias GuideEdit = OpenAGCCore.GuideEdit
typealias GuideChange = OpenAGCCore.GuideChange
typealias GuideKind = OpenAGCCore.GuideKind
typealias GuideStatus = OpenAGCCore.GuideStatus
typealias GuideSource = OpenAGCCore.GuideSource
typealias GuideScope = OpenAGCCore.GuideScope
typealias GuideCheck = OpenAGCCore.GuideCheck
typealias GuideCheckKind = OpenAGCCore.GuideCheckKind
typealias GuideQuote = OpenAGCCore.GuideQuote
typealias GuideImport = OpenAGCCore.GuideImport
typealias AudienceGroup = OpenAGCCore.AudienceGroup
typealias AudienceStatus = OpenAGCCore.AudienceStatus
typealias GuideProgress = OpenAGCCore.GuideProgress
typealias GuideRunInfo = OpenAGCCore.GuideRunInfo
typealias AnalysisProgress = OpenAGCCore.AnalysisProgress
typealias FactInfo = OpenAGCCore.FactInfo
typealias FactScope = OpenAGCCore.FactScope
typealias FactMergeResult = OpenAGCCore.FactMergeResult
typealias FactFields = OpenAGCCore.FactFields
typealias FactEdit = OpenAGCCore.FactEdit
typealias FactUse = OpenAGCCore.FactUse
typealias FactSource = OpenAGCCore.FactSource
typealias FactStatus = OpenAGCCore.FactStatus
typealias FactChange = OpenAGCCore.FactChange
typealias FactCategoryInfo = OpenAGCCore.FactCategoryInfo
typealias CategoryEdit = OpenAGCCore.CategoryEdit
typealias StarterSet = OpenAGCCore.StarterSet
typealias StarterSetInfo = OpenAGCCore.StarterSetInfo
typealias AnalysisQueue = OpenAGCCore.AnalysisQueue
typealias ProposalPage = OpenAGCCore.ProposalPage
typealias AnalysisFactProposalInfo = OpenAGCCore.AnalysisFactProposalInfo
typealias FactsFrom = OpenAGCCore.FactsFrom
typealias AnalysisSettings = OpenAGCCore.AnalysisSettings
typealias AnalysisProposalInfo = OpenAGCCore.AnalysisProposalInfo
typealias AnalysisPairInfo = OpenAGCCore.AnalysisPairInfo
typealias AnalysisMetrics = OpenAGCCore.AnalysisMetrics
typealias AnalysisOp = OpenAGCCore.AnalysisOp
typealias GuideEntryHealth = OpenAGCCore.GuideEntryHealth
typealias AnalysisRunInfo = OpenAGCCore.AnalysisRunInfo
typealias GuideRunKind = OpenAGCCore.GuideRunKind
typealias GuideRunStatus = OpenAGCCore.GuideRunStatus
typealias GuideRunRequest = OpenAGCCore.GuideRunRequest
typealias GuideSampleFilter = OpenAGCCore.GuideSampleFilter
typealias GuideSampleInfo = OpenAGCCore.GuideSampleInfo
typealias GuideRendered = OpenAGCCore.GuideRendered
typealias GuideCheckFailure = OpenAGCCore.GuideCheckFailure
typealias GuideChangeQuestion = OpenAGCCore.GuideChangeQuestion
typealias GuideMergePlan = OpenAGCCore.GuideMergePlan
typealias GuideMergeDecision = OpenAGCCore.GuideMergeDecision

// MARK: - Events

/// What changed in a mailbox's thread list (spec §4.3).
struct ThreadChangeHint: Sendable, Equatable {
    var inserted: [String] = []
    var updated: [String] = []
    var removed: [String] = []
    /// Too much changed to describe; re-query the visible window.
    var invalidate = false
}

enum CoreClientEvent: Sendable, Equatable {
    /// An event and the account it is about (spec §7.7).
    struct Tagged: Sendable, Equatable {
        let accountID: String?
        let event: CoreClientEvent
    }

    enum SyncState: Sendable, Equatable { case idle, bootstrapping, syncing, offline, error }

    /// A message that just arrived, unread in the Inbox.
    struct NewMail: Sendable, Equatable {
        let messageID: String
        let threadID: String
        let senderName: String
        let subject: String
        let snippet: String
    }

    case threadsChanged(mailboxID: String, hint: ThreadChangeHint)
    /// `message`: why sync paused or stopped, in the provider's words.
    case syncStatus(SyncState, pending: UInt32, headers: UInt32, message: String?)
    case outboxStatus(pending: UInt32, failed: UInt32)
    case newMail([NewMail])
    case agent(sessionID: String, events: [AgentEventInfo])
    case routinesChanged
    case tasksChanged
    case guideChanged
    case guideProgress(GuideProgress)
    case analysisProgress(AnalysisProgress)
    case analysisChanged
    case factsChanged
    /// An agent mailbox's publishing to a rules server moved on (spec §10.6).
    case rulesPublicationChanged
    case importProgress(ImportStatus)
    case error(CoreClientError)
}

/// Rust's view of the Keychain (spec §12). Errors cross as `CoreError`.
private final class SecretBridge: SecretStore, Sendable {
    private let keychain: KeychainSecretStore

    init(_ keychain: KeychainSecretStore) {
        self.keychain = keychain
    }

    func get(key: String) throws -> String? {
        do { return try keychain.get(key) } catch { throw CoreError.Failed(kind: .storage, message: error.description) }
    }

    func set(key: String, value: String) throws {
        do { try keychain.set(key, value) } catch { throw CoreError.Failed(kind: .storage, message: error.description) }
    }

    func delete(key: String) throws {
        do { try keychain.delete(key) } catch { throw CoreError.Failed(kind: .storage, message: error.description) }
    }
}

/// PDF text for agents (spec §10.2), through PDFKit rather than a Rust
/// parser. Called on a core worker thread.
final class PDFTextExtractor: TextExtractor, Sendable {
    func pdfText(path: String) -> String? {
        guard let document = PDFDocument(url: URL(filePath: path)), !document.isLocked else { return nil }
        let text = document.string?.trimmingCharacters(in: .whitespacesAndNewlines)
        return text?.isEmpty == false ? text : nil
    }
}

/// Receives events on a Rust runtime thread and hands them to the stream.
private final class EventBridge: EventListener, Sendable {
    private let continuation: AsyncStream<CoreClientEvent.Tagged>.Continuation

    init(_ continuation: AsyncStream<CoreClientEvent.Tagged>.Continuation) {
        self.continuation = continuation
    }

    func onEvent(accountId: String?, event: CoreEvent) {
        // Rust warn/error records are logged here rather than delivered to
        // stores; Swift owns unified-logging privacy (spec §17). Rust has
        // already kept secrets and mail content out, and scrubbed addresses
        // and tokens (logging::scrub), so they can be public. Messages from
        // core *errors* can quote user data and are logged `.private`.
        if case let .log(level, target, message) = event {
            let logger = Logger(subsystem: "ai.actual.openagc", category: target)
            switch level {
            case .warn: logger.warning("\(message, privacy: .public)")
            case .error: logger.error("\(message, privacy: .public)")
            }
            return
        }
        if let mapped = CoreClientEvent(event) {
            continuation.yield(.init(accountID: accountId, event: mapped))
        }
    }
}

private extension CoreClientEvent {
    /// `nil` for events handled inside the bridge (log records).
    init?(_ event: CoreEvent) {
        switch event {
        case let .threadsChanged(mailboxId, hint):
            self = .threadsChanged(mailboxID: mailboxId, hint: ThreadChangeHint(
                inserted: hint.inserted, updated: hint.updated,
                removed: hint.removed, invalidate: hint.invalidate))
        case let .syncStatus(state, pending, pendingHeaders, message):
            self = .syncStatus(SyncState(state), pending: pending, headers: pendingHeaders, message: message)
        case let .outboxStatus(pending, failed):
            self = .outboxStatus(pending: pending, failed: failed)
        case let .error(kind, message):
            self = .error(CoreClientError(kind: .init(kind), message: message))
        case .routinesChanged:
            self = .routinesChanged
        case .tasksChanged:
            self = .tasksChanged
        case .guideChanged:
            self = .guideChanged
        case let .guideProgress(progress):
            self = .guideProgress(progress)
        case let .analysisProgress(progress):
            self = .analysisProgress(progress)
        case .analysisChanged:
            self = .analysisChanged
        case .factsChanged:
            self = .factsChanged
        case .rulesPublicationChanged:
            self = .rulesPublicationChanged
        case let .agentEvents(sessionId, events):
            self = .agent(sessionID: sessionId, events: events)
        case let .newMail(messages):
            self = .newMail(messages.map {
                NewMail(messageID: $0.messageId, threadID: $0.threadId,
                        senderName: $0.from.map { $0.name ?? $0.email } ?? "Unknown sender",
                        subject: $0.subject, snippet: $0.snippet)
            })
        case let .importProgress(status):
            self = .importProgress(status)
        case .log:
            return nil
        }
    }
}

private extension CoreClientEvent.SyncState {
    init(_ state: OpenAGCCore.SyncState) {
        switch state {
        case .idle: self = .idle
        case .bootstrapping: self = .bootstrapping
        case .syncing: self = .syncing
        case .offline: self = .offline
        case .error: self = .error
        }
    }
}
