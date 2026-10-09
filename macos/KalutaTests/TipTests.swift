import Foundation
import Testing
@testable import Kaluta

@MainActor
struct TipTests {
    private var inbox = Tip.Context(inInbox: true, searching: false, categoriesAvailable: true, categoriesShown: true,
                                    importantOnly: false, agentShown: false)

    @Test func tipsComeOneAtATimeInOrder() {
        #expect(Tip.next(dismissed: [], context: inbox) == .categories)
        #expect(Tip.next(dismissed: ["categories"], context: inbox) == .importantOnly)
        #expect(Tip.next(dismissed: ["categories", "importantOnly"], context: inbox) == .agent)
        #expect(Tip.next(dismissed: ["categories", "importantOnly", "agent"], context: inbox) == nil)
    }

    @Test func tipsSkipWhatDoesNotApply() {
        var c = inbox
        c.categoriesAvailable = false
        #expect(Tip.next(dismissed: [], context: c) == .importantOnly, "no categories in this account")
        c.importantOnly = true
        c.agentShown = true
        #expect(Tip.next(dismissed: [], context: c) == nil, "already using both")
        var searching = inbox
        searching.searching = true
        #expect(Tip.next(dismissed: [], context: searching) == nil)
        var elsewhere = inbox
        elsewhere.inInbox = false
        #expect(Tip.next(dismissed: [], context: elsewhere) == nil)
        var agentMailbox = inbox
        agentMailbox.categoriesAvailable = false
        agentMailbox.importantAvailable = false
        #expect(Tip.next(dismissed: [], context: agentMailbox) == .agent, "no Important marks in an agent mailbox")
    }

    @Test func actingOnATipDoesItAndPutsItAwayForGood() {
        let suite = "kaluta-tests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = AppModel(core: nil, defaults: defaults)
        model.finishTip(.importantOnly, accept: true)
        #expect(model.inboxImportantOnly)
        model.finishTip(.agent, accept: false)
        #expect(!model.agent.isPresented, "Not Now does not open it")
        #expect(Set(defaults.stringArray(forKey: AppModel.dismissedTipsKey) ?? []) == ["importantOnly", "agent"])
        #expect(AppModel(core: nil, defaults: defaults).dismissedTips == ["importantOnly", "agent"], "remembered")
    }

    @Test func aLargeInboxSuggestsCleanUpFirstUntilItIsOpenedOrPutAway() {
        var large = inbox
        large.inboxCount = 1_001
        #expect(Tip.next(dismissed: [], context: large) == .cleanUp)
        #expect(Tip.next(dismissed: ["cleanUp"], context: large) == .categories, "never again once put away")
        var exactly = inbox
        exactly.inboxCount = 1_000
        #expect(Tip.next(dismissed: [], context: exactly) == .categories, "more than 1,000 only")
        var imported = large
        imported.cleanUpAvailable = false
        #expect(Tip.next(dismissed: [], context: imported) == .categories, "an imported mailbox has no Clean Up")
        #expect(Tip.cleanUp.action == "Open Clean Up")
        #expect(Tip.cleanUp.dismiss == "Not Now")
    }

    @Test func openingCleanUpRetiresItsTip() {
        let suite = "kaluta-tests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = AppModel(core: nil, defaults: defaults)
        var opened = false
        model.openCleanUp = { opened = true }
        model.finishTip(.cleanUp, accept: true)
        #expect(opened, "Open Clean Up opens the window")
        let other = AppModel(core: nil, defaults: UserDefaults(suiteName: suite + "-2")!)
        other.cleanUpOpened()
        #expect(other.dismissedTips.contains("cleanUp"), "opened from the menu: the tip is done too")
        UserDefaults().removePersistentDomain(forName: suite + "-2")
    }
}
