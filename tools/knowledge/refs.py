#!/usr/bin/env python3
"""Source references in the repository's tracked Markdown records: check them, index them,
and name the records a diff touches.

A record cites the code it describes. Nothing kept those citations true: a renamed file or a
deleted function left a record that still sent the next reader to it. This tool reads what
git tracks, plus the new files git would not ignore (so a record is checked before its
`git add`), and nothing else: two checkouts at one commit give the same answer.

What counts as a reference, in a scanned Markdown file outside fenced code blocks:

  - an inline code span whose first path segment is a top-level entry git tracks, of the form
    `path`, `path/` (a directory), `path:Symbol` (`A::B::name` checks the last component,
    and `name()` is the same as `name`), `path:123` or `path:12-14` (only the file is checked:
    a bare line number cannot be verified and rots on every rebase, so cite a symbol instead),
    or `path:123:Symbol` / `path:12-14:Symbol`, a line anchored to a symbol: the lines must
    exist and the symbol must occur, as a whole word, on one of them;
  - each pathspec in the front matter's `tracks:` list.

A file reference must name a tracked file, a directory reference a directory holding tracked
files, a symbol must occur as a whole word in the file it is cited in, an anchored line must
hold its symbol, and a `tracks` pathspec must match at least one tracked file. With
`--strict-lines` a bare line number fails too: a document that promises `path:line` citations
(a capability matrix) cites every one anchored, so each is checked. An anchored line that moved
is named with the line its symbol is on now; `--fix-lines` rewrites it when that line is
unambiguous (the symbol's only line, or its only declaring line).

What is not judged, because it is not a claim about this tree: a span that resolves to nothing
and has no file extension, no trailing `/` and no symbol (a branch such as `research/x`, a
protocol method such as `tools/list`); a path git ignores (a checkout's local state); every
span of a record whose front matter `status` is `superseded` or `deprecated` (history cites
what was true then).

Commands:
  check  [--pathspec P]... [--file F]... [--strict-lines] [--fix-lines]
                                     every reference resolves; exit 1 naming each that does not
                                     (--file: any file, such as a model's answer, judged the same;
                                     --strict-lines: a line number must be anchored; --fix-lines:
                                     move each anchored line whose symbol moved unambiguously)
  index  [--pathspec P]... [--out F] the reverse index (cited file -> records) as JSON
  impact <rev-range> [--pathspec P]... [--json]
                                     the records that cite or track a file the range changes,
                                     each with the files and symbols to re-read

Generic: no fact about any one repository is written here. The default scan is the AI layer
(`.ai/**/*.md`); `--pathspec` replaces it.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_PATHSPECS = [":(glob).ai/**/*.md",
                     # a vendored package is another repository's text, citing that repository
                     ":(glob,exclude).ai/repo/rules/vendor/**"]

SPAN = re.compile(r"(?<!`)`([^`\n]+)`(?!`)")
REF = re.compile(r"^(?P<path>[A-Za-z0-9_.][A-Za-z0-9_./-]*)"
                 r"(?::(?:(?P<symbol>[A-Za-z_~][A-Za-z0-9_:~<>,* ]*(?:\(\))?)"
                 r"|(?P<line>[0-9]+(?:-[0-9]+)?)(?::(?P<anchor>[A-Za-z_~][A-Za-z0-9_:~]*(?:\(\))?))?))?$")
FENCE = re.compile(r"^\s*(```|~~~)")
HISTORY = re.compile(r"^status:\s*['\"]?(superseded|deprecated)\b", re.M)


@dataclass(frozen=True)
class Ref:
    record: str
    line: int
    path: str
    symbol: str | None
    kind: str  # file | dir | tracks
    lines: tuple[int, int] | None = None  # a cited line or range, `path:12-14`
    anchor: str | None = None             # the symbol a cited line must hold, `path:12:Name`
    raw: str = ""                         # the span as written, which --fix-lines replaces

    def label(self) -> str:
        if self.kind == "tracks":
            return f"tracks '{self.path}'"
        if self.lines:
            a, b = self.lines
            cited = f"{self.path}:{a}" + (f"-{b}" if b != a else "")
            return cited + (f":{self.anchor}" if self.anchor else "")
        return f"{self.path}:{self.symbol}" if self.symbol else self.path


@dataclass
class Repo:
    root: Path
    files: set[str] = field(default_factory=set)
    dirs: set[str] = field(default_factory=set)
    top: set[str] = field(default_factory=set)
    _text: dict[str, str] = field(default_factory=dict)

    @classmethod
    def load(cls, root: Path) -> "Repo":
        out = git(root, "ls-files", "-z", *tracked_sources())
        repo = cls(root=root)
        for name in out.split("\0"):
            if not name:
                continue
            repo.files.add(name)
            parts = name.split("/")
            repo.top.add(parts[0])
            for i in range(1, len(parts)):
                repo.dirs.add("/".join(parts[:i]))
        return repo

    def text(self, path: str) -> str:
        if path not in self._text:
            try:
                self._text[path] = (self.root / path).read_text(encoding="utf-8", errors="replace")
            except OSError:
                self._text[path] = ""
        return self._text[path]


def git(root: Path, *args: str) -> str:
    env = dict(os.environ)
    if env.get("GIT_INDEX_FILE"):
        # git may export a relative index path; `-C root` would resolve it against root.
        env["GIT_INDEX_FILE"] = os.path.abspath(env["GIT_INDEX_FILE"])
    return subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True,
                          text=True, env=env).stdout


def tracked_sources() -> list[str]:
    """What counts as a file: the index, plus new files git would not ignore.

    Inside a git hook (git exports GIT_INDEX_FILE to pre-commit) the commit holds only the index,
    so a file written but not added is left out, exactly as tools/surfaces/surfaces.py load_repo.
    """
    sources = ["--cached"]
    if not os.environ.get("GIT_INDEX_FILE"):
        sources += ["--others", "--exclude-standard"]
    return sources


def glob_regex(spec: str) -> re.Pattern[str]:
    """The pathspec glob subset: `**` spans directories, `*` and `?` stay inside a segment."""
    spec = spec.removeprefix(":(glob)")
    out, i = [], 0
    while i < len(spec):
        if spec.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif spec.startswith("**", i):
            out.append(".*")
            i += 2
        elif spec[i] == "*":
            out.append("[^/]*")
            i += 1
        elif spec[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(spec[i]))
            i += 1
    if spec.endswith("/"):
        out.append(".*")
    return re.compile("^" + "".join(out) + "$")


def front_matter(text: str) -> tuple[list[str], int]:
    """The `tracks:` entries of a YAML front matter block, and the line the body starts on."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return [], 0
    tracks, in_tracks = [], False
    for n, line in enumerate(lines[1:], start=1):
        if line.strip() == "---":
            return tracks, n + 1
        if re.match(r"^tracks:\s*$", line):
            in_tracks = True
            continue
        if in_tracks:
            m = re.match(r"^\s+-\s+['\"]?([^'\"#]+?)['\"]?\s*(#.*)?$", line)
            if m:
                tracks.append(m.group(1).strip())
                continue
            in_tracks = False
    return [], 0


