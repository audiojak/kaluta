# A design system that is Kaluta's own

Part 3 of the review (`docs/plans/ux-review-2026-11.md`). What makes
Kaluta look like any Mac app today, what is already its own, three
directions it could take, and a recommendation.

| Direction | Page |
|---|---|
| 1. Paper | [direction-1-paper.html](mockups/direction-1-paper.html) |
| 2. Signal | [direction-2-signal.html](mockups/direction-2-signal.html) |
| 3. Daylight | [direction-3-daylight.html](mockups/direction-3-daylight.html) |

Each page has a style tile (type, colour and shape, voice) and the same
two screens drawn in it: the Dashboard and the mail window. Buttons
switch light and dark and the window's width.

## Kaluta has two faces today

**The mail window is the system's.** System type at system sizes,
system colours, the user's accent colour for unread, glass, the
platform's toolbar and split view, title-case empty states. This is
`docs/design-system.md` working as written: principle 1 is *the system
first*, and the thread row is described as "calm, as in Mail". Put
beside the platform's own mail app, little tells them apart (compare
`img/main-light.png`).

**The Dashboard and workflows are Kaluta's.** Type at 34 and 40 points,
a colour per workflow, keys drawn on every button, one pill, a ring and
a chord at the end, sentences with full stops ("Everything new is
decided."). Nothing else on the platform looks like
`img/wf-triage-light.png`.

So the question is less "what should Kaluta look like" than "which of
its two faces is it". The user moves between them many times a day
(Esc, M), and the change is abrupt: different type scale, different
colour rules, different voice.

### Already Kaluta's own

- **Keys on the controls** (`Keycap`, `KeyHints`): the most recognisable
  thing in the app.
- **A colour per workflow** (`Tone.workflow`).
- **One sentence that says where things stand**: the headline, the end
  line, the pill.
- **The name.** The AI is Kaluta, it proposes, the user decides.
- **Act, then Undo**: no confirmation dialogs anywhere.

### Generic, or working against it

- The mail window's type, colour and density are the platform's.
- The assistant is called by its backend's name in about a dozen places
  (finding F5): the product's own character is replaced by someone
  else's exactly where the user talks to it.
- Two voices: sentence case with a full stop in workflows, title case
  in the mail window's empty states (F22).
- The same place is "Mail", "the start" and "Dashboard" (F4).
- Icons carry most of the mail window's controls, and they are the
  system's icons (F12).

## Where distinctness is safe, and where it costs

**Keep from the system, whatever is chosen:** how things behave. Menus
and their order, standard keys, focus and Full Keyboard Access, window
and toolbar behaviour, sheets, VoiceOver, Increase Contrast, Reduce
Motion, larger text. These are what make an app feel right on the Mac
and none of them is what makes it look generic.

**Open to being Kaluta's:** type (faces and scale), colour (ground, ink,
one accent, the workflow colours), shape (corner radii, rules, the
weight of cards), density, motion and sound, illustration and empty
states, and above all the words.

**What it costs:**

- **The user's accent colour.** Today unread follows it. A Kaluta
  accent means either ignoring the system setting or keeping two
  accents. Recommended: Kaluta's accent for Kaluta's own marks (unread,
  the primary button, suggestions), the system's for selection and
  focus rings, which the system draws.
- **HTML mail is designed for white.** Any tinted ground needs the
  message itself on a white (or true dark) card.
- **Every colour needs four proofs:** light, dark, and each with
  Increase Contrast, at 4.5:1 for text.
- **Custom faces need care at larger text sizes;** the three directions
  use only faces that ship with the system (the system's serif, its
  rounded and its monospaced), so nothing is bundled or licensed.

## The three directions

### 1. Paper

A reading room. A serif for subjects, headlines and the message; warm
paper for the ground; ink, not black; hairlines, not boxes; one deep
green for everything Kaluta does.

- **Says:** mail is reading and writing, and this is a good place to do
  both.
- **For it:** distinct at a glance with no custom control; makes long
  mail pleasant; unlikely to date.
- **Against it:** a serif in lists can feel bookish; furthest from the
  system's look, so sheets and menus contrast with it; the paper ground
  needs a white card for HTML mail.

### 2. Signal

