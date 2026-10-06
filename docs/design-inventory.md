# Design pattern inventory

Every visual and interaction pattern in `macos/OpenAGC` as of 2026-09-29,
checked against `docs/design-system.md`, `macos/OpenAGC/Design/*.swift` and
`scripts/design-lint.sh`. It is the input for oagc-068.2, which writes the
missing patterns into the design system and extends the lint.

"Covered?" means the design system names the pattern and gives its rules:
**Yes** (named, with rules), **Partly** (named, or a token covers part of
it), **No** (not mentioned). Paths are relative to `macos/OpenAGC/`.

## Surfaces

| Pattern | Where | Covered? | Notes |
| --- | --- | --- | --- |
| Reader message cards (HTML) | `Features/MessageView/EmailDocument.swift:28-60`, CSS `:140-170` | No | One `<details>` card per message. The CSS uses its own numbers: card radius 12px (`Radius.panel`, but cards are `Radius.card` = 8), padding `10px 14px`, gap `10px` and `margin 0 0 8px` (10 and 14 are off the scale), body inset 42px, body font 14px/1.45. Card fill is `CanvasText 4%` with a 10 % outline, but the design system says only attention cards have outlines. Nothing ties the CSS to `Space`/`Radius`. |
| Reader card: collapsed vs expanded | `EmailDocument.swift:101-103`, CSS `:150,157,161` | No | Latest and unread messages open. Collapsed cards shrink the avatar (32 to 26px), hide address and To, and show a one-line snippet. Clicking toggles (native `<details>`); there is no keyboard shortcut to expand or collapse all. |
| Reader "paper" body | `EmailDocument.swift:51,105-111`, CSS `:166` | No | In dark mode, mail that sets its own colours is drawn on white paper (`#fff`/`#111`, radius 8, padding 12). Worth a rule: "never recolour sender HTML; give it paper". |
| Reader header (subject, message count, Edit Draft) | `Features/MessageView/ThreadReaderView.swift:31-53` | Partly | `TypeRole.title` is used for the subject, as documented. The count uses a raw `.font(.callout)` (`:46`) and not `TypeRole.meta`. It is not a `.columnHeader`: the header does not scroll, because the web view scrolls inside it. That exception is not written down. |
| Remote-images banner | `ThreadReaderView.swift:55-63` | Yes | `Banner(intent: .neutral, inset: Space.xxl)`, matching the doc. |
| Composer header rows (To/Cc/Bcc/Subject/From) | `Features/Composer/ComposerView.swift:118-164` | No | Right-aligned secondary label 64pt wide (literal), `Space.l`/`Space.s` padding, an `InsetRule` under every row, top-aligned on purpose (the comment at `:149-152` explains a hang). The "Cc/Bcc" link button uses `.font(.callout)`. |
| Composer quote pane and toggle | `ComposerView.swift:66-81,166-190,280-291` | No | `VSplitView` between editor and original. The quote CSS is separate from the reader's: 13px, `margin 8px 16px`, GrayText. The toggle bar pads `Space.xl`, but the header rows and writing-help bar pad `Space.l`. |
| Writing-help bar (ComposerAssistant) | `ComposerView.swift:193-251`; `Features/Composer/ComposerAssistant.swift` | No | A sparkles icon, a plain field, an ideas menu, then Write or Stop. After writing, "Written by X. Read it before sending." with a link-style Undo. Errors are red caption text. It sits on an `InsetRule` overlay, not in a bar or band. Its Undo is local to the bar and separate from the app's undo notice and ⌘Z. |
| Composer banners (created by agent, save error) | `ComposerView.swift:58-65` | Yes | `Banner(.info)` and `Banner(.caution)`, matching the doc. |
| Agent prompt strip under the reader | `App/MainWindow.swift:54-61`; `Features/Agent/AgentPanel.swift:5-125` | Partly | `.glassCapsule()` in a strip of its own, as the Surfaces table says. The strip's `maxWidth: 680` and `Space.xl`/`Space.l` padding are inline in MainWindow. The doc comment at `AgentPanel.swift:3` still says the capsule is "floating over the reader". |
| Suggestion chips over the prompt | `AgentPanel.swift:128-155` | Partly | Each chip is its own glass capsule (`.glassEffect(.regular.interactive())`), not `.glassCapsule()`, with `Tone.highlight` for the keyboard-highlighted chip. ↑/↓/Tab move, Return chooses, Escape hides, and VoiceOver announces the count (`:102-109`). The keyboard model is not documented. |
| Agent column header | `AgentPanel.swift:233-277` | Yes | `.columnHeader` holding `TypeRole.heading`, Approve All, Stop, a history menu and New Conversation. |
| Agent capabilities (empty column) | `AgentPanel.swift:160-199` | No | Groups of example prompts as tinted plain buttons in curly quotes. The heading uses `.font(.headline)` (`:167`), not `TypeRole.heading`. This is the column's empty state, but it is not a `ContentUnavailableView`, and the doc does not describe it. |
| Agent transcript entries | `AgentPanel.swift:280-344` | Partly | The user's prompt is a `.card(.info)`; the reply is plain text; thinking and tool calls are `DisclosureGroup`s (tool status: spinner, secondary check, red `xmark.octagon`). Errors are an orange label (`:336-339`), not a `Banner` or card, and orange is the caution tone, not failure. The results list draws its own background and a `.separator` outline (`:334-335`) instead of `.card`, which breaks "outlines only on attention cards". |
| Agent approval card (ProposalCard) | `AgentPanel.swift:348-415` | Yes | `.card(.attention)` while pending, `.neutral` after. Reject has `role: .destructive`, Approve is `.borderedProminent`, both `.small`. Approved is green, as documented. The Undo-send time uses `time: .standard` (with seconds). |
| Agent result rows | `AgentPanel.swift:418-441` | No | A SwiftUI copy of the thread row: `.font(.caption)`/`.callout`, one line of snippet, `.selection` background. Only `onTapGesture` selects it, so there is no keyboard or button trait path beyond `.isButton`. |
| Sheets | `Features/Accounts/ImportMailbox.swift:31-119`; `Features/Agent/AgentActivity.swift:104-142`; `Features/Routines/RoutinesWindow.swift:479-523` (PromptEditor), `:538-580` (HandoffSheet) | Partly | `Space.xxxl` is documented as "sheet padding", but no sheet uses it: Import uses `Space.xxl`, PromptEditor and Handoff use `Space.xl`, Activity uses `Space.l`. Sheet titles vary: `.title2.weight(.semibold)` (Import, `:37`), `.title3.weight(.semibold)` (progress, `:81`) and `.headline` (PromptEditor `:487`, Handoff `:548`). The doc says `TypeRole.title`. Only Import has Cancel on Escape and a default action on Return. PromptEditor and Handoff have neither, and only Activity puts a `PaneDivider` above its button bar. |
| Alerts | `Features/MessageView/MessageWebView.swift:82-89` | No | An `NSAlert` for mismatched links. The safe choice ("Don't Open") is first and default. This is the only alert in the app. |
| Confirmation dialogs | `Features/Settings/AccountSettings.swift:84-92` | Partly | Remove account: a question title, a consequence message and a destructive button. The `// no-help` exemption is documented, but the confirmation pattern itself is not. |
| Popover (label picker) | `Features/ThreadList/LabelPicker.swift:53-160` | No | A transient `NSPopover` with a filter field (focused on appear), tree rows indented `depth * 14` (literal) and "Create …". Return applies the first match. It is 280 wide with `maxHeight: 320` (literals). Errors are red caption text. |
| Settings panes | `Features/Settings/SettingsView.swift`, `AccountSettings.swift`, `Features/Agent/AgentSettings.swift`, `AgentActivity.swift:7-101` | No | All `Form` + `.formStyle(.grouped)`, 640×520 tabs. Footers repeat `.foregroundStyle(.secondary)` although grouped-form footers are secondary already. Header case is mixed: "Sending" and "New Mail" are title case, "Google sign-in client", "Mail on this Mac" and "Leave alone" are sentence case. `SyncWindowSection` (`AccountSettings.swift:255-286`) is only used for its `choices`, and its "Last month" differs from the row picker's "Last 30 days" (`:125`). |
| Sync Debugger grids | `Features/SyncDebugger/SyncDebuggerView.swift:193-279` | No | Label/value `Grid` (secondary label, `TypeRole.meta`) and table grids with a `caption.semibold` secondary header row and monospaced digits. Section titles use `TypeRole.groupLabel`. Page padding is `Space.xxxl`, although that token is documented for sheets. This is the best-tokenised window in the app, so it is a good model for a "data grid" entry. |
| Onboarding | `Features/Onboarding/OnboardingView.swift` | Partly | `Space.page` padding, as documented. The title uses `.title.weight(.semibold)` (no token), the Agents heading `.font(.headline)`, and `.controlSize(.large)` buttons. Its layout is shared with nothing else. Sign-in errors are a red label, which is fine. Provider status is green when ready (`:106`), but the doc reserves green for "approved". |
| Routines window | `Features/Routines/RoutinesWindow.swift` | No | `NavigationSplitView`: a list with name (`.headline`), runner badge and activity (`.caption`), and a mini switch. The editor is a grouped Form with a bottom action bar (`.background(.bar)`, `Space.l`, `:307-345`). The action bar is a third kind of bar, not `Banner` or `ListHeaderBar`. "Unpublished changes" and the hand-edited prompt warning are orange text. Bucket colour dots are 10×10 (literal). |
| Sidebar | `Features/Sidebar/SidebarView.swift` | Partly | System `List(.sidebar)` with Favorites, the account section (with the label tree) and Routines, plus `.badge` counts. Drafts show a total and Sent/Archive/Trash/Spam show none (`:161-168`). Label colours tint `tag.fill`. `activityIndent = 26` follows the doc's named-constant rule. Groups are secondary `folder` rows. None of these rules is in the doc. |
| Sidebar footer (sync status) | `Features/Sidebar/SyncStatusView.swift` | No | A mini linear progress bar (max 150) over a `caption.medium` title and a `caption2` detail, placed in `safeAreaInset(.bottom)`. It uses raw fonts. |
| Thread list rows (AppKit) | `Features/ThreadList/ThreadRowView.swift` | Yes | As documented. Inconsistencies: the date is 11pt (`:16`), but `TypeRole.rowSecondary` (12pt) is documented as "the snippet and the date". The count uses a raw `systemFont(ofSize: 11, weight: .semibold)` (`:37`). Layout constants are inline (`padding 12`, gap `6`, lineHeight 17, top inset 10, date max 90). |
| Swipe actions | `Features/ThreadList/ThreadListView.swift:149-176` | No | Trailing Archive (`.systemPurple`) and leading Read/Unread (`.systemBlue`). These raw colours are not in `Tone`. |
| Context menu (thread list) | `ThreadListView.swift:238-305` | Partly | Menus keep separators (documented). Items show single-key equivalents with no modifier (`ActionItem`, `:328`), which is how list keys surface. |
| Category tabs and list header | `App/MainWindow.swift:160-192` | Yes | `.columnHeader { ListHeaderBar { CapsuleTabs } }`, with `TipCard` under it. |
| List title and subtitle | `MainWindow.swift:72-113` | No | The title is the mailbox leaf name. The subtitle joins parts with " · ": category, "N unread", "Important only", "Filtered: …", "Imported mailbox · cannot send". This is the list's status line, and the Tasks list will need one. |
| Search progress and search error | `MainWindow.swift:128-142` | No | Ad-hoc: a secondary label or a spinner row with `.font(.callout)` and `.padding(Space.m)`, not a `Banner`. |
| Keyboard Shortcuts window | `App/KeyboardShortcuts.swift:68-93` | No | A `Grid` with keys monospaced and right-aligned. `horizontalSpacing: 18, verticalSpacing: 4` are literals: 18 is off the scale, and the lint misses them because of the capital S. Headings use `.font(.headline)`. |

