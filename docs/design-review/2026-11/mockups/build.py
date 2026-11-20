#!/usr/bin/env python3
"""Writes the review's mock-ups (docs/plans/ux-review-2026-11.md, parts 2
and 3): one self-contained HTML page per mail window layout and per
design direction, with the demo mailbox's content. Drawings to choose
from, not prototypes: the only script on a page switches light and dark,
the window's width, and which state is shown.

Run: python3 docs/design-review/2026-11/mockups/build.py
"""
from pathlib import Path

OUT = Path(__file__).parent

# The demo mailbox (crates/kaluta-core demo seed): (sender, subject, preview, time, unread, chip, group)
THREADS = [
    ("Taylor Chen", "Interview feedback for the designer role",
     "Strong portfolio and great with the team; a little light on systems work. I'd lean towards an offer.",
     "7:17 PM", True, None, "answer"),
    ("Avery Kim", "Launch date — go or no-go?",
     "QA signed off this morning. The only open item is the pricing page copy. Are we go for Tuesday?",
     "9:17 PM", True, None, "answer"),
    ("Emerson Silva", "Onboarding plan",
     "I've attached the latest version for your review. Can we find thirty minutes on Thursday?",
     "4:02 PM", False, None, "answer"),
    ("Reese Schmidt", "Customer escalation",
     "Following up on our conversation from last week. The numbers look better than we expected.",
     "2:40 PM", True, "Customers", "decide"),
    ("Logan Schmidt", "Security review",
     "I'll send an updated draft once the team has weighed in. Let me know if anything is unclear.",
     "1:15 PM", False, "Customers", "decide"),
    ("Alex Kim", "Contract review",
     "Thanks again for the quick turnaround. Adding a couple of people who should be in the loop.",
     "11:30 AM", False, None, "waiting"),
    ("Skyler Rivera", "Q3 planning",
     "I'll send an updated draft once the team has weighed in. I've attached the latest version.",
     "Yesterday", False, None, "waiting"),
    ("Weekly Digest", "This week in design systems",
     "Tokens that travel, a field guide to density, and what a calm interface costs.",
     "Yesterday", False, "Newsletters", "read"),
    ("Status Alerts", "Your invoice for November",
     "Your invoice is ready. Amount due: $48.00. It will be charged on December 1.",
     "Yesterday", False, "Receipts", "read"),
    ("Careers", "3 new applicants for Product Designer",
     "Three people applied since Monday. Review them before Friday's sync.",
     "Nov 18", False, "Hiring", "read"),
]

MESSAGE = """<p>Hi Jo,</p>
<p>Strong portfolio and great with the team; a little light on systems work. I'd lean towards an offer.
Do you want to meet them once more, or should I start on the offer?</p>
<p>Taylor</p>"""

SUGGESTION = "Kaluta suggests a quick reply · about 20 seconds"