An instrument. Keys, counts and times in a monospaced face; tight rows;
strong contrast; square corners; one electric colour for the next thing
to do.

- **Says:** this is the fast way through mail.
- **For it:** fits what the product is (keys and counts everywhere);
  the densest; the strongest in dark.
- **Against it:** the look most fast, keyboard-first tools have chosen,
  so the least distinct among them; cold beside the tone the workflows
  have now; small monospaced text tires.

### 3. Daylight

A companion. The workflow colours come into the mail window as meaning
(answer, decide, tasks, waiting); rounded type; soft corners; a
greeting for the time of day.

- **Says:** Kaluta is on your side and keeps track.
- **For it:** grows from what exists, so the cheapest; colour means the
  same thing in the list, the workflow and the Dashboard.
- **Against it:** five colours that carry meaning are hard to keep
  accessible and must never be the only signal; soft and rounded can
  read as less serious; competes with the mail service's label colours.

## Recommendation

**Paper as the base, keeping the keycaps and the workflow colours.**

Reasons:

1. **It is distinct where it is cheap.** Type and ground colour change
   every screen at once and touch no behaviour. The system stays in
   charge of everything the platform guidelines care about.
2. **It joins the two faces.** The workflows' large type becomes the
   same serif the mail window uses for subjects, so moving between them
   is a change of scale, not of personality.
3. **It suits the product's claim.** Kaluta proposes and the user
   decides, one thing at a time, with the evidence. That is closer to a
   well-edited page than to an instrument panel.
4. **The other two survive inside it.** Signal's keys are already
   Kaluta's and stay (in the monospaced face, on paper). Daylight's
   workflow colours stay as the only strong colour, slightly muted to
   sit on the warm ground. What is dropped is Signal's coldness and
   Daylight's roundness.

This is a judgement of taste as much as of usability, and it is the
maintainer's to make. If Paper feels too far, **Daylight is the safe
choice** (it is the present workflows' look, extended to the mail
window) and most of the adoption steps below are the same.

### What changes in `docs/design-system.md`

- **Principle 1** becomes: *The system's behaviour, Kaluta's face.*
  Behaviour, controls, accessibility and window structure are the
  platform's. Type, colour, shape and words are Kaluta's, and only
  through tokens.
- **A new principle:** *One voice.* Sentence case; a full stop on a
  sentence; Kaluta by name; no title-case headlines; the same word for
  the same place everywhere.
- **Tokens added:** `Tone.ground`, `Tone.surface`, `Tone.ink`,
  `Tone.ink2`, `Tone.rule`, `Tone.kaluta` (the accent); `TypeRole`
  gains a display family used by `title`, `display`, `reading` and the
  workflow roles, and `rowSubject` moves to it.
- **Tokens changed:** `Tone.unread` from the system accent to
  `Tone.kaluta`; `Radius.card` from 8 to 4; cards lose their fill where
  a hairline will do.
- **Unchanged:** spacing, the workflow type scale, `Keycap`,
  `KeyHints`, the undo notice, banners by intent.

### Order to adopt it in

Each step is shippable and useful without the next.

1. **Words** (no visual change): Kaluta by name everywhere the user
   asks it something (F5); one name for the Dashboard (F4); sentence
   case in empty states (F22). This is the cheapest way to make the
   app more its own and it fixes two major findings.
2. **Tokens:** add the ground, ink and accent tokens with today's
   values, and move views onto them. No visible change; makes the rest
   a matter of changing values.
3. **Type:** the display family for subjects, the reader's subject and
   the workflow headline. The first visible step, and reversible by one
   token.
4. **Ground and accent:** paper and ink, the Kaluta accent, the white
   card for HTML mail. Checked in light, dark and Increase Contrast.
5. **Shape and density:** radii, hairlines for card fills, row
   padding.
6. **The rest:** empty-state drawings, the icon, sound.

Steps 3 to 5 are best tried behind a debug setting and looked at for a
week before any is made the default.

## Not settled here

- Whether message text itself should be set in the serif. It is drawn
  that way in the mock-up; plain-text mail suits it, HTML mail brings
  its own type.
- A custom (non-system) face. None is proposed; one could replace the
  system serif later without changing anything else.
- The app icon and the name's lettering.