## Components

| Pattern | Where | Covered? | Notes |
| --- | --- | --- | --- |
| `CapsuleTabs` | `Design/Components.swift:95-137`; used `MainWindow.swift:184` | Yes | `pillWidth = 28` is a named constant. Counts go in help and VoiceOver, as documented. It has no keyboard navigation between pills beyond Tab. |
| `TipCard` | `Components.swift:166-201`; `MainWindow.swift:170-177`; `Features/ThreadList/Tips.swift` | Yes | The dismiss help is fixed text ("will not come back"). Tip copy lives in `Tips.swift`. |
| `hoverHelp` / `ToolTipArea` | `Components.swift:139-161` | Yes | Used widely. Misuses: menu-bar items call `.hoverHelp("")` or `.hoverHelp(reason)` (`App/OpenAGCApp.swift:112,162,166,170`), where tooltips never show. Several texts end with a full stop against the rule (`ImportMailbox.swift:103`, `AccountSettings.swift:70,156,196,213`, `RoutinesWindow.swift:333`). Orphan-store Delete has its help on the row, not on the button (`AccountSettings.swift:63-70`). The undo notice's close button says "Dismiss" in help and "Close" to VoiceOver (`Features/Undo/UndoNoticeView.swift:38,41`). |
| `ToolbarHelp` / `ToolbarToolTips` | `Features/MessageView/MessageToolbar.swift:131-196` | Yes | One source for toolbar tips. Its shortcut strings are inconsistent, see Behaviour. |
| Undo notice | `Features/Undo/UndoNoticeView.swift`; `Features/Undo/MailUndo.swift:38-54,204-212` | Partly | The glass capsule is documented, but the notice's behaviour (8 s, pause on hover, focus or inactive window, one at a time, VoiceOver announcement, "Verb N conversations" wording) is only in the spec. It is also used for errors and non-undoable news ("Couldn't open the draft: …", "Already sent"); since oagc-068.3 those show no Undo button (`UndoNotice.offersUndo`). |
| `Banner` | `Components.swift:46-75`; `MainWindow.swift:208`, `ThreadReaderView.swift:56`, `ComposerView.swift:59,64` | Yes | Good. The same "sign in again" state is a yellow attention `Banner` in the main window but an orange label in Settings (`AccountSettings.swift:28`, `:147`). |
| Cards (`.card`) | `Design/Surfaces.swift:19-23` | Yes | Used by ProposalCard, the prompt entry and TipCard. Hand-rolled look-alikes: agent results (`AgentPanel.swift:334-335`) and the Handoff prompt box (`RoutinesWindow.swift:559`, `.quaternary.opacity(0.4)` + `Radius.control`). |
| `InsetRule`, `PaneDivider` | `Components.swift:28-42` | Yes | Used as documented. The composer overlays `InsetRule` at the top of bars (`ComposerView.swift:189,250,276`), a pattern the doc does not name. |
| `LabelChip` / label chips | `Components.swift:78-89`; AppKit `ThreadRowView.swift:88-104` | Partly | `LabelChip` is used nowhere outside `Design/`. Rows draw chips as attributed text with thin-space padding. A third chip style exists in the Routines preview: capsule, colour at 20 %, `.caption.monospaced()` (`RoutinesWindow.swift:417-419`). Composer attachment chips are a fourth (below). There is no chip for a category. |
| Attachment tiles (reader) | `Features/MessageView/AttachmentStrip.swift:16-72` | Partly | `Radius.control` + `Tone.controlFill` (documented tokens). Filename max width 220 and icon 16×16 are literals. Click previews, double-click opens, drag copies, and there is a context menu and an a11y "Open" action. Errors are red caption text under the strip. |
| Attachment chips (composer) | `ComposerView.swift:253-277` | No | These are capsules on `.quaternary` with a remove ✕, which differs from the reader's rounded-rect `Tone.controlFill` tiles, and they use a generic `doc` icon instead of the file-type icon. |
| Avatars: reader (HTML) | `EmailDocument.swift:72-98`, CSS `:148-150` | No | Ten hex colours with a 32-bit FNV hash and finaliser. Initials handle "Last, First". |
| Avatars: `AccountAvatar` | `Features/Accounts/AccountAvatar.swift` | No | Nine `NSColor.system*` colours with a 64-bit FNV hash. It does not handle "Last, First". The same address gets a different colour, and can get different initials, in Settings and in the reader. The menu version adds an accent ring for the current account (`Features/Accounts/AccountMenu.swift:92-101`). |
| Empty states | `MainWindow.swift:122,145,147,198`; `ComposerView.swift:25`; `AgentActivity.swift:127`; `SyncDebuggerView.swift:28`; `RoutinesWindow.swift:18-25` | Partly | `ContentUnavailableView` is used as documented. Other empty states are secondary text inside forms and lists ("No runs yet.", "No routines yet.", "Nothing has run yet.", "No labels", "No earlier conversations"); they need a written rule, and their trailing full stops are inconsistent. The agent column's empty state is a custom view. |
| Progress indicators | `MainWindow.swift:118,134-142`; `SyncStatusView.swift:13`; `AgentSettings.swift:15-18`; `AccountSettings.swift:34-37`; `RoutinesWindow.swift:321-324`; `ImportMailbox.swift:86-90` | No | These are `.small` spinners beside secondary text ending in "…", `.mini` spinners inside rows and cards, and linear bars for known totals. The pattern is consistent, but nothing writes it down. |
| Error text | `AttachmentStrip.swift:28-31`; `LabelPicker.swift:97`; `ComposerView.swift:242`; `AgentActivity.swift:48`; `OnboardingView.swift:56`; `SyncDebuggerView.swift:68`; `RoutinesWindow.swift:311,603` | Partly | "Red is only for failure" is documented, but the form of the text is not. Current forms: red caption, red callout, a red label with a triangle, and a red `TypeRole.meta`. Errors also appear orange: import error (`ImportMailbox.swift:52`), agent transcript error (`AgentPanel.swift:337-339`), reauth hint (`AccountSettings.swift:28`). |
| Warnings (caution text) | `RoutinesWindow.swift:61,139-140,493-494`; `AccountSettings.swift:147` | Partly | Orange `foregroundStyle` text, where the token is `Intent.caution` (a fill for bands and cards). Nothing says orange text is the inline caution form. |
| Status colours in tables | `AgentActivity.swift:157-164`; `RoutinesWindow.swift:467`; `SyncDebuggerView.swift:271` | No | Red for failed or denied, secondary for rejected or expired, orange for "pending" (Waiting). Waiting needs the user, which is the attention (yellow) intent, not caution. |
| Badges and counts | sidebar `.badge` (`SidebarView.swift:134,156`); row count (`ThreadRowView.swift:36-38,191-193`); `CapsuleTabs` counts in help; "Approve All (N)" (`AgentPanel.swift:241`); account menu "(12)" (`AccountMenu.swift:74`); reader "N messages" (`ThreadReaderView.swift:44-48`) | Partly | The row count is documented. The other count forms (parentheses in the account menu and Approve All, text in the reader, system badges) are not. |
| Importance and state markers | `ThreadRowView.swift:39-41,213-241` | Yes | The unread dot, replied arrow, Important chevron and star/paperclip badge are documented. Symbol point sizes 9 are literals. |
| Disclosure groups | `AgentPanel.swift:297,303`; `RoutinesWindow.swift:222,452`; `OnboardingView.swift:62`; `SidebarView.swift:108` | No | Used for thinking, tool calls, buckets, runs, Advanced and label groups. There is no rule for when to disclose rather than show. |
| Link-style buttons | `ComposerView.swift:125-128,236-238`; `AgentPanel.swift:178-184` | No | `.buttonStyle(.link)` for "Cc/Bcc" and Undo; tinted `.plain` for example prompts. |

