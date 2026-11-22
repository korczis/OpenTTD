#!/usr/bin/env python3
"""Derive the PrismaticAI settings and Rhai policy tables of tools/research/prismatic-ai/README.md.

RESEARCH-ONLY. Usage:
  python3 -B tools/research/prismatic-ai/derive_docs.py           # rewrite the regions in place
  python3 -B tools/research/prismatic-ai/derive_docs.py --check   # write nothing; exit 1 when stale

Sources, each read as the repository states it, nothing restated by hand:
  - PrismaticAI/info.nut: every AddSetting block (name, description, bounds, step, custom_value
    default, flags; CONFIG_INGAME makes a setting live-changeable in a running game);
  - tools/research/ai-battle/battle.json `tuning`: which settings are genes of the battle's genetic
    algorithm (`space` {name: [min, max, step]}, or `genes` [...] with `narrow` {name: [min, max,
    step]}) and which it pins (`fixed`);
  - tools/research/mcp-panel/rhai/policy/*.rhai: each stage's header comment (first sentence),
    the environment parameters it reads (`env("NAME")`) and the bot settings it names.

The regions are `<!-- prismatic-ai-settings:begin -->` ... `:end -->` and
`<!-- prismatic-ai-policy:begin -->` ... `:end -->`. Registered in surfaces.toml, so
`tools/surfaces/surfaces sync` regenerates them and `tools/surfaces/surfaces check` proves them.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
INFO = HERE / "PrismaticAI" / "info.nut"
README = HERE / "README.md"
BATTLE = ROOT / "tools" / "research" / "ai-battle" / "battle.json"
POLICY = ROOT / "tools" / "research" / "mcp-panel" / "rhai" / "policy"

SETTINGS_REGION = "prismatic-ai-settings"
POLICY_REGION = "prismatic-ai-policy"


def strip_comments(text: str) -> str:
    """Squirrel/Rhai source with // and /* */ comments blanked, strings kept, newlines kept."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c in '"`':  # a Rhai template string (`...`) may hold "//" too
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * text.count("\n", i, j))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def unquote(s: str) -> str:
    return re.sub(r"\\(.)", lambda m: {"n": " ", "t": " "}.get(m.group(1), m.group(1)), s[1:-1])


FIELD_RE = re.compile(r'(\w+)\s*=\s*((?:"(?:[^"\\]|\\.)*"|[^,}\n"])+)')
STRING_RE = re.compile(r'"(?:[^"\\]|\\.)*"')
LOOP_RE = re.compile(r"for\s*\(\s*local\s+(\w+)\s*=\s*(-?\d+)\s*;"
                     r"\s*\1\s*(<=?)\s*(-?\d+)[^)]*\)\s*\{")


def block_end(clean: str, start: int) -> int:
    """Index just past the brace that closes the block whose body starts at `start`."""
    depth, j = 1, start
    while j < len(clean) and depth:
        if clean[j] in '"`':
            q, j = clean[j], j + 1
            while j < len(clean) and clean[j] != q:
                j += 2 if clean[j] == "\\" else 1
        elif clean[j] == "{":
            depth += 1
        elif clean[j] == "}":
            depth -= 1
        j += 1
    return j


def evaluate(expr: str, loop_var: str | None, as_name: bool) -> str:
    """A field value: one literal, or string literals and the loop variable joined by `+`."""
    expr = expr.strip()
    parts = re.split(r'\s*\+\s*(?=(?:[^"]*"[^"]*")*[^"]*$)', expr)
    if len(parts) == 1 and not expr.startswith('"'):
        return expr
    out = []
    for part in parts:
        if part.startswith('"'):
            out.append(unquote(part))
        elif loop_var and part == loop_var:
            out.append("<N>" if as_name else "N")
        else:
            out.append(part)
    return "".join(out)


