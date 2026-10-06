# Plan: proposed rules in the Writing Guide, a Facts page, no Analysis page

Status: built (2026-10-06), branch `feedback`. Known issue: oagc-9hf (Facts rows clipped, also on main).

## Why

The writing guide is a set of rules, and every decision the user makes,
from learning or from the daily review, is a decision about a rule. Those
decisions belong in the guide, not in a separate Analysis page. Facts get
a page of their own with the same layout.

## Decisions (maintainer, 2026-10-05)

- **No Analysis page.** The sidebar loses Analysis and gains **Facts**,
  under Writing Guide.
- **Writing Guide list:** a **Proposed** section at the top with one row
  per proposed rule, from learning and from the daily review alike
  (newest first, *Accept All* on the section), then **Watching**
  (collapsed), then the categories as before. Selecting a proposed rule
  shows it in the detail with its evidence (the learning quotes, or the
  AI-drafted and sent pairs). Return accepts, ⌫ rejects, e edits.
- **The daily review lives in the Writing Guide's header**: Run Now,
  Pause, progress, the metrics line and the settings button, under the
  learning controls. The Facts header has one line saying when facts were
  last looked for.
- **Facts page:** the same layout: header (Add Fact…, Categories menu, the
  learning line), a **Proposed** section (one row per proposed fact,
  selectable, with Accept / Reject and the use choice), then facts by
  category. Selecting a proposed fact shows it in the detail.
- **Dots and badges:** Writing Guide shows the count of proposed rules
  and a dot while one is new; Facts the same for proposed facts. Seen is
  tracked per page (core: `analysis_seen(target)`), so opening one page
  does not clear the other's dot.
- **Settings › Analysis is renamed Learning.** The settings sheet from the
  header is *Learning Settings*.
- The notification for new proposals opens the page they are for (Writing
  Guide when there are new rules, else Facts).

## Revised after stepping through (2026-10-06)

With 16 proposed rules the Proposed section pushed the guide's categories
off screen. The maintainer chose a review flow instead (oagc-bm2): the
middle column shows only categories (facts); its header has *Review N
Proposed Rules* (*Review N Proposed Facts*), which fills the detail with
the proposals as cards, one current, Return / ⌫ / e / j / k, *Accept All*
at the top, Watching folded at the end. The learning progress bars show
only while a run is going.

## Steps

1. Core: per-page seen (`analysis_seen` takes a target; the queue says
   which page has something new). Tests.
2. Writing Guide: Proposed and Watching sections, proposal and decision
   detail, keys, the review in the header; remove the "decisions waiting
   in Analysis" link and the learning prompt's jump to Analysis.
3. Facts page: sidebar entry, header, Proposed rows selectable, detail.
4. Remove the Analysis page (route, views, snapshot flags, tests moved to
   the two pages); rename Settings › Analysis to Learning.
5. Spec §14.9 to §14.11, design-system snapshots, the plan's status.