## Behaviour

| Pattern | Where | Covered? | Notes |
| --- | --- | --- | --- |
| Toolbar groups | `MessageToolbar.swift:6-86`; `App/MainWindow.swift:45,70` | Yes | This matches the doc. The composer toolbar (Attach, Discard, Send in one `.primaryAction` group, `ComposerView.swift:88-104`) and the Routines toolbar (New Routine) are not described. |
| Disabled, not hidden | `MessageToolbar.swift:30-31`; `OpenAGCApp.swift:103-105` | Yes | Mostly followed. Exceptions: Approve All and Stop appear only when relevant (`AgentPanel.swift:240-250`), and the ListViewOptions menu appears only in the Inbox (`MessageToolbar.swift:12`). |
| Shortcut text in help tags | `MessageToolbar.swift:143-171`; `KeyboardShortcuts.swift:20-64` | Partly | The doc's example is "Archive (E)". Help tags write list single keys in upper case ("Archive (E)", "Label (L)", "Star (S)"), which reads as ⇧E, while the Keyboard Shortcuts window lists them in lower case ("e"). The menu equivalents differ: Archive is ⌃⌘A, Star is ⇧⌘L. "Move to Trash (⌘⌫)" names the menu key but "Archive (E)" names the list key, with no rule for which a tag names. Return is written "(Return)" and Escape "(Esc)" in tags, but "↩" in the shortcuts window. |
| Single keys in lists | `ThreadListView.swift:206-234` | No | Gmail/Mail keys (e, u, s, l, #, !, r, a, f, c, j, k, /, Return in Drafts, ⌫) apply only when no ⌘/⌃/⌥ is held. They are in the shortcuts window, but the rule "single keys act in the focused list; the same actions have ⌘ keys in menus" is not in the design system. The Tasks list will reuse e, r, a, f, c, ⌫ and ↩ with other meanings, so this needs writing down. |
| ⌘ keys in menus | `OpenAGCApp.swift:107-211` | No | Menu shortcuts apply only when the mail window is key (`isMailWindow`, `:97-103`). Not documented. |
| Shortcuts window completeness | `KeyboardShortcuts.swift` vs `OpenAGCApp.swift` | No | These are missing from the guide: ⇧⌘/ (Keyboard Shortcuts, `OpenAGCApp.swift:139`), ⌘Z/⇧⌘Z mail undo, ⌃1–⌃9 accounts (`AccountMenu.swift:63`), ⌘S in Routines (`RoutinesWindow.swift:338`), and the prompt's ↑/↓/Tab/Return/Escape. |
| Confirm vs undo | Undo: `MailUndo.swift`; confirm: `AccountSettings.swift:84`, `MessageWebView.swift:82` | No | "Act, then offer Undo; confirm only what cannot be undone" is in the plan, not the design system. Discard draft (when it has content), Delete leftover mail data and Delete routine now ask first (oagc-068.3). Still without confirmation: Remove API key (`AgentActivity.swift:35`), Clear Suggestions History (`AgentSettings.swift:35`); neither deletes mail. |
| Destructive button role | `AgentPanel.swift:371`; `AccountSettings.swift:63,172`; `RoutinesWindow.swift:233,318`; `AgentActivity.swift:35` | No | `role: .destructive` is used for Reject (which is not destructive) and for Remove on buckets (undoable with Revert). There is no rule. |
| Default and cancel actions | `ImportMailbox.swift:56-65,98`; `AgentActivity.swift:135` | No | Only 4 `keyboardShortcut(.defaultAction/.cancelAction)` in the app, and PromptEditor and HandoffSheet have none. The tip "(Esc)"/"(Return)" convention is informal. |
| Date formats | `ThreadRowView.swift:259-288` (list); `EmailDocument.swift:129-138` (reader); `AgentActivity.swift:113` and `RoutinesWindow.swift:466` (tables); `SyncDebuggerView.swift:178,189`; `AgentPanel.swift:387`; `RoutinesStore.swift:261-274` (relative) | No | There are six styles: Mail-style relative-ish (list and agent results), medium date + short time (reader), month-day-hour-minute (activity and runs), time with seconds (Sync Debugger, undo-send deadline), time only (breaker), and `RelativeDateTimeFormatter` (routine activity, created per call). Due dates for Tasks need a seventh unless a rule is written. |
| Relative wording and separators | `MainWindow.swift:95-113`; `AccountSettings.swift:237-249`; `RoutinesStore.swift:272` | No | " · " joins status parts everywhere, which is consistent but undocumented. "—" is used for missing values (`SyncDebuggerView.swift:146`). |
| Accessibility labels | `ThreadRowView.swift:136-140`; `ThreadListView.swift:120-138`; `Components.swift:126-132`; `AgentPanel.swift:51,58,147-153,404`; `AttachmentStrip.swift:69-71` | No | Good practice exists but is unwritten: rows read as one comma-joined label; VoiceOver custom actions mirror swipes and keys; icon-only controls get labels; avatars are hidden; announcements for suggestions and notices (`AgentPanel.swift:102-109`, `MailUndo.swift:245`). Gaps: agent result rows and routine preview rows select only through `onTapGesture` (`AgentPanel.swift:329-330`, `RoutinesWindow.swift:425`), and the reader's HTML cards have no landmarks or labels beyond `<summary>`. |
| Focus handling | `LabelPicker.swift:102`; `AgentPanel.swift:93`; `MainWindow.swift:79-80`; `ComposerView.swift:70` (`focusOnAppear`); `UndoNoticeView.swift:30,54` | No | Popovers focus their field on appear; ⌘K and ⌘F move focus by counters (`agentFocusRequests`, `searchFocusRequests`); the composer focuses the body when To is filled. Sheets set no initial focus. There is no rule, and the task dialog needs one (focus the title). |
| Motion | `MainWindow.swift:9,69`; `UndoNoticeView.swift:48,53`; `AgentPanel.swift:27,91` | No | Every animation checks `accessibilityReduceMotion`. The durations (0.15, 0.2, 0.25 s, `.snappy`/`.easeOut`) are literals with no tokens. |
| Hover and pointer | `UndoNoticeView.swift:45`; `AttachmentStrip.swift:59-61` | No | Hover pauses the notice. Click and double-click on tiles. |
| Drag and drop | `ThreadListView.swift:101-105,182-192`; `SidebarView.swift:124-129`; `AttachmentStrip.swift:106-118`; `ComposerView.swift:111-115` | No | Threads drag onto labels, attachments drag out as file promises, and files drop onto the composer. |
| Window sizes | `OpenAGCApp.swift:16,34,46,53`; `SettingsView.swift:25`; many `.frame(minWidth:)` | No | These are literals (1200×760, 720×560, 980×720, 900×720, 640×520, sheet widths 420/460/620). |
| Copy style | help texts; section headers; `Tips.swift` | Partly | "Short sentence without a full stop" is documented for help, and some texts break it (see `hoverHelp` above). Titles and buttons use title case. Section headers and sheet titles mix cases ("Import finished" vs "Import Mailbox"). British spelling ("summarise", "colour") sits beside "organize" and "Labeled" (`AccountSettings.swift:50`, `MailUndo.swift:50`). |

## Gaps to write down in oagc-068.2

In priority order:

1. **Dialog (sheet) pattern** (Tasks needs it: the `t` task dialog and the
   bulk sheet). Rules to set: title in `TypeRole.title`; one padding token
   (settle `Space.xxxl` vs the `Space.xxl` actually used); a button bar
   under a `PaneDivider`, trailing, Cancel then the default action;
   `.keyboardShortcut(.cancelAction)` on Cancel and `.defaultAction` on the
   primary; initial focus on the first field; progress and "ask again"
   inside the dialog. **Closest code:** `ImportMailboxSheet`
   (`Features/Accounts/ImportMailbox.swift:31-71`, the only sheet with both
   Return and Escape), with `AgentActivityView`'s `PaneDivider` + button
   bar (`Features/Agent/AgentActivity.swift:130-139`). Extract a `SheetScaffold`
   (title, content, buttons) into `Design/Components.swift` and move all
   four sheets onto it.
2. **List row with a due date** (Tasks list). Rules to set: the due date
   takes the date's slot at the trailing edge in `rowSecondary`; overdue
   uses the caution tone as text (`NSColor.systemOrange`, needs a
   `Tone.cautionNS`), never red; today may use the accent colour; grouped
   headings (Overdue, Today, This Week, Later, No Date). One date rule for
   due dates ("Today", "Tomorrow", weekday within a week, "Sep 12", short
   date) alongside `RowDateFormatter`. **Closest code:** `ThreadRowView`
   (`Features/ThreadList/ThreadRowView.swift`: fixed-height AppKit row,
   date slot `:156`, chip line `:88-104`) and `RowDateFormatter`
   (`:262-288`). Fix the date's font discrepancy (11pt vs `rowSecondary`)
   first so the new row copies the right thing.
3. **Category chips** (Tasks: a chip per row, chips in the dialog and
   bulk sheet). Rules to set: one chip shape and fill for SwiftUI and
   AppKit (`Radius.chip`, `Tone.chipFill`, `TypeRole.chip`), colour per
   category, and a selectable chip variant for choosing in the dialog.
   **Closest code:** `LabelChip` (`Design/Components.swift:78-89`, unused
   today) for SwiftUI, `ThreadRowView.snippetLine` for AppKit rows, and
   `CapsuleTabs` (`Components.swift:95-137`) for a selectable row of
   choices. Retire the Routines preview capsule chip
   (`RoutinesWindow.swift:417-419`) in favour of the same component.
4. **Confirm vs undo rule.** Write "act, then offer Undo; confirm only
   what cannot be undone", then apply it: confirm Discard draft, Delete
   leftover mail data and Delete routine, or make them undoable. Keep the
   undo notice for undoable actions only, and give errors and "Already
   sent" another surface (or hide the Undo button when there is nothing
   to undo). Task done and delete go through undo.