def parse_settings(text: str) -> list[dict]:
    """Every AddSetting({...}) block of an info.nut, in order, with the line it starts on.

    A block inside `for (local i = A; i <= B; i++)` declares one setting per i: it is listed once,
    its name and description with `<N>` / N for i, and `slots` holding [A, B]."""
    clean = strip_comments(text)
    loops = []
    for lm in LOOP_RE.finditer(clean):
        hi = int(lm.group(4)) - (0 if lm.group(3) == "<=" else 1)
        loops.append((lm.end(), block_end(clean, lm.end()), lm.group(1), int(lm.group(2)), hi))
    settings = []
    for m in re.finditer(r"AddSetting\s*\(\s*\{", clean):
        end = block_end(clean, m.end())
        body = clean[m.end():end - 1]
        loop = next((lp for lp in reversed(loops) if lp[0] <= m.start() < lp[1]), None)
        var = loop[2] if loop else None
        fields = {f.group(1): evaluate(f.group(2), var, f.group(1) == "name")
                  for f in FIELD_RE.finditer(body)}
        if "name" not in fields:
            continue
        flags = {x.strip() for x in fields.get("flags", "").split("|") if x.strip()}

        def num(key, default=None):
            v = fields.get(key)
            try:
                return int(v) if v is not None else default
            except ValueError:
                return v

        settings.append({
            "name": fields["name"],
            "description": fields.get("description", ""),
            "min": num("min_value"),
            "max": num("max_value"),
            "step": num("step_size", 1),
            "default": num("custom_value", num("medium_value")),
            "flags": sorted(flags),
            "live": "CONFIG_INGAME" in flags,
            "boolean": "CONFIG_BOOLEAN" in flags,
            "slots": [loop[3], loop[4]] if loop else None,
            "line": clean.count("\n", 0, m.start()) + 1,
        })
    return settings


def concrete_names(settings: list[dict]) -> set[str]:
    """Every name a bot declares, a loop's names expanded (fr2_from ... fr16_from)."""
    names = set()
    for s in settings:
        if s["slots"]:
            lo, hi = s["slots"]
            names |= {s["name"].replace("<N>", str(i)) for i in range(lo, hi + 1)}
        else:
            names.add(s["name"])
    return names


def parse_genes(battle: dict) -> tuple[dict, dict]:
    """({gene: [min, max, step] or None}, {pinned setting: value}) from battle.json `tuning`."""
    tuning = battle.get("tuning") or {}
    genes: dict = {}
    for name, rng in (tuning.get("space") or {}).items():
        genes[name] = list(rng) if isinstance(rng, (list, tuple)) else None
    for g in tuning.get("genes") or []:
        name = g.get("name") if isinstance(g, dict) else g
        if isinstance(name, str):
            genes.setdefault(name, None)
    for name, rng in (tuning.get("narrow") or {}).items():
        if isinstance(rng, dict):
            rng = [rng.get("min"), rng.get("max"), rng.get("step")]
        genes[name] = list(rng) if isinstance(rng, (list, tuple)) else genes.get(name)
    return genes, dict(tuning.get("fixed") or {})


def header_sentence(text: str) -> str:
    """The first sentence of a script's leading comment block (// lines or one /* */)."""
    lines, block = [], False
    for line in text.splitlines():
        s = line.strip()
        if not lines and not block and (not s or s.startswith("#!")):
            continue
        if block:
            end = "*/" in s
            lines.append(s.split("*/")[0].lstrip("*").strip())
            if end:
                break
        elif s.startswith("/*"):
            rest = s[2:].lstrip("*")
            if "*/" in rest:
                lines.append(rest.split("*/")[0].strip())
                break
            lines.append(rest.strip())
            block = True
        elif s.startswith("//"):
            lines.append(s.lstrip("/").strip())
        else:
            break
    prose = " ".join(x for x in lines if x)
    m = re.search(r"(.+?[.!?])(\s|$)", prose)
    return (m.group(1) if m else prose).strip()


def setting_words(clean: str) -> set[str]:
    """Words that may name a setting: strings ("rich_growth"), properties (s.x), map keys (x: 1)."""
    strings = {unquote(s) for s in STRING_RE.findall(clean)}
    return strings | set(re.findall(r"\.([a-z_][a-z0-9_]*)\b", clean)) \
        | set(re.findall(r"\b([a-z_][a-z0-9_]*)\s*:(?!:)", clean))


