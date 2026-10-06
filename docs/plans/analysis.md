# Plan: Analysis — learning from what you change in AI drafts, and Facts

Status: built (2026-10-05), epic oagc-259, branch `analysis`, PR
audiojak/openagc#9. Spec §14.10 (Analysis) and §14.11 (Facts); ADRs 0012
and 0013. Where the build departs from this plan, the spec says so:
settings are in Settings › Analysis (there is no Settings › Writing
Guide); the agent tool is `facts_lookup` (tool names allow no dots);
rejecting a proposal is "don't suggest this again"; fact proposals show
at once (one message stating a fact is enough) and a starter set after
three reviews; a global fact carries no evidence quotes.
Builds on the writing guide (spec §14.9, ADR 0011, plan `writing-guide.md`).

## Goal

The writing guide is learned once from sent mail and then changes only
when the user changes it. This feature keeps it learning. Once a day,
while the app is open, a background review compares what an AI wrote with
what the user actually sent, and proposes changes to the guide where the
user's edits show it is wrong or missing something. A second, optional
review gleans **facts** (about the user and their company) from sent
mail. Proposals wait in a new **Analysis** section in the sidebar, beside
Tasks, with a dot while there are new ones.

Facts move out of the writing guide into a section of their own, with
their own categories, and can be made global so every account uses them.

## Decisions (maintainer, 2026-10-05)

- **When:** once a day per account, in the background, only while the app
  is open (no launch agent, no server).
- **Who sees it:** hidden until the account has onboarded to the Writing
  Guide and finished one learning run. Before that, nothing records, runs
  or shows.
- **What is kept:** every AI composition leaves a record with its full
  text.
- **Matching:** a reply is compared with the next message the user sends
  in that thread. A new message is compared with the next message the user
  sends to that person, whatever its subject.
- **Comparing:** style differences against the guide's relevant entries,
  proposing to change, add or delete entries.
- **Queue:** "Analysis", in the sidebar next to Tasks, with an indicator
  (a dot) for proposals that are new since the user last looked, until
  they look.
- **Facts:**
  - **The option:** an option in Analysis to have the AI read all mail
    sent each day for new facts, not just AI-written mail.
  - **Their own place:** facts get their own interface and categories.
    "Facts about me" (guide category F3) moves there.
  - **Scope:** per account by default, made global in one step. Global
    facts are listed in Settings.

**Follow-up answers (maintainer, 2026-10-05):**

- **Daily time:** the first chance each day, once the first sync settles
  after the app opens.
- **Threshold:** two examples for a guideline, three for a rule. Weaker
  patterns wait under *Watching*.
- **Fact sources:** only mail the user sends (*Off*, *Mail written with
  AI*, *All mail I send*). Received mail stays out.
- **One queue:** the Writing Guide's Decisions from learning runs move
  into Analysis. Analysis is where every proposed change to the guide or
  facts waits. The Writing Guide is where the guide is read and edited.
- **Retention:** full texts are kept 30 days after review, adjustable to
  7 or 90 days.

## How it fits together

```
compose with AI ──► AI composition record (full text, thread, recipients)
                                    │
user edits and sends ──► sent message syncs in
                                    │
daily review (per account, app open)
   1. match records to sent messages
   2. compare each pair (style)       ──► guide proposals ─┐
   3. glean facts (AI pairs or all)   ──► fact proposals  ─┤
                                                           ▼
                                              Analysis queue (sidebar ●)
                                                           │
                                       accept / edit / reject (undoable)
                                                           ▼
                                           Writing Guide       Facts
```

## 1. Recording AI compositions

Nothing records an AI's text today. Writing help puts its text into the
draft body, and autosave overwrites it with the user's edits. Agent tools
save straight into `drafts.body_html`. The draft row is deleted once the
send is accepted. So every path must write a record when an AI produces
text, and the record must survive the send.