5. **Keyboard conventions.** Single keys in the focused list (lower case,
   no modifiers), ⌘ keys in menus, only while the owning window is key.
   Settle how help tags name keys (lower case single keys, or the menu
   key). Complete the shortcuts window. Reserve `t`, `⇧T` and the task
   list's reuse of e/r/a/f/c/⌫/↩.
6. **Avatars.** One initials and colour function shared by the reader
   HTML and `AccountAvatar` (same palette, hash and "Last, First"
   handling).
7. **Reader HTML tokens.** Map the reader and quote CSS to the scale
   (card radius 8 or document 12 as `Radius.panel`; padding 8/12 instead
   of 10/14; gap 8 instead of 10), or document the CSS as its own
   token set generated from `Space`/`Radius`.
8. **Status text.** Inline forms for error (red), caution (orange), and
   waiting or needs-you (yellow or attention), each with one font (`TypeRole.meta`
   or `caption`) and an optional symbol. Fix the orange errors and the
   orange "Waiting".
9. **Date formats.** A small `DateStyle` with the allowed styles (row,
   reader header, table timestamp, time-only, relative) and when to use
   each. No seconds in UI except diagnostics.
10. **Empty states in forms and lists.** When to use
    `ContentUnavailableView` and when to use a secondary line, and the
    wording without a full stop.