def parse_stage(text: str, setting_names: set[str]) -> dict:
    """A policy file: its header's first sentence (the label before ": " dropped), the parameters
    it reads (`params.x`, `params["x"]`, `env("X")`), the settings it names, and its role."""
    clean = strip_comments(text)
    purpose = header_sentence(text)
    m = re.match(r"[^:]*?\)\s*:\s*(.+)", purpose)
    if m:
        purpose = m.group(1)[:1].upper() + m.group(1)[1:]
    params = set(re.findall(r"\bparams\.([A-Za-z_]\w*)", clean)) \
        | set(re.findall(r'\bparams\s*\[\s*"([^"]+)"', clean))
    env = set(re.findall(r'\benv\s*\(\s*"([^"]+)"', clean))
    role = "chain" if re.search(r"\bfn\s+chain\s*\(", clean) else \
        "stage" if re.search(r"\bfn\s+propose\s*\(", clean) else "library"
    return {"purpose": purpose, "params": sorted(params), "env": sorted(env), "role": role,
            "settings": sorted(setting_words(clean) & setting_names)}


def parse_chain(text: str, setting_names: set[str]) -> dict:
    """chain.rhai's stage order, merge rule and, per stage, the settings its params block names."""
    clean = strip_comments(text)
    m = re.search(r"\bstages\s*:\s*\[([^\]]*)\]", clean)
    order = [unquote(s) for s in STRING_RE.findall(m.group(1))] if m else []
    merge = re.search(r'\bmerge\s*:\s*("(?:[^"\\]|\\.)*")', clean)
    pm = re.search(r"\bparams\s*:\s*#\{", clean)
    blocks = {}
    if pm:
        body = clean[pm.end():block_end(clean, pm.end())]
        for name in order + ["common"]:
            bm = re.search(rf"\b{re.escape(name)}\s*:\s*#\{{", body)
            if bm:
                block = body[bm.end():block_end(body, bm.end())]
                blocks[name] = sorted(setting_words(block) & setting_names)
    return {"order": order, "merge": unquote(merge.group(1)) if merge else None, "settings": blocks}


def cell(s) -> str:
    """Table cell text: pipes escaped, and < > so a "<name>" in prose is not read as HTML."""
    return (str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            .replace("|", "\\|").replace("\n", " "))


def render_settings(settings: list[dict], genes: dict, fixed: dict) -> str:
    rows = ["| Setting | Default | Range / step | Live | GA | Description |",
            "|---|---|---|---|---|---|"]
    for s in settings:
        rng = "0 / 1" if s["boolean"] else f"{s['min']} .. {s['max']}"
        if not s["boolean"] and s["step"] not in (None, 1):
            rng += f" / {s['step']}"
        ga = ""
        if s["name"] in fixed:
            ga = f"pinned {fixed[s['name']]}"
        elif s["name"] in genes:
            g = genes[s["name"]]
            ga = "gene"
            if g and len(g) >= 2 and (g[0], g[1]) != (s["min"], s["max"]):
                ga += f" {g[0]} .. {g[1]}"
                if len(g) > 2 and g[2] not in (None, 1):
                    ga += f" / {g[2]}"
        slots = f" (N = {s['slots'][0]} .. {s['slots'][1]})" if s["slots"] else ""
        link = f"[`{s['name']}`](PrismaticAI/info.nut#L{s['line']}){slots}"
        rows.append(f"| {link} | {cell(s['default'])} "
                    f"| {rng} | {'yes' if s['live'] else 'no'} | {ga} | {cell(s['description'])} |")
    names = concrete_names(settings)
    unknown = sorted(set(genes) - names)
    live = len(concrete_names([s for s in settings if s["live"]]))
    out = [f"{len(names)} settings in {len(settings)} declarations, {live} of them live-changeable "
           f"(`CONFIG_INGAME`), {len(names & set(genes))} genes of the battle's genetic algorithm "
           f"(`tuning` in [`battle.json`](../ai-battle/battle.json)). Derived from "
           f"[`PrismaticAI/info.nut`](PrismaticAI/info.nut) by `derive_docs.py`.", ""]
    out += rows
    if unknown:
        out += ["", "Genes the bot does not declare: " + ", ".join(f"`{g}`" for g in unknown) + "."]
    return "\n".join(out)


