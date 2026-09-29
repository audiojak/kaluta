# Plan: a writing ruleset and style guide, learned from sent mail

Status: planning (2026-09-29). Not started. Branch `writing-guide`.

## Goal

Whenever an AI composes email for the user (a new message, a reply or a
forward; from the composer's writing help, the agent column or a routine),
it follows a **ruleset** and a **style guide** the user has set. Both are
built interactively: the app gathers the user's past sent mail, processes
it in batches into proposed rules and guide entries with evidence, and the
user accepts, edits or rejects each one.

## Decisions (maintainer, 2026-09-29)

- **Source:** sent mail (the user's own text: quoted replies, forwarded
  originals and signatures stripped).
- **Scope:** one guide per account, stored in the account's store
  (ADR 0004).
- **Applies:** everywhere an AI composes email: new drafts, replies and
  forwards, in the composer, the agent column and routines.
- **Review:** batches with merged proposals; each proposal shows example
  quotes from the user's mail; accept, edit or reject one by one.
- **Audience groups:** inferred from the mail and confirmed by the user.
  With fewer than five, the set is filled from the obvious gaps (from:
  colleagues, direct reports, customers, investors, vendors, candidates,
  advisers such as lawyers, friends and family, strangers), each marked
  "suggested" until confirmed.
- **Checks** run only on what an AI drafts, never on what the user types.
  When the user writes a prompt in the writing help box, their draft is
  part of the prompt; the message the AI writes from it follows the guide
  and is checked.
- **One guide for every agent** (Claude, Codex): confirmed.
- **Categories:** the maintainer's second list (26 categories, 2026-09-29)
  was compared with this one; ten were added and one broadened, marked
  "added" below. The rest were already covered under other names.

## Two kinds of entry

- **Rule**: must or must never. Checkable, no exceptions unless the rule
  states them ("Never promise a delivery date", "Sign off with 'John'").
- **Guideline**: how the user usually writes; the agent follows it unless
  the message calls for something else ("Replies are usually under 80
  words").
- **Fact**: something true about the user the agent may use (title,
  calendar link, time zone). Never inferred silently: always confirmed.

## Categories

The processing function checks every batch against this whole list, so
nothing depends on Claude thinking of a category. "Learned" means sent
mail can show it; "asked" means it cannot, and the interview (below) asks.

### A. Voice and tone
| # | Category | Examples of entries | From |
| --- | --- | --- | --- |
| A1 | Overall voice | formality, warmth, directness, confidence vs hedging | learned |
| A2 | Tone by situation | saying no, bad news, apologising, asking a favour, chasing, thanking, disagreeing, congratulating | learned |
| A3 | Humour, emoji, exclamation marks | whether, how much, with whom | learned |
| A4 | Directness (added) | "Can you send this by Friday?" vs "Would you be able to…"; how plainly requests are stated | learned |
| A5 | Uncertainty in your voice (added) | "I think…", "My understanding is…"; when not to sound certain | learned |
| A6 | Enthusiasm and acknowledgements (added) | "Great", "Sounds good", "Perfect", "Love it"; how often | learned |
| A7 | Personality markers (added) | regional words (Australianisms), colloquialisms, lowercase replies | learned |

### B. Structure
| # | Category | Examples | From |
| --- | --- | --- | --- |
| B1 | Greeting | "Hi Ann," / first name only / none in a running thread | learned |
| B2 | Opening line | straight to the point vs a pleasantry; "hope you're well" or never | learned |
| B3 | Body | answer first; paragraph length; bullets, numbers, bold | learned |
| B4 | Length | typical length by message type | learned |
| B5 | Closing line | next step, "let me know", none | learned |
| B6 | Sign-off and name | "Best," "Thanks," none; "John" / "JK" | learned |
| B7 | Signature block | when it is included; which one | learned + asked |
| B8 | Subject lines | style for new messages; when to change one | learned |
| B9 | Context (added) | how much background before the point; what recipients are assumed to know | learned |
| B10 | Calls to action (added) | where the ask goes; explicit deadlines; one clear next step | learned |
| B11 | Questions (added) | one at a time or several; inline or bulleted; open or specific | learned |

### C. Language
| # | Category | Examples | From |
| --- | --- | --- | --- |
| C1 | Spelling variant | US / UK | learned |
| C2 | Punctuation | serial comma, dashes, ellipses | learned |
| C3 | Capitalisation | product names, titles, headings | learned |
| C4 | Contractions | "I'll" vs "I will" | learned |
| C5 | Numbers, dates, times, money | "3pm PT", "Oct 2", "$5k" | learned |
| C6 | Abbreviations and jargon | which are used, with whom | learned |
| C7 | Favoured words and phrases | phrases the user really uses | learned |
| C8 | Things you never do (broadened) | banned words, phrases and clichés; formatting never used; AI habits to avoid | learned (absence) + asked |
| C9 | Sentence style | length, fragments, active voice | learned |
| C10 | Languages | which language to answer in | learned |

### D. Audience
| # | Category | Examples | From |
| --- | --- | --- | --- |
| D1 | Audience groups | colleagues, customers, investors, vendors, recruiters, friends and family: register for each | learned + asked |
| D2 | Particular people | nickname, formality, things to remember for one person or domain | learned |
| D3 | Forms of address | first names, titles | learned |
| D4 | First contact vs established | how a cold or first message differs; warm vs transactional | learned |
| D5 | Seniority (added) | senior, peer, direct report | learned + asked |

### E. Message types (playbooks)
| # | Category | Examples | From |
| --- | --- | --- | --- |
| E1 | Replies | inline answers vs a fresh note; quoting | learned |
| E2 | Forwards | the note on top: "FYI", a summary, an ask | learned |
| E3 | Introductions | double opt-in, moving the introducer to Bcc | learned |
| E4 | Scheduling | how times are offered, time zone, calendar link | learned + asked |
| E5 | Follow-ups and chasers | after how long, how firm | learned |
| E6 | Declines | how to say no | learned |
| E7 | Requests and delegating | how asks are phrased, deadlines | learned |
| E8 | Status updates and hand-offs | shape, headings | learned |
| E9 | Thanks and acknowledgements | one line or more | learned |
| E10 | Recipients | reply all habits, who is copied, Cc vs Bcc | learned |
| E11 | Attachments and links | how they are mentioned | learned |
| E12 | Disagreeing and negotiating (added) | correcting, pushing back, negotiating | learned |

### F. Content rules
| # | Category | Examples | From |
| --- | --- | --- | --- |
| F1 | Commitments | never promise dates, prices, legal terms without the user | asked |
| F2 | Confidentiality | topics and figures never to mention, or only to some audiences | asked |
| F3 | Facts about me | role, company, phone, calendar link, time zone, working hours, pronouns | asked (suggested from mail) |
| F4 | Never invent | no made-up facts, names, figures; what to do instead | asked (default on) |
| F5 | AI disclosure | whether a message may say an AI helped | asked |
| F6 | Required wording | legal or compliance text for some audiences | asked |

### G. Format
| # | Category | Examples | From |
| --- | --- | --- | --- |
| G1 | Plain or rich text | links as text or URLs, bold, lists | learned |
| G2 | Quoting | trimming quoted text, answering inline | learned |

### H. When the agent is unsure
| # | Category | Examples | From |
| --- | --- | --- | --- |
| H1 | Missing information | ask me, leave a [bracket], or offer options | asked |
| H2 | Conflicts and precedence | rules beat guidelines; a person's entry beats a group's | fixed, shown to the user |
| H3 | Model examples | real sent messages kept as examples per message type | chosen by the user |
| H4 | Draft, send or stay silent (added) | when to only draft, when to ask first, when not to reply at all | asked |

H4 is guidance to the agent; what an agent is *able* to do without
approval stays with Settings › Permissions, which the guide cannot loosen.

## An entry

`id`, `category` (A1…H3), `kind` (rule, guideline, fact), `statement`
(one sentence, imperative), `scope` (any of: audience group, person or
domain, message type, language; empty = always), `evidence` (quotes with
message ids, and a count of messages that support and contradict it),
`source` (learned, you), `status` (proposed, accepted, rejected), optional
`check` (a banned phrase or pattern the app can test without an agent),
times. A rejected proposal is remembered so it is not proposed again.

## The process

1. **Gather.** "Learn from Sent Mail…" picks a sample from the account's
   sent mail: the user chooses how far back and how many (default the
   latest 200), and may exclude people or labels. The sample is spread
   across audiences and message types (new, reply, forward) so every
   category has material. Skipped: automatic replies, calendar responses,
   messages with no text of the user's own. Header-only messages download
   first (ADR 0003).
2. **Prepare** (core, no agent): the user's own text only (`strip_quoted`,
   signature removed and kept once for B7), with what kind of message it
   is, who it went to (by audience group once known), length, and whether
   it answered someone.
3. **Process** (the processing function): batches of about 20 messages go
   to Claude in a read-only session (ADR 0007) with the whole category
   list and the guide so far. Claude answers in JSON, per category: a new
   entry, more evidence for an entry, a contradiction of an entry, or
   nothing found. The core parses leniently, keeps only known categories,
   requires each quote to really occur in the cited message, and merges
   across batches.
4. **Review.** Proposals are shown by category with their quotes and
   counts. Accept, edit (statement, kind, scope) or reject. Contradictions
   of accepted entries are shown as "your mail disagrees: narrow the rule,
   change it, or keep it".
5. **Interview.** For categories mail cannot show (F, H, parts of B7, C8,
   D1, E4), and for categories with no evidence, the app asks short
   questions; answers become entries with source "you".
6. **Coverage.** The guide shows every category with its entries, or
   "nothing yet", so gaps are visible. Learning can be run again later on
   newer mail; only new messages are processed.

## Following the guide

- The core renders the accepted entries as text: rules and facts always;
  guidelines filtered by scope to the message at hand (its recipients'
  groups and people, its type, its language); up to three model examples
  of the same type.
- Every composing path gets it: the composer's writing help, and any agent
  or routine session, through the session's system prompt and again in the
  result of the draft tools (`mail_create_draft`, `mail_update_draft`,
  forward and send), so a long conversation does not lose it.
- **Checks.** Entries with a `check` are tested by the core on every AI
  draft, and only on AI drafts: text the user types is never checked. In
  writing help the user's own draft goes in as part of the prompt; the
  message that comes back is what is checked. A failure is shown on the
  draft ("Uses 'circle back', which your
  rules ban") and, in writing help, sent back once for a rewrite.
- Drafts written under a guide say which version they used.

## Storage and API

- Migration `0010_writing_guide` in the account's store: `guide_entries`,
  `guide_evidence`, `guide_runs` (which messages were processed),
  `guide_examples`.
- Core: list, accept, edit, reject, add, delete entries; `guide_sample`,
  `guide_prompt(batch)`, `parse_guide_proposals`, `merge`, `render_guide(for
  message)`, `check_draft`. Export and import as Markdown.
- The fake agent answers guide prompts with fixed JSON, so every path is
  tested without a real agent. Nothing connects to Gmail in tests.

## UI

- **Writing Guide** window (Window menu, and Settings › Writing opens it):
  categories in a sidebar with counts, entries in the list, evidence and
  scope in the detail; add, edit, delete; coverage at a glance.
- **Learn from Sent Mail…**: a dialog for the sample, progress by batch
  (cancellable, resumable), then the review.
- The composer's writing help bar shows "Following your writing guide"
  with a link to it; a draft that fails a check says why.

## Privacy

Learning sends the chosen sent mail to the user's own Claude (or Codex)
CLI, as the agent column already does with mail it reads. The dialog says
so before it starts, with the number of messages. Sessions are read-only;
mail in prompts is fenced. The guide and its evidence stay in the
account's store on this Mac.

## Issues, in order

1. Spec §14.9 and ADR 0011: the taxonomy, entry schema and precedence.
2. Store and core API (migration, CRUD, render, Markdown export/import).
3. Gather and prepare: sample selection, own-text extraction, signature
   detection.
4. The processing function: prompt, parser, quote verification, merge;
   the fake agent's answers.
5. Review UI: proposals by category, accept, edit, reject, contradictions.
6. Writing Guide window and the interview for what mail cannot show.
7. Following the guide: system prompt, draft tools, routines, composer.
8. Checks on AI drafts and the rewrite loop.
9. Re-learning from newer mail; guide versions.

## Open questions

- The default sample: latest 200 sent messages in batches of 20?