**Table `ai_compositions` (per account, new migration):**

| Column | What |
|---|---|
| `id` | |
| `created_at`, `updated_at` | |
| `source` | `writing_help`, `agent`, `routine` |
| `agent` | `claude-code`, `codex` |
| `kind` | `reply`, `forward`, `new` |
| `draft_id` | the local draft it went into |
| `thread_id`, `in_reply_to` | for replies and forwards |
| `recipients` | To and Cc, normalised addresses (JSON) |
| `subject` | as drafted |
| `instruction` | what the user asked for (writing help), or the agent prompt |
| `ai_text` | the full text the AI wrote, as plain text (and `ai_html`) |
| `guide_version` | the guide it was written under |
| `audiences` | the audience it was written for |
| `rfc822_message_id` | set when the draft is sent (the strong link) |
| `status` | `waiting`, `matched`, `unmatched`, `discarded`, `reviewed` |
| `matched_message_id`, `match_method` | `sent_draft`, `thread_next`, `recipient_next` |
| `sent_text` | the user's own text from the matched message (quotes and signature stripped) |
| `distance` | how much changed (normalised word edit distance, 0–1) |

**Where records are written:**

- **Writing help** (`ComposerAssistant.apply`): each text the agent writes.
  A rewrite for the guide's checks, or for another audience, updates the
  same record. The last AI text before the user's edits is what is
  compared.
- **Agent tools** (`create_draft`, `update_draft`, and replies through
  `reply_draft`): the agent's body, keyed by draft id. `update_draft`
  replaces the text.
- **Routines:** the same tools, with `source = routine`.
- **On send** (`send_draft` sets `drafts.rfc822_message_id`): the record
  copies the rfc822 id before the draft row goes. This gives an exact
  match when the sent copy syncs back.
- **On discard:** the record is marked `discarded`. That is a signal in
  itself (the draft was not used), counted in the metrics but not compared.

## 2. Matching

Run at the start of each daily review, for records still `waiting`.
Strongest first:

1. **Sent from the draft:** a sent message whose `rfc822_message_id` equals
   the record's. Exact.