BASE_CSS = """
*{box-sizing:border-box;margin:0}
:root{
  --page:#e9e9ec; --ink:#1d1d1f; --ink2:#6e6e73; --ink3:#a1a1a6;
  --win:#ffffff; --side:#f2f2f4; --line:#e3e3e6; --sel:#e4e4e8; --card:#f5f5f7;
  --accent:#0a66d8; --accent-ink:#ffffff; --unread:#0a66d8;
  --ok:#2f7d6d; --warn:#b4691f; --chip:#e8e8ed; --shadow:0 18px 50px rgba(0,0,0,.18);
  --font:-apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif;
  --display:var(--font); --mono:ui-monospace,"SF Mono",Menlo,monospace;
  --radius:8px; --row-pad:10px;
}
:root[data-theme=dark]{
  --page:#111113; --ink:#f2f2f4; --ink2:#a1a1a8; --ink3:#6c6c73;
  --win:#1c1c1f; --side:#232326; --line:#323236; --sel:#34343a; --card:#27272b;
  --accent:#4c9dff; --accent-ink:#06121f; --unread:#4c9dff;
  --ok:#5fc2ad; --warn:#e0a060; --chip:#34343a; --shadow:0 18px 50px rgba(0,0,0,.6);
}
@media (prefers-color-scheme: dark){ :root:not([data-theme=light]){
  --page:#111113; --ink:#f2f2f4; --ink2:#a1a1a8; --ink3:#6c6c73;
  --win:#1c1c1f; --side:#232326; --line:#323236; --sel:#34343a; --card:#27272b;
  --accent:#4c9dff; --accent-ink:#06121f; --unread:#4c9dff;
  --ok:#5fc2ad; --warn:#e0a060; --chip:#34343a; --shadow:0 18px 50px rgba(0,0,0,.6);
}}
body{background:var(--page);color:var(--ink);font:13px/1.45 var(--font);padding:28px 16px 60px;-webkit-font-smoothing:antialiased}
.intro{max-width:1200px;margin:0 auto 18px}
.intro h1{font:600 22px/1.2 var(--font);margin-bottom:6px}
.intro p{color:var(--ink2);max-width:780px;font-size:14px}
.pros{display:grid;grid-template-columns:1fr 1fr;gap:6px 28px;max-width:900px;margin-top:12px;font-size:13px}
.pros h2{font:600 11px/1 var(--font);letter-spacing:.06em;text-transform:uppercase;color:var(--ink2);margin-bottom:4px}
.pros li{margin-left:16px;color:var(--ink)}
.controls{display:flex;gap:8px;flex-wrap:wrap;margin:14px 0 0}
.controls button{font:500 12px var(--font);padding:5px 12px;border-radius:999px;border:1px solid var(--line);background:var(--win);color:var(--ink);cursor:pointer}
.controls button[aria-pressed=true]{background:var(--ink);color:var(--win);border-color:var(--ink)}
.stage{overflow-x:auto;padding:6px 0 30px}
.window{width:1200px;height:760px;margin:0 auto;background:var(--win);border-radius:12px;box-shadow:var(--shadow);overflow:hidden;display:flex;flex-direction:column;border:1px solid var(--line);position:relative;transition:width .25s}
.narrow .window{width:820px}
.bar{height:52px;flex:none;display:flex;align-items:center;gap:14px;padding:0 16px;border-bottom:1px solid var(--line)}
.lights{display:flex;gap:8px;margin-right:6px}.lights i{width:12px;height:12px;border-radius:50%;background:var(--ink3);opacity:.55}
.bar .title{font-weight:600;font-size:14px}.bar .sub{color:var(--ink2);font-size:12px}
.bar .grow{flex:1}
.tool{color:var(--ink2);font-size:12px;padding:4px 9px;border-radius:6px;white-space:nowrap}
.tool.primary{background:var(--accent);color:var(--accent-ink);font-weight:600}
kbd{font:500 10.5px var(--mono);border:1px solid var(--line);border-bottom-width:2px;border-radius:4px;padding:0 5px;color:var(--ink2);background:var(--win);margin-left:5px}
.body{flex:1;display:flex;min-height:0}
.sidebar{width:200px;flex:none;background:var(--side);border-right:1px solid var(--line);padding:12px 8px;overflow:hidden}
.sidebar h3{font:600 11px var(--font);color:var(--ink2);margin:14px 8px 4px;letter-spacing:.02em}
.sidebar a{display:flex;justify-content:space-between;padding:5px 8px;border-radius:6px;color:var(--ink);text-decoration:none}
.sidebar a.on{background:var(--sel);font-weight:600}.sidebar a span{color:var(--ink2);font-variant-numeric:tabular-nums}
.list{width:350px;flex:none;border-right:1px solid var(--line);overflow:hidden;display:flex;flex-direction:column}
.listhead{padding:12px 16px 8px;display:flex;align-items:baseline;gap:8px}
.listhead b{font-size:15px}.listhead span{color:var(--ink2);font-size:12px}
.row{padding:var(--row-pad) 16px;border-bottom:1px solid var(--line);position:relative}
.row.on{background:var(--sel)}
.row .top{display:flex;justify-content:space-between;gap:8px}
.row .from{font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.row.unread .from{font-weight:700}
.row.unread::before{content:"";position:absolute;left:5px;top:16px;width:7px;height:7px;border-radius:50%;background:var(--unread)}
.row .when{color:var(--ink2);font-size:12px;white-space:nowrap}
.row .subj{font-size:12.5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.row .prev{color:var(--ink2);font-size:12.5px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.chip{display:inline-block;font:500 11px var(--font);padding:0 6px;border-radius:4px;background:var(--chip);color:var(--ink2);margin-right:5px}
.reader{flex:1;min-width:0;display:flex;flex-direction:column}
.reader .scroll{flex:1;overflow:hidden;padding:26px 36px}
.reader .inner{max-width:680px;margin:0 auto}
.reader h2{font:600 22px/1.25 var(--display);margin-bottom:4px}
.reader .meta{color:var(--ink2);font-size:12.5px;margin-bottom:18px}
.reader p{font-size:15px;line-height:1.55;margin-bottom:12px}
.suggest{display:flex;gap:8px;align-items:center;color:var(--ok);font-size:12.5px;margin:0 0 14px}
.answers{display:flex;flex-direction:column;gap:8px;margin-top:18px}
.answer{border:1px solid var(--line);border-radius:var(--radius);padding:9px 12px;font-size:13.5px;display:flex;gap:10px;align-items:baseline}
.answer kbd{margin:0}
.ask{flex:none;border-top:1px solid var(--line);padding:10px 16px;color:var(--ink2);display:flex;justify-content:space-between;font-size:13px}
.actions{flex:none;display:flex;gap:8px;padding:12px 36px;border-top:1px solid var(--line);align-items:center;flex-wrap:wrap}
.btn{padding:7px 14px;border-radius:999px;background:var(--card);font-weight:600;font-size:13px;white-space:nowrap}
.btn.primary{background:var(--accent);color:var(--accent-ink)}.btn.primary kbd{background:transparent;color:inherit;border-color:currentColor;opacity:.8}
.empty{flex:1;display:flex;align-items:center;justify-content:center;color:var(--ink3);font-size:15px}
.hide{display:none!important}
.note{max-width:1200px;margin:8px auto 0;color:var(--ink2);font-size:12.5px}
"""