11. **Settings panes.** Grouped form, header case (title case), footers
    without an explicit secondary style, and destructive buttons behind a
    confirmation.
12. **Bars.** Name the bottom action bar (`RoutinesWindow.swift:307-345`)
    and the composer's top-rule bars (`ComposerView.swift:189,250,276`)
    as components, or fold them into `ListHeaderBar`/`Banner`.
13. **Accessibility and focus.** Row label format, custom actions
    mirroring keys, announcements for transient UI, button traits for
    tappable rows (fix `onTapGesture`-only rows), and initial focus in
    sheets and popovers.
14. **Motion tokens.** Durations such as 0.15 / 0.2 / 0.25 s as named
    values, always behind the Reduce Motion check.
15. **Data grids** (Sync Debugger, Keyboard Shortcuts). The label/value
    and table grid styles, and a fix for the shortcuts grid's 18/4 literals.

## Lint candidates

Rules a grep in `scripts/design-lint.sh` could enforce, outside `Design/`:

- **Stack spacing with a capital S.** `(horizontal|vertical)Spacing:
  *[1-9]` catches `KeyboardShortcuts.swift:75`, which the current
  `spacing:` pattern misses because it is case-sensitive.
- **Raw type roles.** `\.font\(\.(callout|headline|caption)\)` and
  `\.font\(\.title[23]?(\.weight|\))` should be `TypeRole.meta`,
  `.heading`, `.caption` and `.title`. There are 33 raw callout, 32 raw
  caption and 7 raw headline uses today, so start as a warning, or allow
  `// type:` for deliberate exceptions (`caption2`, monospaced).