def scan(repo: Repo, record: str, text: str | None = None) -> list[Ref]:
    text = repo.text(record) if text is None else text
    tracks, body_start = front_matter(text)
    if body_start and HISTORY.search("\n".join(text.split("\n")[:body_start])):
        return []
    refs = [Ref(record, 1, t, None, "tracks") for t in tracks]
    fenced = False
    for n, line in enumerate(text.split("\n"), start=1):
        if n <= body_start:
            continue
        if FENCE.match(line):
            fenced = not fenced
            continue
        if fenced:
            continue
        for span in SPAN.findall(line):
            m = REF.match(span.strip())
            if not m:
                continue
            path, symbol = m.group("path"), m.group("symbol")
            if "/" not in path or path.split("/")[0] not in repo.top:
                continue
            bare = path.rstrip("/")
            if path.endswith("/") or (not symbol and bare in repo.dirs):
                refs.append(Ref(record, n, bare, None, "dir"))
            elif symbol or "." in bare.rsplit("/", 1)[-1] or bare in repo.files:
                refs.append(Ref(record, n, bare, symbol.strip() if symbol else None, "file",
                                line_range(m.group("line")), m.group("anchor"), span.strip()))
    return refs


def line_range(text: str | None) -> tuple[int, int] | None:
    if not text:
        return None
    a, _, b = text.partition("-")
    return int(a), int(b or a)


def word_rx(symbol: str) -> re.Pattern[str]:
    return re.compile(r"(?<![A-Za-z0-9_])" + re.escape(symbol_word(symbol)) + r"(?![A-Za-z0-9_])")


def anchor_lines(repo: Repo, ref: Ref) -> list[int]:
    """The lines of the cited file that hold the anchor's symbol."""
    rx = word_rx(ref.anchor or "")
    return [n for n, line in enumerate(repo.text(ref.path).splitlines(), start=1) if rx.search(line)]


