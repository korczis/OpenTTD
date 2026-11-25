"""Feature maturity: every feature record's status is a level the repository defines, and its
evidence section carries, line by line, the evidence that level requires, each line citing it.

A feature record (`schema: feature/v1`, the files of the layer's `features` section) states its
status in its front matter. The values are the feature schema's own; what each one means and what
evidence promotes a feature to it are the repository's, defined once in a document (this
repository: .ai/repo/features/README.md, "Maturity levels") and declared for the check in the
`[features]` table of surfaces.toml:

  section     the heading of the evidence section in a record's body (default "Maturity")
  definition  the document that defines the levels, named in every finding
  levels      {level = {requires = [label, ...], allows = [label, ...]}}: the evidence lines a
              record at that level must carry, and those it may carry besides

An evidence line is a list item `- **label**: text`, continued by its indented lines. It cites its
evidence by at least one of: a code span naming a path git tracks (`path`, `path/`, `path:Symbol`,
`path:line:Symbol`; tools/knowledge/refs proves the symbol and the anchored line), an issue record
of the layer (I0123) or a pull request (PR #123). An undeclared `[features]` table checks nothing.
`tools/surfaces/surfaces check` reports each finding as kind `feature`.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

DEFAULTS = {"section": "Maturity", "definition": "", "levels": {}}

ITEM_RE = re.compile(r"^[-*] \*\*(?P<label>[^*]+)\*\*:\s*(?P<text>.*)$")
SPAN_RE = re.compile(r"(?<!`)`([^`\n]+)`(?!`)")
# The form tools/knowledge/refs judges: a path, then optionally a symbol or an anchored line.
REF_RE = re.compile(r"^(?P<path>[A-Za-z0-9_.][A-Za-z0-9_./-]*)(?::[A-Za-z0-9_:~<>,*() -]+)?$")
ISSUE_RE = re.compile(r"(?<![A-Za-z0-9])I\d{4}(?![0-9])")
PR_RE = re.compile(r"\bPR #\d+\b")
FENCE_RE = re.compile(r"^\s*(```|~~~)")


@dataclass
class Problem:
    path: str
    line: int
    message: str


@dataclass
class Item:
    label: str
    line: int
    text: str


def section_items(lines: list[str], heading: str) -> tuple[int, list[Item]] | None:
    """The line of the `## <heading>` section and its evidence lines, or None without one."""
    start, fenced = None, False
    items: list[Item] = []
    for no, line in enumerate(lines, 1):
        if FENCE_RE.match(line):
            fenced = not fenced
            continue
        if fenced:
            continue
        if start is None:
            if line.rstrip() == f"## {heading}":
                start = no
            continue
        if line.startswith("## ") or line.startswith("# "):
            break
        m = ITEM_RE.match(line)
        if m:
            items.append(Item(m["label"].strip(), no, m["text"]))
        elif items and line.startswith((" ", "\t")) and line.strip():
            items[-1].text += " " + line.strip()
    return None if start is None else (start, items)


def cites(text: str, tracked: set[str], dirs: set[str], issues: set[str]) -> bool:
    """True when the text names at least one piece of evidence this repository can resolve."""
    for span in SPAN_RE.findall(text):
        m = REF_RE.match(span.strip())
        if m:
            path = m["path"].rstrip("/")
            if path in tracked or path in dirs:
                return True
    if any(i in issues for i in ISSUE_RE.findall(text)):
        return True
    return bool(PR_RE.search(text))


def check(root: Path, features: list[dict], tracked: set[str], dirs: set[str],
          issues: set[str], cfg: dict) -> list[Problem]:
    """Every problem of every feature record against the levels `cfg` declares."""
    levels: dict = cfg.get("levels") or {}
    if not levels:
        return []
    heading = cfg.get("section") or DEFAULTS["section"]
    where = f" ({cfg['definition']})" if cfg.get("definition") else ""
    problems: list[Problem] = []
    for f in features:
        if f.get("kind", "feature") != "feature":  # a domain record shares the directory
            continue
        path, status = f["path"], f.get("status")

        def say(line: int, message: str) -> None:
            problems.append(Problem(path, line, message))

        level = levels.get(status) if isinstance(status, str) else None
        if level is None:
            say(1, f"status '{status}' is not a defined maturity level: {', '.join(levels)}{where}")
            continue
        requires = list(level.get("requires") or [])
        known = requires + [a for a in level.get("allows") or [] if a not in requires]
        lines = (root / path).read_text(encoding="utf-8").splitlines()
        found = section_items(lines, heading)
        if found is None:
            say(1, f"no '## {heading}' section: a {status} feature cites its evidence there "
                   f"({', '.join(requires)}){where}")
            continue
        start, items = found
        seen = set()
        for item in items:
            if item.label not in known:
                say(item.line, f"'{item.label}' is not evidence a {status} feature carries "
                               f"({', '.join(known)}){where}")
                continue
            seen.add(item.label)
            if not cites(item.text, tracked, dirs, issues):
                say(item.line, f"'{item.label}' cites nothing this repository resolves: a tracked "
                               "path in a code span (path, path:line:Symbol), an issue (I0123) "
                               "or PR #123")
        for label in requires:
            if label not in seen:
                say(start, f"a {status} feature needs a '- **{label}**:' line citing its "
                           f"evidence{where}")
    return problems
