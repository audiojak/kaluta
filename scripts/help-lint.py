#!/usr/bin/env python3
"""Every button, menu button, toggle and picker in the app has a hover
description (`.help(...)`), docs/design-system.md. Exempt: menu-bar
commands (OpenAGCApp.swift), items inside a Menu (macOS shows no tooltips
there, nor in context menus) and lines marked `// no-help: <why>`.
Usage: scripts/help-lint.py [--strict]"""
import pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "macos" / "OpenAGC"
CONTROL = re.compile(r'(?<![.\w])(Button|Menu|Toggle|Picker)\s*[\(\{]')
findings = []
for path in sorted(ROOT.rglob("*.swift")):
    rel = path.relative_to(ROOT)
    if rel.as_posix() == "App/OpenAGCApp.swift":
        continue
    lines = path.read_text().split("\n")
    menu_depth = []  # indentation of open `Menu {` blocks
    for i, line in enumerate(lines):
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())
        while menu_depth and stripped.startswith("}") and indent <= menu_depth[-1]:
            if stripped.startswith("} label:"):
                menu_depth.pop()
                break
            menu_depth.pop()
        if stripped.startswith("//") or "// no-help" in line or not CONTROL.search(line):
            if re.search(r'(?<![.\w])Menu\s*(\([^)]*\))?\s*\{|\.contextMenu\s*\{', line):
                menu_depth.append(indent)
            continue
        inside_menu = bool(menu_depth)
        if re.search(r'(?<![.\w])Menu\s*(\([^)]*\))?\s*\{', line):
            menu_depth.append(indent)
        if inside_menu:
            continue
        # Its modifiers: following lines until the statement returns to
        # this indentation with something that is not a modifier.
        found = ".help(" in line or ".hoverHelp(" in line
        for nxt in lines[i + 1:i + 40]:
            n_indent = len(nxt) - len(nxt.lstrip())
            s = nxt.strip()
            if (".help(" in nxt or ".hoverHelp(" in nxt) and (n_indent > indent or s.startswith(".")):
                found = True
                break
            if s and n_indent <= indent and not s.startswith((".", "}", ")")):
                break
        if not found:
            findings.append(f"{rel}:{i + 1}: {stripped[:90]}")

# SwiftUI's own .help shows nothing in column-header bars and in
# Settings forms on macOS 26: views use .hoverHelp (Design/Components),
# which adds an AppKit tool tip. Toolbar content keeps .help, copied onto
# the toolbar items by ToolbarToolTips.
TOOLBAR = {"Features/MessageView/MessageToolbar.swift", "Features/Accounts/AccountMenu.swift", "App/Snapshot.swift",
           "Design/Components.swift", "App/OpenAGCApp.swift"}
for path in sorted(ROOT.rglob("*.swift")):
    rel = path.relative_to(ROOT).as_posix()
    if rel in TOOLBAR:
        continue
    for i, line in enumerate(path.read_text().split("\n")):
        if re.search(r'(?<![\w])\.help\(', line) and "// toolbar" not in line:
            findings.append(f"{rel}:{i + 1}: use .hoverHelp, not .help: {line.strip()[:70]}")

# Help is a short sentence without a full stop (docs/design-system.md).
for path in sorted(ROOT.rglob("*.swift")):
    rel = path.relative_to(ROOT).as_posix()
    for i, line in enumerate(path.read_text().split("\n")):
        if re.search(r'\.(hover)?[hH]elp\("[^"]*\."\)', line):
            findings.append(f"{rel}:{i + 1}: help ends with a full stop: {line.strip()[:70]}")

for f in findings:
    print(f"help-lint: {f}")
print(f"help-lint: {len(findings)} control(s) without .help" if findings else "help-lint: clean")
sys.exit(1 if findings and "--strict" in sys.argv else 0)