SCRIPT = """
<script>
const root=document.documentElement, stage=document.querySelector('.stage');
function press(group,btn){document.querySelectorAll('[data-g="'+group+'"]').forEach(b=>b.setAttribute('aria-pressed',b===btn))}
document.querySelectorAll('.controls button').forEach(b=>b.addEventListener('click',()=>{
  const g=b.dataset.g,v=b.dataset.v; press(g,b);
  if(g==='theme') root.dataset.theme=v;
  if(g==='width') stage.classList.toggle('narrow',v==='narrow');
  if(g==='state') document.querySelectorAll('[data-state]').forEach(e=>e.classList.toggle('hide',!e.dataset.state.split(' ').includes(v)));
}));
</script>
"""


def page(filename, title, lede, gains, loses, window, extra_css="", states=None, note=""):
    def lis(items):
        return "".join(f"<li>{i}</li>" for i in items)
    state_buttons = ""
    if states:
        state_buttons = "".join(
            f'<button data-g="state" data-v="{v}" aria-pressed="{"true" if i == 0 else "false"}">{label}</button>'
            for i, (v, label) in enumerate(states))
    html = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title><style>{BASE_CSS}{extra_css}</style></head><body>
<div class="intro"><h1>{title}</h1><p>{lede}</p>
<div class="pros"><div><h2>Gains</h2><ul>{lis(gains)}</ul></div><div><h2>Loses</h2><ul>{lis(loses)}</ul></div></div>
<div class="controls">
<button data-g="theme" data-v="light" aria-pressed="false">Light</button>
<button data-g="theme" data-v="dark" aria-pressed="false">Dark</button>
<button data-g="width" data-v="wide" aria-pressed="true">1200 wide</button>
<button data-g="width" data-v="narrow" aria-pressed="false">820 wide</button>
{state_buttons}</div></div>
<div class="stage">{window}</div>
<p class="note">{note}</p>
{SCRIPT}</body></html>"""
    (OUT / filename).write_text(html)
    print("wrote", filename)


def lights():
    return '<div class="lights"><i></i><i></i><i></i></div>'


def row(t, on=False, compact=False):
    sender, subject, preview, when, unread, chip, _ = t
    cls = "row" + (" on" if on else "") + (" unread" if unread else "")
    chip_html = f'<span class="chip">{chip}</span>' if chip else ""
    prev = "" if compact else f'<div class="prev">{chip_html}{preview}</div>'
    return (f'<div class="{cls}"><div class="top"><span class="from">{sender}</span>'
            f'<span class="when">{when}</span></div><div class="subj">{subject}</div>{prev}</div>')


def rows(threads, selected=0, compact=False):
    return "".join(row(t, i == selected, compact) for i, t in enumerate(threads))


def sidebar():
    return """<nav class="sidebar">
<a class="on">Inbox <span>12</span></a><a>Tasks <span>3</span></a><a>Drafts <span>2</span></a><a>Sent</a><a>All Mail</a>
<h3>Kaluta</h3><a>What Kaluta Knows</a><a>Routines</a>
<h3>Labels</h3><a>Customers</a><a>Hiring</a><a>Receipts</a><a>Newsletters</a></nav>"""


def reader(actions=True, ask=True, answers=True):
    ans = ""
    if answers:
        ans = """<div class="answers">
<div class="answer"><kbd>1</kbd>Go ahead with the offer.</div>
<div class="answer"><kbd>2</kbd>Let's meet them once more first.</div>
<div class="answer"><kbd>3</kbd>Hold until I've seen the portfolio again.</div></div>"""
    act = ""
    if actions:
        act = """<div class="actions"><span class="btn primary">Reply<kbd>R</kbd></span><span class="btn">Make a task<kbd>T</kbd></span>
<span class="btn">Archive<kbd>E</kbd></span><span class="btn">Later<kbd>B</kbd></span></div>"""
    ask_html = '<div class="ask"><span>✦ Ask Kaluta about this conversation…</span><span><kbd>⌘K</kbd></span></div>' if ask else ""
    return f"""<section class="reader"><div class="scroll"><div class="inner">
<div class="suggest">✦ {SUGGESTION}</div>
<h2>Interview feedback for the designer role</h2>
<div class="meta">Taylor Chen · today at 7:17 PM · to you</div>
{MESSAGE}{ans}</div></div>{act}{ask_html}</section>"""


def toolbar(title, sub, tools=("Reply", "Archive", "Label", "Search"), switcher=False):
    name = f'<span class="tool" style="border:1px solid var(--line)">{title} ▾<kbd>⌘K</kbd></span>' if switcher \
        else f'<span class="title">{title}</span>'
    tool_html = "".join(f'<span class="tool">{t}</span>' for t in tools)
    return (f'<div class="bar">{lights()}{name}<span class="sub">{sub}</span><span class="grow"></span>'
            f'{tool_html}<span class="tool primary">New Message</span></div>')