2. **Reply:** the first message the user sends in `thread_id` after
   `created_at` (`SENT` label, from one of the user's addresses).
3. **New message (or forward):** the first message the user sends after
   `created_at` whose recipients include a recipient of the record (any
   overlap, To before Cc), whatever the subject. Forwards also try the
   thread first.

A sent message is matched to at most one record; the most recent record
wins. A record not matched within **14 days** becomes `unmatched` and is
dropped from review. Matches by rules 2 and 3 that look unrelated are
dropped too: under 15% word overlap with the AI text, after quotes and
signatures are stripped. A tiny reply such as "Thanks!" after a long AI
draft is a different message, not an edit.

The sent text is prepared as learning prepares it: quotes, forwarded
originals and the signature are stripped (`guide_learn::own_lines`,
`without_signature`).

## 3. The daily review (per account)

- **When:** the core's scheduler, which already ticks for routines,
  checks each open account once an hour. A review runs once per calendar
  day. It starts after the first sync of the day goes idle, or at the
  first opportunity once the user opens the app. Its state is in
  `analysis_runs`.
- **Gate:** the account has a finished learning run, and a connected
  agent. With no agent, the review waits and Analysis says why ("Connect
  Claude Code or Codex in Settings › Agents").
- **How it runs:** like a learning run (`guide_run`). The work is a job
  per account, in batches, in hidden read-only agent sessions (ADR 0007),
  pausable, and resumed after a relaunch. The two progress bars and the
  time-left estimate from the learning run are reused.
- **Work, in order:**
  1. Match (section 2).
  2. **Compare.** For each matched pair with `distance > 0.05`, in batches
     of 10 pairs. The prompt gives each pair's AI text and sent text
     (fenced), the user's instruction, the message type and audience, and
     the guide entries that applied to that message (rendered for its
     target, so scope is right). The agent answers with proposals in the
     change-by-prompt format (`add`, `edit`, `rescope`, `remove`), each
     with evidence. Evidence is a pair id, a quote from the sent text, and
     what it replaced in the AI text.
  3. **Glean facts** (section 5), if the facts option is on.
  4. **Merge proposals** across pairs and days. The same statement in the
     same category becomes one proposal whose evidence grows. A proposal
     the user rejected is not raised again, as in learning. A proposal
     that contradicts an accepted entry is marked so, as in decisions.
- **Thresholds:** a proposal needs two pairs of evidence before it shows,
  so one odd edit doesn't change the guide. A rule (not a guideline) needs
  three. Weaker ones wait, collecting evidence, and are listed under
  *Watching*.
- **Unchanged drafts count too.** A draft sent as written (`distance ≤
  0.05`) adds support to the entries that applied to it. Entries that are
  repeatedly overridden lose standing and can be proposed for removal.
- **Cost cap:** at most 50 pairs a day by default (Settings). Anything
  more waits for the next day, oldest first.
- **Run Now:** the same review on demand, from Analysis.

## 4. The Analysis section

A sidebar entry under Favorites, after Tasks and before Writing Guide:
**Analysis**. It appears when the account's first learning run finishes,
holding that run's decisions. Daily reviews start from then on.

It is the **one queue** for proposed changes:
- the decisions from learning runs, which move out of the Writing Guide;
- the daily review's guide proposals;
- fact proposals.

The Writing Guide keeps its progress bars and a link, "12 decisions
waiting in Analysis", but no queue of its own. The Writing Guide's badge
moves to Analysis.

- **Indicator:** a red dot while there are proposals created since the
  user last opened Analysis (`analysis_meta.last_viewed_at`, per account).
  It clears when they open it. The dot is an "unseen" signal, not a count,
  as asked. A number badge would duplicate the Writing Guide's.
- **List column:** four groups.
  - **From learning:** decisions from learning runs, in category order,
    as the Decisions screen shows them today. This is the same flow,
    moved: contradiction cards, *Use This Instead*, and the keys.
  - **Writing guide:** proposed changes to entries from daily reviews.
    Each shows the change (before → after, or "new" / "remove"), its
    category, and its strength ("seen in 4 replies").
  - **Facts:** proposed new, changed or removed facts.
  - **Watching:** proposals still short of the threshold, collapsed.
- **Detail:** for a guide proposal, the change and the evidence, as
  side-by-side snippets of what the AI wrote and what the user sent, the
  differing words marked. For a fact, the value and the quote it came
  from.
- **Actions:** the same keys as decisions: Return accepts, ⌫ rejects, e
  edits. Each is one change on the account's undo stack (ADR 0006), and
  accepted guide changes make a new guide version. *Accept All in Group*
  for the confident ones, also undoable as one change.
- **Header:**
  - when the review last ran;
  - what it examined (pairs, matched or unmatched);
  - the next run;
  - *Run Now* and *Pause*;
  - the progress bars while it runs.
- **Settings for Analysis** (in the section's header menu and in Settings
  › Writing Guide):
  - **Daily review** on/off.
  - **Learn facts from**, as a three-step control: *Off* · *Mail written
    with AI* · *All mail I send*. The default is *Mail written with AI*.
    This is the slider asked for. A segmented control reads better than a
    slider for three discrete steps.
  - **Pairs a day** (cost cap).
  - **Keep AI drafts for** 30 days by default (section 7).
- **Metrics, small and at the top:** how much AI drafts get changed, as a
  four-week trend (median `distance`), and how many were sent as written.
  This is the plain answer to "is the guide getting better?".

## 5. Facts

### What a fact is

| Field | What |
|---|---|
| `category` | a built-in or custom category (below) |
| `label` | short, unique within its category: "Title", "Calendar link" |
| `value` | the fact itself |
| `scope` | `account` (this account) or `global` (every account) |
| `use` | *Use freely* · *Ask before using* · *Never share* (default *Use freely*; People and the sensitive starter categories default to *Ask*) |
| `as_of` | when it was true. Some facts age (travel dates, a company's headcount), and old ones are flagged for review. |
| `source` | *You*, *Learned* (with evidence quotes), *Writing help* (answered a question) |
| `status` | `accepted`, `proposed`, `rejected` |

An account fact with the same category and label as a global fact
overrides it for that account.

### Categories

An account may be personal, for work, or both, so the built-in categories
are the few that fit anyone. Everything else is a **custom category**:
the user's own, or one added from a **starter set**.

**Built in (every account)**

| Key | Category | Labels it suggests |
|---|---|---|
| `identity` | Identity | Full name, Preferred name, Pronouns, Name pronunciation |
| `contact` | Contact | Phone, Other email addresses, Mailing address, Website or profiles |
| `availability` | Availability | Time zone, Usual hours, Calendar link, Where I usually am, Away or travel dates |
| `people` | People | People the user mentions: who they are to the user (partner, assistant, colleague, child's teacher) and how to refer to them. Default *Ask before using*. |
| `work` | Work | Occupation or role, Organisation, Team. Empty on a personal account, and then not shown. |
| `preferences` | Preferences | How the user likes to be reached or to meet ("phone over video", "no calls before 10"), and things to keep in mind when making plans |
| `other` | Other | Anything that fits nowhere else, until it is moved |

The list shows only categories that have facts. *Add Fact* offers them
all. Built-in categories can be hidden but not renamed or deleted, so the
interview, the migration from the guide and the gleaning prompt stay
stable.

**Custom categories**

- **What one has:** a name and a one-line description ("Properties I'm
  currently selling"). The description is what tells the gleaning prompt
  and the drafting agent what belongs there and when it matters.
- **Changing them:** they can be renamed, reordered and deleted.
  Deleting one moves its facts to Other, and can be undone.
- **Duplicates:** a new name close to an existing category ("Contact
  info") offers that category instead.
- **Scope:** a category belongs where it was made: an account or global.
  Making a fact global also makes its custom category global.
- **Export and merge:** custom categories travel with exported facts.
  Merging into another account creates any that are missing.

**Starter sets** (optional; each adds a few custom categories at once, which
the user can then edit like any other)

| Set | Categories it adds |
|---|---|
| Business | Company (name, what it does, founded, size, offices, website); Products and services; Customers and markets; Pricing and terms (*Ask*); Funding and investors (*Ask*); Policies and support (hours, response times, refunds, compliance); Approved wording (boilerplate, taglines, disclaimers; ties to guide rule F6); Links and resources |
| Freelance or consulting | Services and rates (*Ask*); Portfolio and references; Availability for new work |
| Household | Home (address details, service providers); Family logistics (school, activities; *Ask*); Health providers (names only, *Ask*) |
| Job search | Experience and skills; Roles I'm looking for; References (*Ask*) |

**Where starter sets and new categories come from:**
- **The user:** *Add Categories › From a Starter Set…* in the Facts tab.
- **Analysis:** the daily review proposes a new custom category when three
  or more facts in Other look alike. It proposes a starter set when
  gleaning keeps finding facts that would fit one, such as company facts
  on a work account. Both wait in the queue like any proposal.

What is never stored, even when found:
- passwords;
- card and bank numbers;
- government ids;
- health details about others;
- anything about third parties beyond their name, role and how the user
  knows them.

Gleaning drops these by pattern before a proposal is made, and the prompt
says so.

### Where facts come from

- **The user:** the Facts section's editor, and the interview (its F3
  questions become fact questions in Identity, Contact, Availability and Work).
- **Writing help's questions:** answers saved "to my writing guide" today
  become facts. The agent's question carries a category and label.
- **Gleaning** (the daily review, if on). From each day's AI-matched sent
  mail, or all mail sent that day, the agent extracts facts about the user
  and their company, each with a quote. A value that differs from an
  accepted fact becomes an *alter* proposal. A fact contradicted by recent
  mail can be proposed for removal.

### Facts in prompts

Facts render into the guide's "Facts about the user you may use" section
as today. *Use freely* facts are given as facts. *Ask before using* facts
are listed with "ask the user before using". *Never share* facts are left
out entirely. Global facts are merged under account ones. Facts are also
offered to the agent as a read-only tool (`facts.lookup`), so a long list
doesn't have to sit in every prompt.

### The Facts interface

- **Inside Analysis:** a *Facts* tab beside the proposals.
  - All facts by category, with a globe on global ones; custom categories
    after the built-in ones, in the user's order.
  - *Add Category…* and *Add Categories › From a Starter Set…*.
  - Proposed changes at the top.
  - Add, edit, delete, change *use*, and *Make Global* / *Make This
    Account's Only*. Each is one undoable change.
- **Settings › Facts:** the global facts list (as asked: "in the
  configuration, I should be able to see global facts"), with the same
  editing. A fact made global from an account moves there.
- **The Writing Guide's F3 category** becomes a pointer: "Facts now live
  in Analysis › Facts."

### Storage

- **Account facts:** a `facts` table in the account's store, with
  `fact_evidence` beside it.
- **Global facts:** a new store, `data_dir/global/facts.sqlite`, the same
  schema, opened once by the core. This is the first cross-account store.
  It needs **ADR 0012**, amending ADR 0004's "nothing is merged across
  accounts" for this one, explicit, user-chosen case. Undo across the two
  stores (*Make Global* moves a row) is one change recorded in the
  account's store, with both sides in its snapshot. Edits made in
  Settings › Facts, with no account in view, are recorded in the global
  store and undone there.
- **Categories:** custom categories in a `fact_categories` table beside
  `facts` (name, description, order, hidden built-ins), in whichever store
  they belong to.
- **Moving existing facts:** a migration moves accepted guide F3 entries
  ("Facts about me") into `facts`, following the interview's templates:
  - "My role: X" goes to Work › Role.
  - "My calendar link", time zone and working hours go to Availability.
  - Phone goes to Contact, and pronouns to Identity.
  - Anything else goes to Other. The guide entries are
  removed in the same transaction, with a guide version noting it.

## 6. Other capabilities worth including

- **Explain a proposal:** "Why?" opens the pairs behind it with the edits
  marked. The user decides on evidence, not on the agent's say-so.
- **Ignore this kind of edit:** on a rejected proposal, *Don't Suggest
  This Again* (as rejection already does for learning), plus *Ignore
  Edits to This Message* for one-off cases ("I rewrote it because the
  plan changed, not because of style").
- **Content edits are not style:** the compare prompt separates changes
  of fact or substance (a different date, a new paragraph about the plan)
  from changes of style. Only style goes to guide proposals. Substance
  feeds fact gleaning, when it is about the user or the company.
- **Per-entry health in the guide:** each entry shows how often drafts
  that applied it were sent unchanged or overridden. This comes from the
  same data and helps the user prune.
- **Notification (opt-in):** "3 new proposals in Analysis" once a day,
  only when the app is not in front. Off by default; the dot is enough.
- **Multiple accounts:** reviews run per account. The dot shows for the
  open account. The account menu shows a small dot on accounts with
  unseen proposals.
- **Archived-mailbox accounts:** never record or review (they cannot
  compose).
- **Export:** the Facts list exports to Markdown and JSON, as the guide
  does, and can be merged into another account's facts.

## 7. Privacy and safety

- **Disclosure:** recording is local. AI texts sit in the account's store.
  The daily review sends matched pairs, and with *All mail I send* the
  day's sent mail, to the user's own agent CLI. That is the same
  disclosure as learning, and Analysis says so before the first run.
- **Retention:** full AI texts and sent texts in `ai_compositions` are
  kept for 30 days after review (Settings: 7, 30 or 90 days). After that
  only the distance and the proposal links remain.
- **Prompt safety:** mail is fenced (`<` → `‹`) and evidence quotes are
  verified against the cited text, as in learning. Gleaned facts are
  checked against the never-store patterns before they reach a proposal.
- **Guardrails:** nothing changes the guide or facts without the user
  accepting. Every change is undoable and versioned.
- **No new permissions:** reviews use hidden read-only sessions (ADR
  0007). They cannot draft, send or change mail.

## Storage and API (summary)

- **Migrations (per account):**
  - `ai_compositions`;
  - `analysis_runs` (as `guide_runs`, kind `daily`);
  - `analysis_proposals` (guide changes awaiting a decision, with their
    evidence pairs, or reuse `guide_entries` with `status = proposed` and
    `origin = analysis`);
  - `analysis_meta` (`last_viewed_at`, settings);
  - `facts` and `fact_evidence`;
  - moving F3 facts.
- **Global store:** `data_dir/global/facts.sqlite` (ADR 0012).
- **Core FFI:**
  - `record_ai_composition`, `composition_sent`, `composition_discarded`;
  - `start_analysis_run`, `analysis_progress`, `pause` / `resume`;
  - `analysis_proposals`, `decide_analysis_proposal`;
  - `analysis_seen`, `analysis_unseen_count`;
  - `list_facts(scope)`, `apply_fact_edits` (undoable),
    `make_fact_global` / `make_fact_local`;
  - `analysis_settings` / `set_analysis_settings`;
  - `analysis_metrics`.
- **Events:** `AnalysisProgress`, `AnalysisChanged`, `FactsChanged`.
- **Fake agent answers** keyed on prompt markers ("OpenAGC analysis
  compare", "OpenAGC facts glean"), so the whole flow is tested without
  real agents.

## Issues, in order (epic oagc-259)

| # | Issue | Notes |
|---|---|---|
| 1 | Spec §14.10 Analysis and §14.11 Facts; ADR 0012 global facts store; ADR 0013 recording AI compositions and retention | Decisions first |
| 2 | Store: `ai_compositions`, recording from writing help, agent tools and routines; rfc822 id copied on send; discard marks it | Tests with the fake agent and demo sends |
| 3 | Matching (sent-from-draft, thread-next, recipient-next, 14-day expiry, unrelated guard) | Pure function plus store queries; table-driven tests |
| 4 | Daily scheduler and the analysis run (gate, once a day, pause, resume, relaunch, cost cap, Run Now) | Reuses the routine tick and `guide_run` patterns |
| 5 | Compare prompt, parser and merge into proposals (thresholds, Watching, reinforcement from unchanged drafts) | Fake agent `compare_answer` |
| 6 | Analysis section: sidebar entry, unseen dot, list, detail with evidence, keys, undo, header, metrics; learning Decisions move in (Writing Guide keeps a link) | Light and dark snapshots |
| 7 | Facts store, built-in and custom categories, starter sets; migration from guide F3; rendering by *use*; `facts.lookup` tool | |
| 8 | Global facts: store, Make Global or Local, overrides, Settings › Facts | ADR 0012 |
| 9 | Fact gleaning (AI-only or all sent mail) into built-in and custom categories by their descriptions; never-store filter; alter and remove proposals; new-category and starter-set proposals; `as_of` staleness | Fake agent `glean_answer` |
| 10 | Facts tab UI with category editing and starter sets; interview and writing help write facts; Writing Guide F3 pointer | |
| 11 | Settings: daily review, learn facts from, pairs a day, retention; opt-in notification; account-menu dots | |
| 12 | Retention purge and export of facts; review pass; handoff | |
