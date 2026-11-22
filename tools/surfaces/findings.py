#!/usr/bin/env python3
"""Open findings: what a repository's own records say is missing, blocked, handed off, not run or
failed, computed from those records and never written by hand.

Usage: findings.py [--repo DIR] [--config FILE] [--json] [--live]

  --repo DIR     repository root (default: the git toplevel of the current directory)
  --config FILE  the TOML file whose [findings] table declares the sources (default: surfaces.toml)
  --json         print the findings as JSON instead of a Markdown table
  --live         also run the declared live sources (a command whose answer depends on the
                 moment, such as a planner's drift report); they are printed, never committed
  -h, --help     show this help

Every fact comes from a tracked file; the [findings] table only says where to look and which
vocabulary the repository uses (tools/surfaces/README.md, "Open findings"). Sources:

  issues        YAML issue records: an open issue waiting on one that is not done (blocked), a
                non_scope entry of a finished issue that no open issue takes over (out-of-scope),
                an evidence result that reports a declared open status (for example NOT RUN)
  tables        a Markdown table row whose status column holds a declared open status
  front_matter  a document whose front matter field holds a declared value
  markers       a passage a document marks with <!-- open-finding kind=K [follow-up=ID] -->, up
                to <!-- /open-finding --> or the end of its paragraph
  live          a command printing JSON, mapped field by field (only with --live)

Each finding carries its kind, its text, its evidence locator (path:line, record id), its
follow-up (open issue ids, or none) and a label: OBSERVED when the source states the open status
itself, DERIVED when the generator computed it from several records. Exit codes: 0 ok, 2 usage,
10 a declaration or marker is malformed.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from dataclasses import asdict, dataclass, field
from pathlib import Path

import yaml

EXIT_OK, EXIT_USAGE, EXIT_FINDINGS = 0, 2, 10
OBSERVED, DERIVED = "OBSERVED", "DERIVED"
NO_FOLLOW_UP = "none — needs an issue"
MARKER = "open-finding"
BEGIN_RE = re.compile(rf"(?<!`)<!-- {MARKER}((?: [^ >]+)*) -->")
END_RE = re.compile(rf"(?<!`)<!-- /{MARKER} -->")
FENCE_RE = re.compile(r"^\s*(```|~~~)")
SENTENCE_RE = re.compile(r"(?<=[.;])\s+")
# The generator's own kinds; every other kind is the repository's, declared in [findings] kinds.
BLOCKED, OUT_OF_SCOPE = "blocked", "out-of-scope"
DEFAULTS = {
    "kinds": {}, "statuses": {}, "issues": {}, "tables": {}, "front_matter": [], "markers": {},
    "live": [], "max_chars": 280,
}


class DeclarationError(ValueError):
    """A [findings] declaration or a document marker that cannot be read; a check finding."""


@dataclass
class Finding:
    kind: str
    text: str
    path: str
    line: int
    record: str = ""
    follow_up: list[str] = field(default_factory=list)
    label: str = OBSERVED


@dataclass
class Issue:
    id: str
    path: str
    title: str
    status: str
    rec: dict
    lines: dict


def glob_re(pattern: str) -> re.Pattern:
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out, i = out + "(?:.*/)?", i + 3
        elif pattern.startswith("**", i):
            out, i = out + ".*", i + 2
        elif pattern[i] == "*":
            out, i = out + "[^/]*", i + 1
        elif pattern[i] == "?":
            out, i = out + "[^/]", i + 1
        else:
            out, i = out + re.escape(pattern[i]), i + 1
    return re.compile(out + r"\Z")


def matches(path: str, patterns) -> bool:
    if isinstance(patterns, str):
        patterns = [patterns]
    return any(glob_re(p).match(path) for p in patterns)


def plain(text: str) -> str:
    text = re.sub(r"<!--.*?-->", "", text, flags=re.S)
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"<[^>]+>", "", text)
    text = re.sub(r"[*`]+", "", text)
    text = re.sub(r"(?<!\w)_+|_+(?!\w)", "", text)
    return re.sub(r"\s+", " ", text).strip()


def clip(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[: limit - 1].rsplit(" ", 1)[0] + "…"


def yaml_lines(text: str) -> dict:
    """1-based line of every node of a YAML document, keyed by its path of keys and indexes."""
    out: dict = {}

    def walk(node, key: tuple) -> None:
        out[key] = node.start_mark.line + 1
        if isinstance(node, yaml.MappingNode):
            for k, v in node.value:
                walk(v, key + (k.value,))
        elif isinstance(node, yaml.SequenceNode):
            for i, v in enumerate(node.value):
                walk(v, key + (i,))

    root = yaml.compose(text)
    if root is not None:
        walk(root, ())
    return out


def issue_status(rec: dict) -> str:
    """The lifecycle stamps' status; only a completed issue satisfies a dependency (as in
    Majordomus: a verified one still awaits its evidence)."""
    if rec.get("completed_at"):
        return "done"
    if rec.get("verified_at"):
        return "verified"
    if rec.get("started_at"):
        return "active"
    return "open"


def fenced_lines(lines: list[str]) -> set[int]:
    """0-based indexes of the lines inside (or opening and closing) a fenced code block."""
    out, fence = set(), None
    for i, line in enumerate(lines):
        m = FENCE_RE.match(line)
        if fence is None and m:
            fence = m.group(1)
            out.add(i)
        elif fence is not None:
            out.add(i)
            if line.strip().startswith(fence):
                fence = None
    return out


def front_matter(text: str) -> tuple[dict | None, int]:
    """(front matter, the 1-based line the front matter starts on) of a '---' fenced header."""
    if not text.startswith("---\n"):
        return None, 0
    end = text.find("\n---\n", 3)
    if end < 0:
        return None, 0
    try:
        data = yaml.safe_load(text[4:end])
    except yaml.YAMLError:
        return None, 0
    return (data, 2) if isinstance(data, dict) else (None, 0)


class Sources:
    """The repository as the generator reads it: tracked files, their text, the issue records."""

    def __init__(self, root: Path, tracked: list[str], cfg: dict):
        self.root, self.tracked = root, sorted(tracked)
        self.cfg = {**DEFAULTS, **cfg}
        self.errors: list[str] = []
        self.texts: dict[str, str] = {}
        icfg = self.cfg["issues"]
        self.id_re = re.compile(rf"\b(?:{icfg['id']})\b") if icfg.get("id") else None
        self.statuses: dict[str, str] = self.cfg["statuses"]
        self.status_re = re.compile(
            r"(?<![\w-])(" + "|".join(re.escape(s) for s in sorted(self.statuses, key=len, reverse=True))
            + r")(?![\w-])") if self.statuses else None
        self.issues: dict[str, Issue] = {}
        if icfg.get("in"):
            for path in self.files(icfg["in"], icfg.get("exclude", [])):
                text = self.text(path)
                rec = yaml.safe_load(text) or {}
                iid = str(rec.get("id") or Path(path).stem)
                self.issues[iid] = Issue(iid, path, str(rec.get("title", "")), issue_status(rec),
                                         rec, yaml_lines(text))

    def files(self, include, exclude=()) -> list[str]:
        return [p for p in self.tracked if matches(p, include) and not matches(p, list(exclude))]

    def text(self, path: str) -> str:
        if path not in self.texts:
            self.texts[path] = (self.root / path).read_text(encoding="utf-8", errors="replace")
        return self.texts[path]

    def open_ids(self, text: str, own: str = "") -> list[str]:
        """The open issues a text names, in order, without `own`."""
        if not self.id_re:
            return []
        out = []
        for iid in self.id_re.findall(text):
            issue = self.issues.get(iid)
            if issue and issue.status != "done" and iid != own and iid not in out:
                out.append(iid)
        return out

    def clip(self, text: str) -> str:
        return clip(plain(text), int(self.cfg["max_chars"]))

    def status_kind(self, cell: str) -> str | None:
        """The kind of an open status a cell begins with, or None."""
        m = self.status_re.match(plain(cell)) if self.status_re else None
        return self.statuses[m.group(1)] if m else None


def from_issues(src: Sources) -> list[Finding]:
    out: list[Finding] = []
    of = src.cfg["issues"].get("non_scope_of", ["done"])
    for issue in src.issues.values():
        rec, at = issue.rec, issue.lines
        if issue.status != "done":
            waiting = [d for d in rec.get("depends_on") or []
                       if d in src.issues and src.issues[d].status != "done"]
            if waiting:
                names = ", ".join(f"{d} ({src.issues[d].status})" for d in waiting)
                out.append(Finding(BLOCKED, src.clip(f"{issue.title} — waits on {names}"), issue.path,
                                   at.get(("depends_on",), 1), issue.id, waiting, DERIVED))
        if issue.status in of:
            for i, entry in enumerate(rec.get("non_scope") or []):
                named = src.id_re.findall(str(entry)) if src.id_re else []
                named = [n for n in named if n in src.issues and n != issue.id]
                follow = src.open_ids(str(entry), issue.id)
                if named and not follow:
                    continue  # every issue it names is done: taken over and finished
                out.append(Finding(OUT_OF_SCOPE, src.clip(str(entry)), issue.path,
                                   at.get(("non_scope", i), 1), issue.id, follow, DERIVED))
        for i, ev in enumerate(rec.get("evidence") or []):
            result = str(ev.get("result", "")) if isinstance(ev, dict) else ""
            seen: set[str] = set()
            for m in src.status_re.finditer(result) if src.status_re else ():
                if m.group(1) in seen:
                    continue
                seen.add(m.group(1))
                begin = max((b.end() for b in SENTENCE_RE.finditer(result, 0, m.start())), default=0)
                clause = SENTENCE_RE.split(result[begin:], maxsplit=1)[0]
                if not clause.startswith(m.group(1)) and len(clause) > int(src.cfg["max_chars"]):
                    clause = result[m.start():]  # a long sentence: start where the status does
                follow = [issue.id] if issue.status != "done" else src.open_ids(clause, issue.id)
                out.append(Finding(src.statuses[m.group(1)], src.clip(clause), issue.path,
                                   at.get(("evidence", i, "result"), 1), issue.id, follow))
    return out


def table_cells(line: str) -> list[str]:
    return [c.strip() for c in re.split(r"(?<!\\)\|", line.strip().strip("|"))]


def from_tables(src: Sources) -> list[Finding]:
    tcfg = src.cfg["tables"]
    if not tcfg.get("in"):
        return []
    columns = set(tcfg.get("columns", []))
    out: list[Finding] = []
    for path in src.files(tcfg["in"], tcfg.get("exclude", [])):
        lines = src.text(path).split("\n")
        fenced = fenced_lines(lines)
        i = 0
        while i < len(lines) - 1:
            if (i in fenced or not lines[i].lstrip().startswith("|")
                    or not re.fullmatch(r"\s*\|[\s:|-]+\|\s*", lines[i + 1])):
                i += 1
                continue
            header = [plain(c) for c in table_cells(lines[i])]
            wanted = [k for k, h in enumerate(header) if h in columns]
            i += 2
            while i < len(lines) and lines[i].lstrip().startswith("|"):
                cells = table_cells(lines[i])
                for k in wanted:
                    kind = src.status_kind(cells[k]) if k < len(cells) else None
                    if kind:
                        name = next((c for j, c in enumerate(cells) if j != k and plain(c)), "")
                        out.append(Finding(kind, src.clip(f"{name}: {cells[k]}"), path, i + 1,
                                           follow_up=src.open_ids(lines[i])))
                i += 1
    return out


def from_front_matter(src: Sources) -> list[Finding]:
    out: list[Finding] = []
    for decl in src.cfg["front_matter"]:
        name, values = decl["field"], [str(v) for v in decl["values"]]
        for path in src.files(decl["in"], decl.get("exclude", [])):
            text = src.text(path)
            fm, start = front_matter(text)
            if not fm or str(fm.get(name)) not in values:
                continue
            line = next((n for n, l in enumerate(text.split("\n"), 1)
                         if n >= start and l.startswith(f"{name}:")), start)
            what = str(fm.get("title") or fm.get("description") or Path(path).stem)
            out.append(Finding(decl["kind"], src.clip(f"{fm.get(name)}: {what}"), path, line,
                               str(fm.get("id") or Path(path).stem), src.open_ids(what)))
    return out


def marker_attrs(raw: str, where: str) -> dict:
    attrs = {}
    for token in raw.split():
        key, eq, value = token.partition("=")
        if not eq or key not in ("kind", "follow-up"):
            raise DeclarationError(f"{where}: {MARKER} takes kind=K and follow-up=ID, not '{token}'")
        attrs[key] = value
    if "kind" not in attrs:
        raise DeclarationError(f"{where}: {MARKER} needs kind=K")
    return attrs


def from_markers(src: Sources) -> list[Finding]:
    mcfg = src.cfg["markers"]
    if not mcfg.get("in"):
        return []
    out: list[Finding] = []
    for path in src.files(mcfg["in"], mcfg.get("exclude", [])):
        text = src.text(path)
        if f"{MARKER}" not in text:
            continue
        lines = text.split("\n")
        fenced = fenced_lines(lines)
        visible = "\n".join("" if i in fenced else l for i, l in enumerate(lines))
        begins = list(BEGIN_RE.finditer(visible))
        ends = list(END_RE.finditer(visible))
        used: set[int] = set()
        for b in begins:
            line = visible.count("\n", 0, b.start()) + 1
            where = f"{path}:{line}"
            try:
                attrs = marker_attrs(b.group(1), where)
            except DeclarationError as err:
                src.errors.append(str(err))
                continue
            if attrs["kind"] not in src.cfg["kinds"]:
                src.errors.append(f"{where}: kind '{attrs['kind']}' is not declared in [findings] kinds")
                continue
            para = re.search(r"\n\s*\n|\Z", visible[b.end():])
            stop = b.end() + para.start()
            nxt = next((x.start() for x in begins if x.start() > b.start()), len(visible))
            end = next((e for e in ends if b.end() <= e.start() < min(stop, nxt)), None)
            if end:
                used.add(end.start())
            body = visible[b.end(): end.start() if end else min(stop, nxt)]
            follow = src.open_ids(body)
            if "follow-up" in attrs:
                fid = attrs["follow-up"]
                if fid not in src.issues:
                    src.errors.append(f"{where}: follow-up '{fid}' names no issue record")
                    continue
                follow = [fid]
            out.append(Finding(attrs["kind"], src.clip(body), path, line, follow_up=follow))
        for e in ends:
            if e.start() not in used:
                line = visible.count("\n", 0, e.start()) + 1
                src.errors.append(f"{path}:{line}: /{MARKER} closes no {MARKER} in its paragraph")
    return out


def dig(data, dotted: str):
    for part in dotted.split(".") if dotted else []:
        data = data.get(part, {}) if isinstance(data, dict) else {}
    return data


def from_live(src: Sources) -> tuple[list[Finding], list[str]]:
    """Run each declared live command; return its findings and a NOT RUN line per command that
    could not answer."""
    out: list[Finding] = []
    not_run: list[str] = []
    for decl in src.cfg["live"]:
        try:
            r = subprocess.run(decl["command"], cwd=src.root, capture_output=True, text=True,
                               timeout=int(decl.get("timeout", 300)))
            if r.returncode != 0:
                raise RuntimeError(f"exited {r.returncode}: {r.stderr.strip()[-200:]}")
            data = json.loads(r.stdout)
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as err:
            not_run.append(f"NOT RUN: {decl['id']}: {err}")
            continue
        keep = decl.get("keep", {})
        for item in dig(data, decl.get("items", "")) or []:
            if any(str(item.get(k)) not in [str(v) for v in vs] for k, vs in keep.items()):
                continue
            record = str(item.get(decl.get("record", ""), ""))
            issue = src.issues.get(record)
            text = " — ".join(str(item.get(f, "")) for f in decl.get("text", []))
            follow = [record] if issue and issue.status != "done" else src.open_ids(text, record)
            out.append(Finding(decl["kind"], src.clip(text), issue.path if issue else decl["id"],
                               1 if issue else 0, record, follow, DERIVED))
    return out, not_run


@dataclass
class Result:
    findings: list[Finding]
    errors: list[str]
    not_run: list[str]
    issues: dict[str, str]  # issue id -> its record's path


def collect(root: Path, tracked: list[str], cfg: dict, live: bool = False) -> Result:
    """The findings in kind order, the declaration errors and the NOT RUN line of each live source
    that could not answer."""
    src = Sources(root, tracked, cfg)
    found = from_issues(src) + from_tables(src) + from_front_matter(src) + from_markers(src)
    not_run: list[str] = []
    if live:
        extra, not_run = from_live(src)
        found += extra
    order = list(src.cfg["kinds"])
    for f in found:
        if f.kind not in order:
            src.errors.append(f"{f.path}:{f.line}: kind '{f.kind}' is not declared in [findings] kinds")
            order.append(f.kind)
    found.sort(key=lambda f: (order.index(f.kind), f.path, f.line))
    return Result(found, src.errors, not_run, {i: x.path for i, x in src.issues.items()})


def cell(text: str) -> str:
    return str(text).replace("|", "\\|").replace("\n", " ")


def render(result: Result, cfg: dict, link) -> str:
    """The legend and the table, in Markdown; `link(path)` is a path's link from the page."""
    kinds, found, issues = {**DEFAULTS, **cfg}["kinds"], result.findings, result.issues
    legend = ["| Kind | Count | Meaning |", "|---|---|---|"]
    legend += [f"| {k} | {sum(f.kind == k for f in found)} | {cell(v)} |" for k, v in kinds.items()]
    rows = ["| Kind | Finding | Evidence | Follow-up | Label |", "|---|---|---|---|---|"]
    for f in found:
        if f.line:
            where = f"[{f.record if f.record in issues else f.path}]({link(f.path)}):{f.line}"
        else:
            where = f"`{f.path}` ({f.record})" if f.record else f"`{f.path}`"
        follow = ", ".join(f"[{i}]({link(issues[i])})" if i in issues else i for i in f.follow_up)
        rows.append(f"| {f.kind} | {cell(f.text)} | {where} | {follow or NO_FOLLOW_UP} | {f.label} |")
    return "\n".join(legend) + "\n\n" + "\n".join(rows)