def render_policy(stages: dict[str, dict], chain: dict | None = None) -> str:
    if not stages:
        return ("No Rhai policy stage yet: `tools/research/mcp-panel/rhai/policy/*.rhai` holds "
                "none. "
                "Each file added there gets a row on the next `surfaces sync`.")
    chain = chain or {"order": [], "merge": None, "settings": {}}
    order = {f"{n}.rhai": i for i, n in enumerate(chain["order"], 1)}
    head = f"{len(stages)} files in `tools/research/mcp-panel/rhai/policy/`"
    if chain["order"]:
        head += (f"; the chain runs {len(chain['order'])} stages in this order"
                 + (f", merged by `{chain['merge']}`" if chain["merge"] else ""))
    out = [head + ". Each row is derived from the file's header comment and source, and the "
           "settings "
           "column also from the stage's `params` block in `chain.rhai`, by `derive_docs.py`.", "",
           "| # | File | Role | Purpose | Parameters it reads | Settings it moves or reads |",
           "|---|---|---|---|---|---|"]
    rank = lambda kv: (order.get(kv[0], len(order) + 1), kv[0])  # noqa: E731
    for name, st in sorted(stages.items(), key=rank):
        sets = sorted(set(st["settings"]) | set(chain["settings"].get(name[:-5], [])))
        reads = [f"`{p}`" for p in st["params"]] + [f"env `{e}`" for e in st["env"]]
        no = order.get(name, "")
        out.append(f"| {no} | [`{name}`](../mcp-panel/rhai/policy/{name}) | {st['role']} "
                   f"| {cell(st['purpose'])} | {', '.join(reads) or 'none'} "
                   f"| {', '.join(f'`{s}`' for s in sets) or 'none'} |")
    return "\n".join(out)


def replace_region(text: str, region: str, body: str) -> str:
    begin, end = f"<!-- {region}:begin -->", f"<!-- {region}:end -->"
    if begin not in text or end not in text:
        raise SystemExit(f"derive_docs: {README.relative_to(ROOT)} lacks the markers "
                         f"{begin} ... {end}")
    a, b = text.index(begin), text.index(end) + len(end)
    return text[:a] + f"{begin}\n\n{body}\n\n{end}" + text[b:]


def derive(readme: str, info: str, battle: dict, stage_texts: dict[str, str]) -> str:
    settings = parse_settings(info)
    genes, fixed = parse_genes(battle)
    names = concrete_names(settings)
    stages = {n: parse_stage(t, names) for n, t in stage_texts.items()}
    chain = parse_chain(stage_texts["chain.rhai"], names) if "chain.rhai" in stage_texts else None
    text = replace_region(readme, SETTINGS_REGION, render_settings(settings, genes, fixed))
    return replace_region(text, POLICY_REGION, render_policy(stages, chain))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Derive the PrismaticAI settings and policy tables.")
    ap.add_argument("--check", action="store_true",
                    help="write nothing; exit 1 when a region is stale")
    args = ap.parse_args(argv)
    battle = json.loads(BATTLE.read_text()) if BATTLE.is_file() else {}
    stages = {p.name: p.read_text() for p in sorted(POLICY.glob("*.rhai"))} \
        if POLICY.is_dir() else {}
    old = README.read_text()
    new = derive(old, INFO.read_text(), battle, stages)
    if args.check:
        if new != old:
            print(f"derive_docs: {README.relative_to(ROOT)} is stale; run "
                  "python3 -B tools/research/prismatic-ai/derive_docs.py", file=sys.stderr)
            return 1
        return 0
    if new != old:
        README.write_text(new)
        print(f"derive_docs: wrote {README.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