# ── Part 2: layouts ─────────────────────────────────────────────────────

def layout_a():
    css = """
.narrow .sidebar{width:56px}.narrow .sidebar a{font-size:0;justify-content:center}.narrow .sidebar a span{font-size:11px}
.narrow .sidebar h3{display:none}.narrow .list{width:300px}
"""
    win = f"""<div class="window">{toolbar("Inbox", "12 new since 9:00 · 3 need you")}
<div class="body">{sidebar()}<div class="list"><div class="listhead"><b>Inbox</b><span>12 new · 3 need you</span></div>{rows(THREADS[:7])}</div>
{reader()}</div></div>"""
    page("layout-a-three-columns.html", "A. Three columns, refined",
         "Today's window with its worst findings fixed. Kaluta's column is gone: the prompt is one line under the "
         "reader and its answers open over the reader, so the reader never drops below a readable width. "
         "The list header counts what is new and what needs you. When the window narrows, the sidebar "
         "shrinks to a strip before the reader does.",
         ["Nothing to relearn; mailboxes and labels stay in view", "The reader keeps at least 480 points at any window size",
          "Selection is visible at every level, as the platform expects"],
         ["Still three scrolling regions competing for attention", "The list shows only seven rows of two-line previews",
          "Looks like every other mail app: adds nothing to Kaluta's own character"],
         win, css, note="The control: the other four are compared with this one.")


def layout_b():
    css = ".list{width:400px}.narrow .list{width:320px}"
    win = f"""<div class="window">{toolbar("Inbox", "12 new · 3 need you", switcher=True)}
<div class="body"><div class="list">{rows(THREADS[:8])}</div>{reader()}</div></div>"""
    page("layout-b-two-panes.html", "B. Two panes",
         "The list and the reader, nothing else. Mailboxes, labels and Kaluta's pages move into one switcher at the "
         "top of the list (a click, or ⌘K and a few letters). The sidebar's 200 points go to the list and the reader.",
         ["A wider list (sender, subject and two lines without truncation) and a wider reader",
          "One fewer region to scan; the window reads left to right as choose, then read",
          "Works at 820 points without anything collapsing"],
         ["Other mailboxes are out of sight: counts for Drafts or Tasks are not visible at a glance",
          "Dragging a conversation onto a label needs another way (the L key and the palette already exist)",
          "The platform's guidance is not to hide the sidebar by default; this goes against it knowingly"],
         win, css, note="The switcher lists what the sidebar lists today, with counts, and remembers the last few places.")


def layout_c():
    css = """
.solo{flex:1;overflow:hidden;display:flex;justify-content:center}
.solo .col{width:100%;max-width:780px;display:flex;flex-direction:column}
.solo .row{padding:13px 22px}.solo .row .from{font-size:14.5px}.solo .row .subj{font-size:14px}.solo .row .prev{font-size:13.5px;-webkit-line-clamp:1}
.crumb{padding:10px 36px;color:var(--ink2);font-size:12.5px;border-bottom:1px solid var(--line);display:flex;justify-content:space-between}
"""
    win = f"""<div class="window">{toolbar("Inbox", "12 new · 3 need you", switcher=True)}
<div class="body" data-state="list"><div class="solo"><div class="col"><div class="listhead"><b>Inbox</b><span>12 new · 3 need you</span></div>{rows(THREADS)}</div></div></div>
<div class="body hide" data-state="open" style="flex-direction:column"><div class="crumb"><span>‹ Inbox<kbd>Esc</kbd></span><span>1 of 12 · next<kbd>J</kbd> previous<kbd>K</kbd></span></div>{reader()}</div>
</div>"""
    page("layout-c-one-column.html", "C. One column",
         "The list fills the window; Return opens a conversation in its place, as the workflows show one thing at a "
         "time. Esc goes back to the same row. J and K move to the next and previous conversation without going back.",
         ["The mail window and the workflows become one way of working: one thing on screen, keys named beside it",
          "The most room for reading of any option, at any window size", "The calmest: one region, one scroll"],
         ["No list beside the message: comparing two conversations means going back and forth",
          "A click on a row now navigates, where before it only selected; multi-select needs care",
          "Wide windows leave empty margins unless the reading column is allowed to grow"],
         win, css, states=[("list", "The list"), ("open", "A conversation open")],
         note="Use the two buttons above to see each state.")