def load_findings_config(path: Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh).get("findings", {})


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(prog="findings", description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", default=".", help="repository root (default: the git toplevel of the cwd)")
    ap.add_argument("--config", default="surfaces.toml", help="the TOML file holding [findings]")
    ap.add_argument("--json", action="store_true", help="print JSON instead of a Markdown table")
    ap.add_argument("--live", action="store_true", help="also run the declared live sources")
    args = ap.parse_args(argv)
    root = Path(subprocess.run(["git", "-C", args.repo, "rev-parse", "--show-toplevel"],
                               check=True, capture_output=True, text=True).stdout.strip())
    cfg = load_findings_config(root / args.config)
    tracked = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True,
                             capture_output=True, text=True).stdout.split("\0")
    tracked = [p for p in tracked if p and (root / p).is_file()]
    result = collect(root, tracked, cfg, live=args.live)
    if args.json:
        print(json.dumps({"findings": [asdict(f) for f in result.findings], "errors": result.errors,
                          "not_run": result.not_run}, indent=2, ensure_ascii=False))
    else:
        print(render(result, cfg, lambda p: p))
        for line in result.not_run:
            print(line)
    for err in result.errors:
        print(f"FAIL findings {err}", file=sys.stderr)
    return EXIT_FINDINGS if result.errors else EXIT_OK


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