def declares(line: str, word: str) -> bool:
    """Does this line declare or define `word`: a keyword before it (`fn`, `def`, `function`,
    `class`, `const`, ...), a typed constant (`const auto TABLE = ...`), or a definition at
    column 0 such as `json ResourceMcpStatus()`? An `impl` block is not the declaration."""
    w = re.escape(word)
    return bool(re.search(r"\b(?:fn|def|function|class|struct|enum|const|static|type|let|trait|mod)\s+"
                          r"(?:[A-Za-z_]\w*::)*" + w + r"\b", line)
                or re.search(r"\b(?:const|static|constexpr)\s+[\w:<>,*& ]*?\b" + w + r"\s*=", line)
                or re.match(r"^[A-Za-z_][\w:<>,*& ]*?\b" + w + r"\s*\([^;]*$", line))


def moved(repo: Repo, ref: Ref) -> Ref | None:
    """An anchored line whose symbol is no longer on it, cited again where the symbol is now: its
    only line, or else its only declaring line, the range shifted with it and cut at the file's
    end. None when it cannot move, or when the new line is ambiguous: moving to the nearest of
    several occurrences would cite a call as readily as the definition, so a person chooses."""
    if not (ref.lines and ref.anchor) or ref.path not in repo.files:
        return None
    found = anchor_lines(repo, ref)
    a, b = ref.lines
    if not found or any(a <= n <= b for n in found):
        return None
    if len(found) > 1:
        lines = repo.text(ref.path).splitlines()
        found = [n for n in found if declares(lines[n - 1], symbol_word(ref.anchor or ""))]
        if len(found) != 1:
            return None
    to = found[0]
    end = min(to + b - a, len(repo.text(ref.path).splitlines()))
    return Ref(ref.record, ref.line, ref.path, None, "file", (to, end), ref.anchor)


def symbol_word(symbol: str) -> str:
    s = symbol.strip().removesuffix("()")
    s = re.sub(r"<.*>", "", s)
    return s.split("::")[-1].strip()


def ignored(repo: Repo, path: str) -> bool:
    """Does git ignore this path? A directory pattern (`.ai/local/`) matches only a path that names a
    directory, and the directory need not exist in this checkout, so the directory form is asked too:
    the answer is the same on every checkout of one commit."""
    for candidate in (path, path.rstrip("/") + "/"):
        if subprocess.run(["git", "-C", str(repo.root), "check-ignore", "-q", "--no-index", candidate],
                          capture_output=True).returncode == 0:
            return True
    return False


def problem(repo: Repo, ref: Ref, strict_lines: bool = False) -> str | None:
    found = problem_of(repo, ref, strict_lines)
    if found and ref.kind != "tracks" and ref.path not in repo.files and ignored(repo, ref.path):
        return None
    return found


def problem_of(repo: Repo, ref: Ref, strict_lines: bool = False) -> str | None:
    if ref.kind == "tracks":
        rx = glob_regex(ref.path)
        return None if any(rx.match(f) for f in repo.files) else "matches no tracked file"
    if ref.kind == "dir":
        return None if ref.path in repo.dirs else "is not a directory git tracks files in"
    if ref.path not in repo.files:
        return "is not a tracked file"
    if ref.lines:
        return line_problem(repo, ref, strict_lines)
    if ref.symbol:
        word = symbol_word(ref.symbol)
        if not word or not re.search(r"(?<![A-Za-z0-9_])" + re.escape(word) + r"(?![A-Za-z0-9_])",
                                     repo.text(ref.path)):
            return f"does not contain '{word}'"
    return None


def line_problem(repo: Repo, ref: Ref, strict_lines: bool) -> str | None:
    a, b = ref.lines or (0, 0)
    count = len(repo.text(ref.path).splitlines())
    exists = 1 <= a <= b <= count
    if ref.anchor:
        word = symbol_word(ref.anchor)
        found = anchor_lines(repo, ref)
        if exists and any(a <= n <= b for n in found):
            return None
        if not found:
            return f"does not contain '{word}'"
        at = ", ".join(str(n) for n in found[:5]) + (" ..." if len(found) > 5 else "")
        how = "--fix-lines moves the citation" if moved(repo, ref) else "ambiguous: edit the citation"
        return f"does not hold '{word}' there; it is on line {at} ({how})"
    if not strict_lines:
        return None  # a bare line number: only the file is checked
    if not exists:
        return f"has {count} line(s), so line {a if a == b else f'{a}-{b}'} does not exist"
    return "cites a line with no anchor, which no check can verify: cite it as path:line:Symbol"