def layout_d():
    css = """
.wide{flex:1;overflow:hidden}
.wrow{display:grid;grid-template-columns:18px 170px 1fr auto 76px;gap:12px;align-items:baseline;padding:9px 18px;border-bottom:1px solid var(--line)}
.wrow.on{background:var(--sel)}.wrow .dot{width:7px;height:7px;border-radius:50%;background:var(--unread);align-self:center}
.wrow .dot.off{background:transparent}.wrow .from{font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.wrow.unread .from{font-weight:700}.wrow .text{white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:var(--ink2)}
.wrow .text b{color:var(--ink);font-weight:500}.wrow .when{color:var(--ink2);font-size:12px;text-align:right}
.peek{position:absolute;top:52px;right:0;bottom:0;width:62%;background:var(--win);border-left:1px solid var(--line);box-shadow:-18px 0 40px rgba(0,0,0,.14);display:flex;flex-direction:column}
.narrow .peek{width:78%}.narrow .wrow{grid-template-columns:18px 120px 1fr 60px}.narrow .wrow .chips{display:none}
.peekbar{padding:9px 18px;border-bottom:1px solid var(--line);color:var(--ink2);font-size:12.5px;display:flex;justify-content:space-between}
"""
    def wrow(t, on=False):
        sender, subject, preview, when, unread, chip, _ = t
        chip_html = f'<span class="chip">{chip}</span>' if chip else ""
        return (f'<div class="wrow{" on" if on else ""}{" unread" if unread else ""}"><span class="dot{"" if unread else " off"}"></span>'
                f'<span class="from">{sender}</span><span class="text"><b>{subject}</b> — {preview}</span>'
                f'<span class="chips">{chip_html}</span><span class="when">{when}</span></div>')
    wrows = "".join(wrow(t, i == 0) for i, t in enumerate(THREADS))
    win = f"""<div class="window">{toolbar("Inbox", "12 new · 3 need you", switcher=True)}
<div class="body"><div class="wide">{wrows}</div></div>
<div class="peek" data-state="peek"><div class="peekbar"><span>Close<kbd>Esc</kbd> · Open in full<kbd>O</kbd></span><span>next<kbd>J</kbd> previous<kbd>K</kbd></span></div>{reader()}</div>
</div>"""
    page("layout-d-peek.html", "D. List with a peek",
         "The list keeps the whole window, one line a conversation, so far more fit on screen. Space or Return slides "
         "a conversation over part of the list; Esc closes it; the list underneath keeps its place.",
         ["Seventeen or more conversations visible at once: the best option for scanning and bulk tidying",
          "Reading does not lose the list: it stays visible at the left edge",
          "A cheap look before deciding, with a full view one key away"],
         ["One-line rows lose the two-line preview that makes many decisions possible without opening",
          "A panel that comes and goes is a second way to open mail that must stay consistent with the first",
          "The covered part of the list is dead space while reading"],
         win, css, states=[("peek", "A conversation peeked"), ("none", "The list alone")],
         note="Use the two buttons above to see the list alone.")


def layout_e():
    css = """
.list{width:400px}.narrow .list{width:330px}
.sec{display:flex;align-items:baseline;gap:8px;padding:12px 16px 5px;font-weight:700;font-size:12px;letter-spacing:.04em;text-transform:uppercase}
.sec span{color:var(--ink2);font-weight:500;text-transform:none;letter-spacing:0}
.sec.answer{color:var(--ok)}.sec.decide{color:#6a5acd}.sec.waiting{color:var(--warn)}.sec.read{color:var(--ink2)}
.more{padding:6px 16px 10px;color:var(--ink2);font-size:12px}
"""
    groups = [("answer", "Answer", "3 · about 4 min"), ("decide", "Decide", "2"),
              ("waiting", "Waiting on others", "2"), ("read", "Read when you can", "5")]
    body = ""
    first = True
    for key, label, count in groups:
        items = [t for t in THREADS if t[6] == key]
        body += f'<div class="sec {key}">{label}<span>{count}</span></div>'
        shown = items[:3] if key != "read" else items[:1]
        for t in shown:
            body += row(t, first, compact=(key in ("waiting", "read")))
            first = False
        if key == "read":
            body += '<div class="more">4 more · open Weekly reading<kbd>5</kbd></div>'
    win = f"""<div class="window">{toolbar("Today", "12 new since 9:00 · 3 need you", switcher=True)}
<div class="body"><div class="list">{body}</div>{reader()}</div></div>"""
    page("layout-e-grouped.html", "E. Grouped by what is needed",
         "The Inbox as four short queues instead of one list by date: what to answer, what to decide, what you are "
         "waiting on, and what can be read whenever. Kaluta's triage suggestions sort the mail; a conversation in the "
         "wrong group is moved with one key and Kaluta learns from it. Each heading starts its workflow.",
         ["Answers the first question of the day (what needs me?) without opening the Dashboard",
          "Each group is small and finishable, with its own count and time estimate",
          "The mail window and the Dashboard stop being two places: this is the Dashboard, with the mail in it"],
         ["Depends on Kaluta sorting well: a conversation in the wrong group is out of sight, so the correction must be easy and visible",
          "Arrival order is lost; a plain list by date has to stay one key away",
          "Without a ready agent there are no groups; it falls back to the plain list"],
         win, css, note="Shown with two panes (option B). The grouping is independent of the pane arrangement and could sit in A, C or D.")


# ── Part 3: design directions ───────────────────────────────────────────