- **Raw fonts in AppKit.** `systemFont\(ofSize:` outside `Design/` and
  the label factory in `ThreadRowView.swift:248-256`.
- **Colour literals.** `Color\.(red|orange|yellow|green)\b` and
  `\.foregroundStyle\(\.(orange|green|yellow)\)`, which push status
  colours into `Tone` (`Tone.failure`, `.caution`, `.approved`).
  `foregroundStyle(.red)` could stay allowed only on lines marked
  `// failure`.
- **Raw system colours in AppKit.** `NSColor\.system[A-Z]` and
  `\.backgroundColor = \.system` (swipe actions).
- **Opacity literals on fills.** `\.opacity\(0\.[0-9]+\)` in `.background`
  or `.fill`, where they belong in `Tone`.
- **Hand-drawn outlines.** `strokeBorder\(` and `RoundedRectangle\(cornerRadius`
  outside `Design/`, because only `.card(.attention)` may draw an outline
  (catches `AgentPanel.swift:335`).
- **Literal frame sizes.** `\.frame\((min|max|ideal)?[wW]idth: *[0-9]`
  (19 today), as a warning that asks for a named constant, per the
  "named constant in its view" rule.
- **Sheets without key equivalents.** Any file that contains `Button("Cancel"`
  but not `.cancelAction`, or a sheet view whose primary button has
  `.borderedProminent` but no `.defaultAction`.
- **Destructive button without a confirmation.** `role: .destructive` in
  a file with no `confirmationDialog`. This is a warning, and a
  `// undoable` mark exempts the line.
- **Help text ending in a full stop.** `hoverHelp\("[^"]*\.\"\)` (an
  addition to `help-lint.py`). The same rule could flag `.hoverHelp("")`.
- **Tooltips on menu-bar items.** `.hoverHelp(` inside
  `App/OpenAGCApp.swift` command groups, where it never shows.
- **Undo notice for errors.** `undo\.show\("Couldn` and similar: errors
  should not use the undo notice.
- **Date formatting outside the shared formatters.** `DateFormatter\(\)`,
  `RelativeDateTimeFormatter\(\)` and `time: \.standard` outside the
  designated file, once a `DateStyle` exists.
- **Tap-only rows.** `\.onTapGesture \{` without `.accessibilityAddTraits(.isButton)`
  or a `Button` within a few lines, which flags `RoutinesWindow.swift:425`.
- **Reader CSS numbers.** Grep the CSS string in `EmailDocument.swift`
  for `px` values outside {2, 4, 6, 8, 12, 16, 20, 24, 32} and the named
  sizes (avatar 26/32, font 11–14), so the HTML follows the same scale.