def fix_lines(repo: Repo, refs: list[Ref]) -> list[tuple[Ref, Ref]]:
    """Rewrite, in each record, every anchored line whose symbol moved; the moves made."""
    moves = [(r, to) for r in refs if (to := moved(repo, r))]
    by_record: dict[str, list[tuple[Ref, Ref]]] = {}
    for old, new in moves:
        by_record.setdefault(old.record, []).append((old, new))
    for record, todo in by_record.items():
        path = repo.root / record
        lines = path.read_text(encoding="utf-8").split("\n")
        for old, new in todo:
            lines[old.line - 1] = lines[old.line - 1].replace(f"`{old.raw}`", f"`{new.label()}`", 1)
        path.write_text("\n".join(lines), encoding="utf-8")
    return moves


def records(repo: Repo, pathspecs: list[str]) -> list[str]:
    out = git(repo.root, "ls-files", "-z", *tracked_sources(), "--", *pathspecs)
    return sorted(p for p in out.split("\0") if p and p.endswith(".md"))


def all_refs(repo: Repo, pathspecs: list[str]) -> list[Ref]:
    return [r for rec in records(repo, pathspecs) for r in scan(repo, rec)]


def cmd_check(repo: Repo, pathspecs: list[str], files: list[str] | None = None,
              strict_lines: bool = False, fix: bool = False) -> int:
    if files:
        refs = [r for f in files for r in scan(repo, f, Path(f).read_text(encoding="utf-8"))]
    else:
        refs = all_refs(repo, pathspecs)
    if fix:
        moves = fix_lines(repo, refs)
        for old, new in moves:
            print(f"MOVED {old.record}:{old.line}: {old.label()} -> {new.label()}")
        done = {id(old): new for old, new in moves}
        refs = [done.get(id(r), r) for r in refs]
    bad = [(r, p) for r in refs if (p := problem(repo, r, strict_lines))]
    for r, p in bad:
        print(f"FAIL {r.record}:{r.line}: {r.label()} {p}")
    recs = len({r.record for r in refs})
    print(f"refs check: {len(refs)} reference(s) in {recs} record(s), {len(bad)} failure(s)")
    return 1 if bad else 0


def build_index(repo: Repo, pathspecs: list[str]) -> dict:
    index: dict[str, dict[str, list[str]]] = {}
    for r in all_refs(repo, pathspecs):
        if r.kind == "tracks":
            rx = glob_regex(r.path)
            for f in sorted(f for f in repo.files if rx.match(f)):
                index.setdefault(f, {}).setdefault(r.record, [])
        else:
            syms = index.setdefault(r.path, {}).setdefault(r.record, [])
            if r.symbol and r.symbol not in syms:
                syms.append(r.symbol)
    return {"schema": "knowledge-refs-index/v1",
            "head": git(repo.root, "rev-parse", "HEAD").strip(),
            "pathspecs": pathspecs,
            "files": {f: {rec: sorted(s) for rec, s in sorted(v.items())}
                      for f, v in sorted(index.items())}}


def cmd_index(repo: Repo, pathspecs: list[str], out: str | None) -> int:
    data = json.dumps(build_index(repo, pathspecs), indent=1, sort_keys=True) + "\n"
    if out:
        Path(out).parent.mkdir(parents=True, exist_ok=True)
        Path(out).write_text(data, encoding="utf-8")
        print(f"refs index: wrote {out}")
    else:
        sys.stdout.write(data)
    return 0


def changed(repo: Repo, rev_range: str) -> dict[str, str]:
    """Changed path -> status letter, renames reported under both names."""
    out = git(repo.root, "diff", "--name-status", "-M", rev_range)
    result: dict[str, str] = {}
    for line in out.splitlines():
        parts = line.split("\t")
        status = parts[0][0]
        if status == "R" and len(parts) == 3:
            result[parts[1]] = "R"
            result[parts[2]] = "R"
        elif len(parts) >= 2:
            result[parts[1]] = status
    return result


def cmd_impact(repo: Repo, rev_range: str, pathspecs: list[str], as_json: bool) -> int:
    diff = changed(repo, rev_range)
    hits: dict[str, dict[str, dict]] = {}
    for r in all_refs(repo, pathspecs):
        if r.kind == "tracks":
            rx = glob_regex(r.path)
            for f, st in diff.items():
                if rx.match(f):
                    hits.setdefault(r.record, {}).setdefault(f, {"status": st, "symbols": [],
                                                                 "via": "tracks"})
        elif r.path in diff or (r.kind == "dir" and any(f.startswith(r.path + "/") for f in diff)):
            keys = [r.path] if r.path in diff else [f for f in diff if f.startswith(r.path + "/")]
            for f in keys:
                entry = hits.setdefault(r.record, {}).setdefault(
                    f, {"status": diff[f], "symbols": [], "via": "reference"})
                entry["via"] = "reference"
                if r.symbol and r.symbol not in entry["symbols"]:
                    entry["symbols"].append(r.symbol)
    # A record whose own text changed in the range was reviewed by whoever changed it.
    result = {rec: v for rec, v in sorted(hits.items()) if rec not in diff}
    if as_json:
        print(json.dumps({"schema": "knowledge-refs-impact/v1", "range": rev_range,
                          "records": result}, indent=1, sort_keys=True))
        return 0
    for rec, files in result.items():
        print(rec)
        for f, e in sorted(files.items()):
            sym = f"  re-read: {', '.join(e['symbols'])}" if e["symbols"] else ""
            print(f"  {e['status']} {f} ({e['via']}){sym}")
    print(f"refs impact {rev_range}: {len(diff)} changed path(s), {len(result)} record(s) to review")
    return 0