DASH_CARDS = [("Replies", "6 waiting on your answer", "about 5 min", "2"), ("Tasks", "3 for today · 2 later this week", "about 6 min", "3"),
              ("Waiting on others", "2 to chase", "about 2 min", "4"), ("Weekly reading", "Nothing due", "", "5"),
              ("Agents", "Nothing waiting", "", "6"), ("Teach Kaluta", "5 to decide", "about 2 min", "7")]

DIR_CSS = """
.tile{max-width:1200px;margin:0 auto 26px;background:var(--win);border:1px solid var(--line);border-radius:14px;padding:26px 30px;display:grid;grid-template-columns:1.2fr 1fr 1fr;gap:28px}
.tile h2{font:600 11px var(--font);letter-spacing:.07em;text-transform:uppercase;color:var(--ink2);margin-bottom:10px}
.tile .big{font:var(--display-weight,600) 40px/1.05 var(--display);letter-spacing:var(--display-track,0)}
.tile .mid{font:500 20px/1.3 var(--display);margin-top:8px}.tile .txt{font-size:15px;line-height:1.55;margin-top:8px;color:var(--ink)}
.sw{display:flex;gap:8px;flex-wrap:wrap}.sw i{width:54px;height:54px;border-radius:var(--radius);display:block;border:1px solid var(--line)}
.sw span{display:block;font:11px var(--mono);color:var(--ink2);margin-top:3px;width:54px}
.voice p{margin-bottom:8px;font-size:14px}.voice p b{display:block;font:600 11px var(--font);color:var(--ink2);letter-spacing:.04em;text-transform:uppercase}
.label{max-width:1200px;margin:22px auto 8px;font:600 12px var(--font);letter-spacing:.06em;text-transform:uppercase;color:var(--ink2)}
.dash{flex:1;overflow:hidden;padding:40px 104px}
.dash .date{color:var(--ink2);font-size:13px}.dash h1{font:var(--display-weight,700) 34px/1.15 var(--display);letter-spacing:var(--display-track,0);margin:4px 0 6px}
.dash .lead{color:var(--ink2);font-size:15px;margin-bottom:20px}
.hero{background:var(--hero);color:var(--hero-ink);border-radius:calc(var(--radius) * 2);padding:22px 26px;display:flex;justify-content:space-between;align-items:center}
.hero small{font:600 11px var(--font);letter-spacing:.08em;opacity:.85}.hero h2{font:var(--display-weight,700) 28px/1.2 var(--display);margin:4px 0}
.hero p{opacity:.92;font-size:15px}.hero .worth{margin-top:12px;font-size:13.5px;opacity:.95}
.hero .go{border:1.5px solid currentColor;border-radius:999px;padding:10px 18px;font-weight:700;white-space:nowrap}
.cards{display:grid;grid-template-columns:repeat(3,1fr);gap:14px;margin-top:16px}
.wcard{border:1px solid var(--line);border-radius:var(--radius);padding:13px 15px;background:var(--card);position:relative}
.wcard b{font:600 15px var(--display)}.wcard p{font-size:13px;margin-top:2px}.wcard small{color:var(--ink2);font-size:12px}
.wcard kbd{position:absolute;right:12px;top:12px}.wcard.quiet{opacity:.5}
.foot{display:flex;justify-content:space-between;color:var(--ink2);font-size:12.5px;margin-top:18px}
.narrow .dash{padding:28px 36px}.narrow .cards{grid-template-columns:repeat(2,1fr)}
"""


def dashboard_window():
    cards = "".join(
        f'<div class="wcard{" quiet" if "Nothing" in d else ""}"><kbd>{k}</kbd><b>{n}</b><p>{d}</p><small>{e}</small></div>'
        for n, d, e, k in DASH_CARDS)
    return f"""<div class="window"><div class="bar">{lights()}<span class="title">Kaluta</span><span class="grow"></span><span class="tool">Mail<kbd>M</kbd></span></div>
<div class="dash"><div class="date">Friday, November 20 · 8:40 AM</div><h1>About 20 minutes of mail today.</h1>
<div class="lead">12 new since 9:00 yesterday · 3 need you</div>
<div class="hero"><div><small>NEXT</small><h2>Triage what's new</h2><p>From Taylor Chen, Avery Kim and 2 others you know · 8 from others</p>
<div class="worth"><b>Taylor Chen</b> — Interview feedback for the designer role · <i>asks you · due Friday</i></div></div><span class="go">Return ↵</span></div>
<div class="cards">{cards}</div>
<div class="foot"><span>Done today: 1 set · 9 decided</span><span>Return starts · 1–7 chooses · M opens Mail</span></div></div></div>"""


def mail_window():
    return f"""<div class="window">{toolbar("Inbox", "12 new · 3 need you", switcher=True)}
<div class="body"><div class="list" style="width:400px">{rows(THREADS[:8])}</div>{reader()}</div></div>"""


def direction(filename, title, lede, css_light, css_dark, extra, tile, gains, loses):
    theme = (f":root{{{css_light}}}:root[data-theme=dark]{{{css_dark}}}"
             f"@media (prefers-color-scheme: dark){{:root:not([data-theme=light]){{{css_dark}}}}}")
    body = (f'{tile}<div class="label">The Dashboard</div>{dashboard_window()}'
            f'<div class="label">The mail window</div>{mail_window()}')
    page(filename, title, lede, gains, loses, body, DIR_CSS + theme + extra,
         note="Both screens use the two-pane arrangement so the directions can be compared like for like; "
              "a direction does not depend on the layout chosen.")


def swatches(items):
    return '<div class="sw">' + "".join(
        f'<div><i style="background:{c}"></i><span>{n}</span></div>' for n, c in items) + "</div>"


def tile(type_note, colours, voice):
    v = "".join(f"<p><b>{k}</b>{t}</p>" for k, t in voice)
    return f"""<div class="tile"><div><h2>Type</h2><div class="big">About 20 minutes of mail today.</div>
<div class="mid">Interview feedback for the designer role</div>
<div class="txt">Strong portfolio and great with the team; a little light on systems work. I'd lean towards an offer.</div>
<div class="txt" style="color:var(--ink2);font-size:12.5px">{type_note}</div></div>
<div><h2>Colour and shape</h2>{swatches(colours)}
<div style="margin-top:16px;display:flex;gap:8px;flex-wrap:wrap;align-items:center"><span class="btn primary">Reply<kbd>R</kbd></span><span class="btn">Archive<kbd>E</kbd></span><span class="chip">Customers</span></div></div>
<div class="voice"><h2>Voice</h2>{v}</div></div>"""