def cmd_coverage(repo: Repo, prefixes: list[str], pathspecs: list[str], as_json: bool,
                 strict: bool) -> int:
    """Which tracked files under the prefixes no record's `tracks` attaches to.

    A pattern that is the prefix itself followed by `**` is an umbrella: it attaches to every
    file under the prefix and therefore says nothing about any one of them, so it is not counted.
    """
    prefixes = [p.rstrip("/") + "/" for p in prefixes]
    umbrella = {p + "**" for p in prefixes}
    rxs = [(r.record, glob_regex(r.path)) for r in all_refs(repo, pathspecs)
           if r.kind == "tracks" and r.path.removeprefix(":(glob)") not in umbrella]
    files = sorted(f for f in repo.files if any(f.startswith(p) for p in prefixes))
    covered: dict[str, list[str]] = {}
    for f in files:
        recs = sorted({rec for rec, rx in rxs if rx.match(f)})
        covered[f] = recs
    missing = [f for f, recs in covered.items() if not recs]
    if as_json:
        print(json.dumps({"schema": "knowledge-refs-coverage/v1", "prefixes": prefixes,
                          "files": len(files), "uncovered": missing,
                          "records": {f: r for f, r in covered.items() if r}},
                         indent=1, sort_keys=True))
    else:
        groups: dict[str, list[str]] = {}
        for f in missing:
            groups.setdefault(f.rsplit("/", 1)[0] if "/" in f else ".", []).append(f)
        for d, fs in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            names = " ".join(f.rsplit("/", 1)[-1] for f in fs[:12])
            more = f" (+{len(fs) - 12})" if len(fs) > 12 else ""
            print(f"UNCOVERED {d}/ {len(fs)}: {names}{more}")
        pct = 100.0 * (len(files) - len(missing)) / len(files) if files else 100.0
        print(f"refs coverage {' '.join(prefixes)}: {len(files) - len(missing)}/{len(files)} "
              f"file(s) attached to a record ({pct:.1f}%), {len(missing)} uncovered")
    return 1 if strict and missing else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="refs", description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", default=None, help="repository root (default: git toplevel of cwd)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("check", "index", "impact", "coverage"):
        p = sub.add_parser(name)
        p.add_argument("--pathspec", action="append", help="records to scan (repeatable)")
        if name == "check":
            p.add_argument("--file", action="append",
                           help="check these files instead, tracked or not (an external answer)")
            p.add_argument("--strict-lines", action="store_true",
                           help="a line number must be anchored (path:line:Symbol) and exist")
            p.add_argument("--fix-lines", action="store_true",
                           help="move every anchored line whose symbol moved, in its record")
        if name == "index":
            p.add_argument("--out")
        if name == "impact":
            p.add_argument("range")
            p.add_argument("--json", action="store_true")
        if name == "coverage":
            p.add_argument("prefix", nargs="+", help="directories whose tracked files to measure")
            p.add_argument("--json", action="store_true")
            p.add_argument("--strict", action="store_true", help="exit 1 when a file is uncovered")
    args = ap.parse_args(argv)
    root = Path(args.repo) if args.repo else Path(
        subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True,
                       text=True).stdout.strip())
    repo = Repo.load(root)
    specs = args.pathspec or DEFAULT_PATHSPECS
    if args.cmd == "check":
        if args.fix_lines and args.file:
            ap.error("--fix-lines rewrites records found by --pathspec, not --file")
        return cmd_check(repo, specs, args.file, args.strict_lines, args.fix_lines)
    if args.cmd == "index":
        return cmd_index(repo, specs, args.out)
    if args.cmd == "coverage":
        return cmd_coverage(repo, args.prefix, specs, args.json, args.strict)
    return cmd_impact(repo, args.range, specs, args.json)


if __name__ == "__main__":
    sys.exit(main())