def directions():
    # 1. Paper
    direction(
        "direction-1-paper.html", "Direction 1: Paper",
        "A reading room. Mail is mostly reading and writing, so the app looks like a well-set page: a serif for "
        "subjects and headlines, warm paper instead of white, ink instead of black, hairlines instead of boxes, and "
        "one deep green for anything Kaluta does. Quiet enough that the mail is the loudest thing on screen.",
        "--page:#e8e4da;--win:#fbf8f1;--side:#f3efe5;--card:#f3efe5;--line:#e2dccd;--sel:#ece6d6;--ink:#23211c;--ink2:#6f6a5e;--ink3:#a8a294;"
        "--accent:#1f5c4a;--accent-ink:#fbf8f1;--unread:#1f5c4a;--ok:#1f5c4a;--chip:#ece6d6;--hero:#1f5c4a;--hero-ink:#fbf8f1;"
        "--display:'New York','Iowan Old Style','Charter',Georgia,serif;--display-weight:600;--display-track:-.01em;--radius:4px;--row-pad:12px",
        "--page:#14130f;--win:#1d1b16;--side:#23211b;--card:#26241d;--line:#36332a;--sel:#322f26;--ink:#efeadc;--ink2:#a59f8f;--ink3:#6f6a5e;"
        "--accent:#7fc4ad;--accent-ink:#10231d;--unread:#7fc4ad;--ok:#7fc4ad;--chip:#322f26;--hero:#234f41;--hero-ink:#f2eee2",
        ".reader p{font-family:var(--display);font-size:16.5px;line-height:1.6}.row .subj{font-family:var(--display);font-size:14px}"
        ".btn{border-radius:4px}.tool.primary{border-radius:4px}.wcard{background:transparent}",
        tile("Serif (the system's New York) for subjects, headlines and message text; the system sans for controls and lists.",
             [("paper", "#fbf8f1"), ("ink", "#23211c"), ("moss", "#1f5c4a"), ("rule", "#e2dccd"), ("clay", "#b4691f")],
             [("Headline", "About 20 minutes of mail today."), ("Suggestion", "Kaluta suggests a short answer."),
              ("End of a set", "That's the Inbox. Nine decided, two for tomorrow."), ("Tone", "Unhurried, literate, never cute.")]),
        ["Unmistakable at a glance without a single custom control: type and colour carry it",
         "Makes long mail pleasant to read, which is where the time goes", "Ages well; nothing here is a trend"],
        ["A serif in lists can feel bookish to people who want a tool", "Warm paper fights with HTML mail designed for white: the reader needs a white card for those",
         "Furthest from the platform's look, so system surfaces (sheets, menus) will contrast with it"])
    # 2. Signal
    direction(
        "direction-2-signal.html", "Direction 2: Signal",
        "An instrument. Kaluta is driven from the keyboard, so the keys become the look: keycaps, counts and times in "
        "a monospaced face, tight rows, strong contrast, square corners, and one electric colour kept for the single "
        "thing to do next. It should feel fast before you have pressed anything.",
        "--page:#dfe1e4;--win:#ffffff;--side:#f4f5f6;--card:#f4f5f6;--line:#d9dbdf;--sel:#e9ebee;--ink:#0d0f12;--ink2:#5b616b;--ink3:#9aa0a9;"
        "--accent:#3d3df5;--accent-ink:#ffffff;--unread:#3d3df5;--ok:#0d7a5f;--chip:#e9ebee;--hero:#0d0f12;--hero-ink:#ffffff;"
        "--display:var(--font);--display-weight:700;--display-track:-.025em;--radius:3px;--row-pad:8px",
        "--page:#050607;--win:#0e1013;--side:#14171b;--card:#171a1f;--line:#262a31;--sel:#1f232a;--ink:#f1f3f5;--ink2:#9099a5;--ink3:#5b616b;"
        "--accent:#8a8aff;--accent-ink:#0a0a2a;--unread:#8a8aff;--ok:#4fd1ae;--chip:#1f232a;--hero:#f1f3f5;--hero-ink:#0e1013",
        ".row .when,.listhead span,.bar .sub,.wcard small,.dash .date,.foot,.reader .meta{font-family:var(--mono);font-size:11.5px}"
        ".btn{border-radius:3px}.tool.primary{border-radius:3px}kbd{border-radius:2px}.hero .go{border-radius:3px;font-family:var(--mono)}"
        ".chip{font-family:var(--mono);font-size:10.5px;text-transform:uppercase;letter-spacing:.04em}",
        tile("The system sans, tight and heavy for headlines; a monospaced face for keys, counts, times and labels.",
             [("white", "#ffffff"), ("black", "#0d0f12"), ("volt", "#3d3df5"), ("grid", "#d9dbdf"), ("go", "#0d7a5f")],
             [("Headline", "20 min of mail. 3 need you."), ("Suggestion", "Suggested: quick reply · 20 s"),
              ("End of a set", "Inbox clear. 9 decided in 4:10."), ("Tone", "Brief, exact, numbers first.")]),
        ["Fits what the product already is: keys on every button, counts and estimates everywhere",
         "Densest of the three: more mail per screen", "Its dark appearance is the strongest of the three"],
        ["The look most often chosen by fast keyboard-first tools, so the least distinct among them",
         "Can read as cold; rewards and the gentle tone the workflows have now would need rethinking",
         "Small monospaced text is harder to read for long and at larger text sizes"])
    # 3. Daylight
    direction(
        "direction-3-daylight.html", "Direction 3: Daylight",
        "A companion with a sense of time. It grows from what is already Kaluta's own: each workflow's colour. Those "
        "colours come into the mail window (what to answer is teal, what to decide is violet, tasks are amber), type "
        "is large and rounded, corners are soft, and the Dashboard greets the part of the day it is.",
        "--page:#ecebf3;--win:#fdfcff;--side:#f4f2fa;--card:#f4f2fa;--line:#e4e1ee;--sel:#ebe7f7;--ink:#211f2b;--ink2:#6b677a;--ink3:#a39fb3;"
        "--accent:#6a5acd;--accent-ink:#ffffff;--unread:#2f8f7f;--ok:#2f8f7f;--chip:#ebe7f7;--hero:linear-gradient(120deg,#2f8f7f,#3c7f9f);--hero-ink:#ffffff;"
        "--display:ui-rounded,'SF Pro Rounded',var(--font);--display-weight:700;--display-track:-.01em;--radius:12px;--row-pad:12px",
        "--page:#0f0e15;--win:#191721;--side:#1f1d29;--card:#221f2d;--line:#322e42;--sel:#2c2840;--ink:#f1eefb;--ink2:#a7a2b9;--ink3:#6b677a;"
        "--accent:#a99cf5;--accent-ink:#1a1535;--unread:#66c7b6;--ok:#66c7b6;--chip:#2c2840;--hero:linear-gradient(120deg,#256f63,#2b5d77);--hero-ink:#f4fffc",
        ".row{border-bottom:none;margin:2px 8px;border-radius:10px}.row.unread::before{left:-2px}"
        ".wcard:nth-child(1){border-top:3px solid #2f8f7f}.wcard:nth-child(2){border-top:3px solid #c7812f}.wcard:nth-child(3){border-top:3px solid #b0596f}"
        ".wcard:nth-child(6){border-top:3px solid #4f8f4a}.reader h2{font-family:var(--display)}.dash h1{font-size:36px}",
        tile("A rounded face (the system's SF Rounded) for headlines and names; the system sans for reading. Large sizes, generous space.",
             [("answer", "#2f8f7f"), ("decide", "#6a5acd"), ("tasks", "#c7812f"), ("waiting", "#b0596f"), ("teach", "#4f8f4a")],
             [("Headline", "Good morning. About 20 minutes of mail today."), ("Suggestion", "Kaluta thinks a quick yes will do."),
              ("End of a set", "All caught up. Nine decided, nicely done."), ("Tone", "Warm, encouraging, plain.")]),
        ["Builds on what exists (workflow colours, pills, large type), so it is the cheapest to adopt",
         "Colour means something everywhere: the same teal says answer in the list, the workflow and the Dashboard",
         "The friendliest of the three; suits an assistant that proposes and is corrected"],
        ["Five meaning-bearing colours must each pass contrast in light, dark and Increase Contrast, and cannot be the only signal",
         "Rounded and soft can read as less serious for work mail", "Colour competes with label colours from the mail service; one of the two has to go quiet"])


if __name__ == "__main__":
    layout_a(); layout_b(); layout_c(); layout_d(); layout_e()
    directions()
