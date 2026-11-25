#!/usr/bin/env python3
"""RESEARCH-ONLY: the LAN AI battle, a headless company-AI tournament on every reachable machine.

Every machine on the local network that answers SSH and has an OpenTTD build becomes an arena. It
runs free-for-all matches between the configured contestant AIs (PrismaticAI from this repository
and AIs from BaNaNaS) with the null video driver, as fast as its CPU allows. The read-only
BattleReferee Game Script logs every company's standing once a quarter. This process schedules
matches onto free arena slots without end, reads their logs over SSH, keeps the leaderboard,
replays an occasional match on a second arena to keep checking that results do not depend on the
machine, and serves a live dashboard. `report` turns the finished matches of the current arena
edition into a record under research/results/, and the leaderboard in research/ai-battle/README.md
is derived from those records (`check` fails when it is not). Full description:
research/ai-battle/README.md.

Commands:
  discover   find the SSH-reachable machines on the local /24 and write the local hosts file
  fetch      download the BaNaNaS contestants and the live-like rivals into the cache (throwaway
             loopback -D server); says which live rival's version the download lacks
  prepare    assemble the arena bundle: base set, AIs, PrismaticAI and referee at a clean commit;
             --referee-only swaps in just the repository's referee (run does it on start)
  probe      show which hosts are usable now (state, build, load, disk; paused and busy hosts)
  run        schedule matches forever and serve the dashboard (SIGTERM / Ctrl-C stops; STOP holds it)
  status     print the current edition's leaderboard
  report     write research/results/battle-<utc>-<edition>.{json,md} and the README leaderboard
  render     rewrite every tracked battle-*.md and ablation-*.md (tables and charts) and the README
             leaderboard from the records' JSON alone; `surfaces sync` runs it, so a renderer change
             reaches every page
  check      validate every battle and ablation record and that every page and the README
             leaderboard is derived (the gate)
  fitness    one short local match of the working tree's PrismaticAI against a reference AI; fails
             on a Squirrel error, an unfinished match or a collapsing fleet (the gate, on change)
  ablate     one local match per PrismaticAI settings variant on one seed and field; --live-like
             plays the owner's live world instead (battle.json live_like: 1950, the seven live
             rivals, the live script budget, seed 15805, the unplanned and the planned arm)
  replay     load a live game's save headless (battle.json replay: 15805's earliest save) with the
             working tree's PrismaticAI (or --rev's) for N game years and fail when the bot repeats
             the live one-train failure (trains, months with at most one train, value vs leader)
  power      the recorded battle's noise (split-half Kendall tau of the GA score, the paired
             deviation across seeds) and the held-start pairs a strategy comparison needs for a
             margin (--margin 1.10); reads research/results only, starts nothing
  live-export    one live game (--game-seed) from a panel's store, read-only (--panel <url> SELECTs
                 through /api/db/query; --db <copy> reads a copy with the duckdb CLI), as
                 research/results/live-<seed>-<run_ms>.json
  live-settings  the next live start's PRISMATIC_BOT_SETTINGS, derived from the newest battle record
                 and the live records (--write: research/results/live-start.json; `render` keeps
                 it current and `check` refuses drift)
  evolve     the config-evolution loop, once: a trigger (--live <live record>, or --panel <url>
             reading gap.<company> and review.<company>.month; --force with --baseline) -> the gap
             attributed per mode -> candidate settings from the recorded battles -> paired
             null-driver matches on battle.json evolve.seeds, the best confirmed on confirm_seeds ->
             a proposal under <cache>/evolve/ (--propose <url>: also the blackboard key
             evolve.proposal.<company>). Applies and approves nothing; bounded by evolve.budget and
             max_minutes, stopped by <cache>/STOP or PAUSED, one at a time (research/config-evolution-loop.md)

Guards (after an incident where `run` kept ~32 matches per host going for three days, drove targets
to load 86 on 18 cores and 35 of 37 GB swap, and nobody on a target could tell whose they were):
  PAUSED     an arena takes no new match while a file `<remote root>/PAUSED*` (default
             $HOME/ttd-battle/PAUSED) exists on it; its first line is shown as the reason. Anyone
             with a shell on the target can pause it: `echo "need the cores" > ~/ttd-battle/PAUSED`.
             Running matches finish on their own (or kill them by <root>/matches/*/*/pid).
  STOP       `run` starts no new match on any arena while `<local root>/STOP` (default
             ~/ttd-battle/STOP) exists on the orchestrator's machine; it logs an event, keeps
             reading the running matches and resumes when the file is gone.
  floor      every probe also reads load average, CPU count, swap and free disk of the remote root
             (one portable shell call). An arena whose load per core is above `max_load_per_core`
             (1.0), whose free disk is under `min_free_disk_gib` (35) or whose used swap is above
             `max_swap_used_gib` (16) is `busy` and takes no match; otherwise new matches are capped
             so the load stays under the ceiling. Defaults: battle.json `resource_floor`; per host:
             `resource_floor` in hosts.json. An unreadable figure never blocks; it is logged.
  ownership  before the first match on an arena, `run` writes `<remote root>/ORCHESTRATOR` (host,
             pid, start time, commit, how to stop it). Every match carries
             TTD_BATTLE_ORCHESTRATOR=<host>:<pid> in its environment (`ps eww`), the wrapper's argv
             names it (`ps` shows `ttd-battle[<host>:<pid>]`), and its match directory holds it in
             a file `orchestrator`.

Exit codes (Majordomus convention): 0 ok, 2 usage, 10 contract unmet, 11 drift, 12 missing.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import http.server
import io
import json
import math
import random
import os
import re
import signal
import socket
import sqlite3
import subprocess
import sys
import tarfile
import threading
import time
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import regimes  # noqa: E402  (failure regimes: research/failure-regimes.md)

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
DEFAULT_CONFIG = HERE / "battle.json"
RESULTS_DIR = REPO / "research" / "results"
README = REPO / "research" / "ai-battle" / "README.md"
BEGIN, END = "<!-- battle:begin -->", "<!-- battle:end -->"
RECORD_SCHEMA = "ai-battle-record/v1"
RECORD_GLOB = "battle-*.json"
# The charts of a record page sit between these markers (rule project.docs-are-derived-and-current:
# `surfaces check` refuses a chart outside a generated region).
CHARTS_BEGIN, CHARTS_END = "<!-- battle-charts:begin -->", "<!-- battle-charts:end -->"
STYLE = REPO / "tools" / "research" / "noirgfx" / "style.toml"  # the NoirGFX palette of every chart

# "REF Q date=<y>-<m> cid=<n> <key>=<int> ... name=<rest of line>" (referee v2 and v3 fields).
REF_LINE = re.compile(r"REF Q date=(\d+)-(\d+) cid=(\d+) (.*?) ?name=(.*)$")
REF_FIELD = re.compile(r"(\w+)=(-?\d+)")
MODES = ("rail", "road", "air", "water")
# An AI's Squirrel error: "dbg: [script:1] [<company>] [S] <text>".
SQ_ERROR = re.compile(r"\[script:\d\] \[(\d+)\] \[S\] ")
# PrismaticAI's quarterly capital report (capital.nut, a warning, so it shows at `-d script=3`):
# "dbg: [script:3] [<company>] [W] PrismaticAI: capital report <year>Q<n>: frozen=<d> days=<d> ...".
CAP_LINE = re.compile(r"\[script:\d\] \[(\d+)\] \[W\] PrismaticAI: capital report (\d+)Q(\d): (.*)$")
# The referee's shared producers: an industry one of whose cargoes rail stations of several companies
# pick up (`cargo=` since 2026-11-23; a line without it counts for the whole industry).
REF_SHARED = re.compile(r"REF S date=(\d+)-(\d+) ind=(\d+)(?: cargo=(\d+))? owners=([\d,]+) name=(.*)$")
REF_TRAINLESS = re.compile(r"REF T date=(\d+)-(\d+) cid=(\d+) rail_stations=(\d+) trainless=(\d+) ids=([\d,]*)")
# PrismaticAI's yearly growth funnel (RESEARCH-ONLY, PrismaticAI/funnel.nut), one line per mode:
# "dbg: [script:1] [<company>] [E] FUNNEL year=<y> mode=<mode> <stage>=<n> ...".
AI_FUNNEL = re.compile(r"\[script:\d\] \[(\d+)\] \[E\] FUNNEL year=(\d+) mode=(\w+) (.*)$")
# Where each mode's funnel loses, by population, so no loss is compared with one counted in other
# units: {mode: [(group, stage, the counts lost, the counts it was lost of), ...]}, each count a sum
# of signed stage counts. Rail discovery is in producers of the last search's census (`served`
# counts cargoes), building in tries, running in lines; road in tries and vehicles; air in
# autopilot calls past its caps (each ends short, with no pair, or tried), tries and vehicles.
FUNNEL_DROPS = {
    "rail": [("discovery", "ours", [("ours", 1)], [("producers", 1)]),
             ("discovery", "claimed", [("claimed", 1)], [("producers", 1)]),
             ("discovery", "served", [("served", 1)], [("producers", 1)]),
             ("discovery", "no_acceptor", [("no_acceptor", 1)], [("producers", 1)]),
             ("build", "stations", [("tried", 1), ("stations", -1)], [("tried", 1)]),
             ("build", "path", [("path_fail", 1)], [("stations", 1)]),
             ("build", "lay", [("stations", 1), ("path_fail", -1), ("laid", -1)], [("stations", 1)]),
             ("build", "given_up", [("laid", 1), ("built", -1)], [("laid", 1)]),
             ("run", "paying", [("lines", 1), ("paying", -1)], [("lines", 1)])],
    "road": [("build", "stop", [("stop_fail", 1)], [("tried", 1)]),
             ("build", "network", [("network_fail", 1)], [("tried", 1)]),
             ("build", "depot", [("depot_fail", 1)], [("tried", 1)]),
             ("build", "vehicle", [("vehicle_fail", 1)], [("tried", 1)]),
             ("run", "paying", [("vehicles", 1), ("paying", -1)], [("vehicles", 1)])],
    "air": [("discovery", "budget", [("short", 1)], [("short", 1), ("no_pair", 1), ("tried", 1)]),
            ("discovery", "pair", [("no_pair", 1)], [("short", 1), ("no_pair", 1), ("tried", 1)]),
            ("build", "build", [("tried", 1), ("built", -1)], [("tried", 1)]),
            ("run", "paying", [("vehicles", 1), ("paying", -1)], [("vehicles", 1)])],
    "ship": [("run", "paying", [("vehicles", 1), ("paying", -1)], [("vehicles", 1)])],
}

SSH_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
            "-o", "ControlMaster=auto", "-o", "ControlPersist=120",
            "-o", "ControlPath=/tmp/ttd-battle-cm-%r@%h:%p"]
DEFAULT_BINARY = "$HOME/dev/OpenTTD/build/openttd"


def log(msg: str) -> None:
    print(time.strftime("%H:%M:%S"), msg, flush=True)


def expand(p: str) -> Path:
    return Path(os.path.expandvars(os.path.expanduser(p)))


def load_config(path: Path) -> dict:
    cfg = json.loads(path.read_text())
    for key in ("cache", "results", "hosts_file", "baseset"):
        cfg[key] = expand(cfg[key])
    t = cfg.get("tuning")
    if t:
        resolve_genes(cfg)
    if t and t.get("enabled"):
        state = tuning_state(cfg)
        cfg["contestants"] = cfg["contestants"] + state["variants"]
    return cfg


# ------------------------------------------------------------------------------- self-tuning (GA)
#
# The gene space is derived from the bot itself: every gene is an AddSetting of PrismaticAI's
# info.nut, and its bounds and step are that setting's min_value, max_value and step_size.
# battle.json's `tuning` declares only the GA's choice of genes (`genes`), a deliberate narrowing
# of a gene's range (`narrow`: {"gene": [min, max, step]}) and the settings pinned for every
# variant (`fixed`). An older configuration's hand-listed `space` is still read as it is.

def strip_squirrel_comments(text: str) -> str:
    """`text` without its /* */, // and # comments; a string literal is kept whole."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c in "\"'":
            j = _string_end(text, i)
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            out.append(" ")
            i = n if j < 0 else j + 2
        elif text.startswith("//", i) or c == "#":
            j = text.find("\n", i)
            i = n if j < 0 else j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def _string_end(text: str, i: int) -> int:
    """The index of the quote closing the string literal that opens at `i`."""
    j = i + 1
    while j < len(text) and text[j] != text[i]:
        j += 2 if text[j] == "\\" else 1
    if j >= len(text):
        raise ValueError(f"unterminated string at offset {i}")
    return j


def _matching(text: str, i: int) -> int:
    """The index of the bracket closing the one at `i` (comments already stripped)."""
    pairs, depth, j = {"{": "}", "(": ")", "[": "]"}, 0, i
    while j < len(text):
        c = text[j]
        if c in "\"'":
            j = _string_end(text, j)
        elif c in pairs:
            depth += 1
        elif c in pairs.values():
            depth -= 1
            if depth == 0:
                return j
        j += 1
    raise ValueError(f"unbalanced {text[i]!r} at offset {i}")


SQ_TOKEN = re.compile(r'\s*(?:("(?:[^"\\]|\\.)*")|(0[xX][0-9a-fA-F]+|\d+)|([A-Za-z_]\w*)|(\S))')
SQ_FOR = re.compile(r"\bfor\s*\(\s*local\s+(\w+)\s*=\s*(-?\d+)\s*;\s*(\w+)\s*(<=|<)\s*(-?\d+)\s*;"
                    r"\s*(\w+)\s*\+\+\s*\)\s*\{")
SQ_ADD = re.compile(r"\bAddSetting\s*\(\s*\{")


def _sq_fields(body: str, env: dict) -> dict:
    """The fields of one AddSetting table, `key = expression` separated by commas or newlines.
    An expression joins string literals, integers, loop variables and flag names with + - |;
    a flag name stands for itself (a frozenset), so `CONFIG_INGAME | CONFIG_BOOLEAN` is a set."""
    toks = []
    for m in SQ_TOKEN.finditer(body):
        s, num, ident, op = m.groups()
        if s is not None:
            try:
                toks.append(("v", json.loads(s)))
            except ValueError:
                toks.append(("v", s[1:-1]))
        elif num is not None:
            toks.append(("v", int(num, 0)))
        elif ident is not None:
            toks.append(("i", ident))
        elif op is not None:
            toks.append(("o", op))
    pos = 0

    def term():
        nonlocal pos
        if pos >= len(toks):
            raise ValueError("AddSetting: expression expected")
        kind, x = toks[pos]
        pos += 1
        if (kind, x) == ("o", "-"):
            return -term()
        if kind == "o":
            raise ValueError(f"AddSetting: unexpected {x!r}")
        return env.get(x, frozenset([x])) if kind == "i" else x

    out = {}
    while pos < len(toks):
        if toks[pos] == ("o", ","):
            pos += 1
            continue
        if toks[pos][0] != "i" or toks[pos + 1:pos + 2] != [("o", "=")]:
            raise ValueError(f"AddSetting: unexpected {toks[pos][1]!r}")
        key, pos = toks[pos][1], pos + 2
        val = term()
        while pos < len(toks) and toks[pos][0] == "o" and toks[pos][1] in "+-|":
            op = toks[pos][1]
            pos += 1
            val = _sq_apply(val, op, term())
        out[key] = val
    return out


def _sq_apply(val, op: str, x):
    if op == "+":
        return str(val) + str(x) if isinstance(val, str) or isinstance(x, str) else val + x
    if op == "-":
        return val - x
    as_set = (lambda v: v if isinstance(v, frozenset) else frozenset() if v == 0 else None)
    a, b = as_set(val), as_set(x)
    return int(val) | int(x) if a is None or b is None else a | b


def _sq_settings(text: str, env: dict, out: dict) -> None:
    pos = 0
    while True:
        found = [m for m in (SQ_FOR.search(text, pos), SQ_ADD.search(text, pos)) if m]
        if not found:
            return
        m = min(found, key=lambda x: x.start())
        end = _matching(text, m.end() - 1)
        body = text[m.end():end]
        if m.re is SQ_FOR:
            var, lo, cond_var, cmp, hi, inc_var = m.groups()
            if not var == cond_var == inc_var:
                raise ValueError(f"unsupported loop: {m.group(0)}")
            for v in range(int(lo), int(hi) + (cmp == "<=")):
                _sq_settings(body, {**env, var: v}, out)
        else:
            f = _sq_fields(body, env)
            name = f.get("name")
            if not isinstance(name, str):
                raise ValueError(f"AddSetting without a literal name: {body[:60].strip()!r}")
            if name in out:
                raise ValueError(f"{name}: declared twice")
            flags = f.get("flags", frozenset())
            flags = flags if isinstance(flags, frozenset) else frozenset()
            boolean = "CONFIG_BOOLEAN" in flags
            try:
                lo_v = f.get("min_value", 0) if boolean else f["min_value"]
                hi_v = f.get("max_value", 1) if boolean else f["max_value"]
            except KeyError as exc:
                raise ValueError(f"{name}: no {exc.args[0]}") from None
            out[name] = {"min": lo_v, "max": hi_v, "step": f.get("step_size", 1),
                         "default": f.get("custom_value", f.get("medium_value", lo_v)),
                         "boolean": boolean, "ingame": "CONFIG_INGAME" in flags}
        pos = end + 1


def info_settings(path: Path) -> dict:
    """Every AddSetting({...}) of an AI's info.nut as {name: {min, max, step, default, boolean,
    ingame}}: step is step_size (1 when absent), default is custom_value, boolean and ingame are
    CONFIG_BOOLEAN and CONFIG_INGAME in its flags. Settings declared in a counted loop
    (`for (local i = 1; i <= 24; i++)`) are expanded, so "line" + i + "_trains" gives
    line1_trains..."""
    out: dict = {}
    _sq_settings(strip_squirrel_comments(Path(path).read_text()), {}, out)
    return out


def info_nut_path(cfg: dict) -> Path:
    # The checkout this file belongs to, not REPO: a check that runs with REPO pointed elsewhere
    # (the record tests use a scratch directory) still reads the configuration's own bot.
    bot = cfg.get("prismatic", {}).get("path", "tools/research/prismatic-ai/PrismaticAI")
    return HERE.parents[2] / bot / "info.nut"


def gene_space(t: dict, settings: dict) -> dict:
    """The GA's effective gene space {gene: [min, max, step]}, in the order of `genes`: info.nut's
    bounds and step, overridden by `narrow`. A configuration with a hand-listed `space` keeps it."""
    if "space" in t:
        return {k: list(v) for k, v in t["space"].items()}
    narrow = t.get("narrow", {})
    out = {}
    for g in t.get("genes", []):
        if g in narrow:
            out[g] = list(narrow[g])
        elif g in settings:
            s = settings[g]
            out[g] = [s["min"], s["max"], s["step"]]
        else:
            raise ValueError(f"gene {g} is not a setting of info.nut")
    return out


def gene_space_errors(t: dict, settings: dict) -> list[str]:
    """What makes `tuning` disagree with the bot: a gene, pinned (`fixed`) or `start` setting that
    info.nut does not declare, a gene that is pinned too (a bug-fix switch is not a gene), a
    `narrow` for a non-gene, and a narrow (or legacy `space`) range outside info.nut's bounds."""
    legacy = "space" in t
    genes = list(t["space"]) if legacy else list(t.get("genes", []))
    errs = []
    if legacy and "genes" in t:
        errs.append("declares both `space` and `genes`")
    if len(set(genes)) != len(genes):
        errs.append("a gene is listed twice")
    for g in genes:
        if g not in settings:
            errs.append(f"{g}: gene is not a setting of info.nut")
    fixed = t.get("fixed", {})
    for k in fixed:
        if k not in settings:
            errs.append(f"{k}: fixed setting is not a setting of info.nut")
        elif k in genes:
            errs.append(f"{k}: pinned in `fixed` and a gene as well")
    for k in t.get("start", {}):
        if k not in genes:
            errs.append(f"{k}: `start` value for a setting that is not a gene")
    label = "space" if legacy else "narrow"
    for k, b in (t["space"] if legacy else t.get("narrow", {})).items():
        if not legacy and k not in genes:
            errs.append(f"{k}: narrow for a setting that is not a gene")
            continue
        if k not in settings:
            continue
        s = settings[k]
        ok = (isinstance(b, list) and len(b) == 3 and all(isinstance(x, int) for x in b)
              and b[2] > 0 and s["min"] <= b[0] <= b[1] <= s["max"])
        if not ok:
            errs.append(f"{k}: {label} {b} is not [min, max, step > 0] within info.nut's "
                        f"[{s['min']}, {s['max']}]")
    return errs


def config_errors(cfg: dict) -> list[str]:
    """`battle.py check`'s view of battle.json (as read, unexpanded): its GA genes against
    PrismaticAI's info.nut."""
    t = cfg.get("tuning")
    if not t:
        return []
    path = info_nut_path(cfg)
    try:
        settings = info_settings(path)
    except (OSError, ValueError) as exc:
        return [f"{path.relative_to(REPO) if path.is_relative_to(REPO) else path}: {exc}"]
    return [f"battle.json tuning: {e} (ADR 0036)" for e in gene_space_errors(t, settings)]


def resolve_genes(cfg: dict) -> None:
    """Sets `tuning.space` to the effective gene space and gives a gene without a `start` value
    info.nut's default. Fails fast on a configuration that `check` refuses."""
    t = cfg["tuning"]
    errs = config_errors(cfg)
    if errs:
        raise SystemExit("\n".join(errs))
    settings = info_settings(info_nut_path(cfg))
    t["space"] = gene_space(t, settings)
    start = t.setdefault("start", {})
    for g in t["space"]:
        if g not in start:
            start[g] = settings[g]["default"]
    # What a live game's bot ran with for a gene its settings do not name (live_genes).
    t["defaults"] = {g: settings[g]["default"] for g in t["space"] if g in settings}

def paired_vs(matches: list[dict], base: str) -> list[dict]:
    """Every configuration against `base` on the same seeds (common random numbers): pairs, mean
    value ratio, wins. A configuration is its settings, not its label: the GA's elite keeps its
    genes but takes a new label every generation, and only pooled by settings do its pairs add up.
    The paired comparison decides a default, not a GA score over a few matches."""
    by_seed: dict[int, dict[str, int]] = {}
    label_of: dict[str, str] = {}
    base_id = None
    for m in sorted(matches, key=lambda m: m.get("started_at", 0)):
        if m.get("status") != "done" or m.get("replica_of"):
            continue
        ident = {c["key"]: (c.get("ai", c["key"]) + "|" + c.get("settings", "")) for c in m.get("lineup", [])}
        for f in m.get("standings") or []:
            cid = ident.get(f["key"], f["key"])
            label_of[cid] = f["key"]                       # the newest label of that configuration
            if f["key"] == base:
                base_id = cid
            by_seed.setdefault((m.get("edition", ""), m["seed"]), {})[cid] = f["value"]
    stats: dict[str, list] = {}
    for vals in by_seed.values():
        if base_id not in vals:
            continue
        for k, v in vals.items():
            if k != base_id:
                stats.setdefault(k, []).append((v, vals[base_id]))
    out = []
    for k, ps in stats.items():
        ratios = [v / max(1, b) for v, b in ps]
        logs = [math.log(max(1, v) / max(1, b)) for v, b in ps]
        geo, ci_low = log_ratio_ci(logs, k)
        out.append({"contestant": label_of.get(k, k), "settings": k.split("|", 1)[1] if "|" in k else "",
                    "pairs": len(ps), "ratio": round(sum(ratios) / len(ratios), 3),
                    "wins": sum(v > b for v, b in ps), "geo": geo, "ci_low": ci_low})
    return sorted(out, key=lambda r: -r["ratio"])


def log_ratio_ci(logs: list[float], key: str, resamples: int = 2000) -> tuple[float, float]:
    """Geometric mean of the paired value ratios and the lower end of its 95% bootstrap interval
    (percentile, deterministic: the resampling is seeded by the configuration). The log ratio,
    not the win count, carries the size of each pair's difference: on 40 pairs the GA's elite
    won 25 (62%) yet its interval was 1.03-1.21."""
    if len(logs) < 2:
        return (round(math.exp(logs[0]), 3) if logs else 1.0), 0.0
    rng = random.Random(key)
    n = len(logs)
    means = sorted(sum(rng.choices(logs, k=n)) / n for _ in range(resamples))
    return round(math.exp(sum(logs) / n), 3), round(math.exp(means[int(0.025 * resamples)]), 3)


def promotions(matches: list[dict], rules: dict) -> list[dict]:
    """Candidates for the bot's next default: a PrismaticAI configuration paired with the default on
    common seeds, with a mean value ratio above `min_ratio`, and either
    - at least `min_pairs` pairs, winning at least `min_win_share` of them (a clear winner), or
    - at least `min_pairs_ci` pairs, with the lower end of the 95% bootstrap interval of its
      geometric mean ratio above `min_ci_low` (a steady gain in a noisy five-AI field, where an
      80% win share is out of reach for a +10-15% configuration).
    Proposed, never applied: a default changes through a commit."""
    base = rules.get("base", "PrismaticAI")
    out = []
    for r in paired_vs(matches, base):
        if not r["contestant"].startswith("PrismaticAI") or r["ratio"] <= rules.get("min_ratio", 1.05):
            continue
        clear = r["pairs"] >= rules.get("min_pairs", 16) and r["wins"] >= rules.get("min_win_share", 0.8) * r["pairs"]
        steady = ("min_pairs_ci" in rules and r["pairs"] >= rules["min_pairs_ci"]
                  and r["ci_low"] > rules.get("min_ci_low", 1.0))
        if clear or steady:
            out.append(dict(r, base=base, why="win share" if clear else "interval"))
    return out


def tuning_path(cfg: dict) -> Path:
    return cfg["cache"] / "tuning" / "state.json"


def settings_str(genes: dict) -> str:
    return ",".join(f"{k}={v}" for k, v in sorted(genes.items()))


def variant(t: dict, gen: int, i: int, genes: dict, parents: list) -> dict:
    # Every gene is clamped into the current space, so narrowing a range in battle.json (say
    # own_fail_pause 0 -> 1) also reaches the elite and children inherited from older generations.
    genes = {k: clamp_gene(t["space"], k, v) if k in t["space"] else v for k, v in genes.items()}
    return {"ai": t.get("ai", "PrismaticAI"), "label": f"{t['label_prefix']} g{gen}.{i}",
            "group": t["group"], "source": "repository", "genes": genes, "parents": parents,
            "settings": settings_str({**t.get("fixed", {}), **genes})}


def clamp_gene(space: dict, k: str, v: float) -> int:
    lo, hi, step = space[k]
    return int(min(hi, max(lo, lo + round((v - lo) / step) * step)))


def first_generation(t: dict) -> list[dict]:
    rng = random.Random(f"{t['seed']}:0")
    out = [variant(t, 0, 0, dict(t["start"]), [])]
    while len(out) < t["population"]:
        out.append(variant(t, 0, len(out), mutate(t, dict(t["start"]), rng, 1.0), []))
    return out


def mutate(t: dict, genes: dict, rng: random.Random, rate: float | None = None) -> dict:
    rate = t.get("mutation_rate", 0.35) if rate is None else rate
    out = dict(genes)
    for k, (lo, hi, step) in t["space"].items():
        if rng.random() < rate:
            span = max(1, (hi - lo) // step)
            out[k] = clamp_gene(t["space"], k, out.get(k, lo) + rng.choice([-1, 1]) *
                                step * rng.randint(1, max(1, span // 4)))
    return out


def brain_proposals(t: dict) -> list[dict]:
    """Up to `brain_offspring` proposals from brain.json (schema battle-brain/v1) younger than
    `brain_max_age_s`, as genes inside the declared space. Nothing when absent, stale or invalid."""
    n = int(t.get("brain_offspring", 0))
    path = expand(t.get("brain_file", "~/ttd-battle/results/brain.json"))
    if n <= 0 or not path.is_file() or time.time() - path.stat().st_mtime > t.get("brain_max_age_s", 7200):
        return []
    try:
        data = json.loads(path.read_text())
        if data.get("schema") != "battle-brain/v1":
            return []
        out = []
        for p in data.get("proposals", [])[:n]:
            kv = dict(x.split("=", 1) for x in str(p.get("settings", "")).split(",") if "=" in x)
            genes = {k: clamp_gene(t["space"], k, float(kv.get(k, t["start"].get(k, t["space"][k][0]))))
                     for k in t["space"]}
            out.append((genes, str(p.get("kind", "")).capitalize()))
        return out
    except (ValueError, KeyError, TypeError):
        return []


def tuning_state(cfg: dict) -> dict:
    p = tuning_path(cfg)
    if p.is_file():
        return json.loads(p.read_text())
    t = cfg["tuning"]
    state = {"generation": 0, "variants": first_generation(t), "history": []}
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(state, indent=1) + "\n")
    return state


def fitness_of(key: str, matches: list[dict], last_penalty: float = 0.0) -> dict:
    """Mean normalised points per match (1 = won, 0 = last), mean log10 of value / winner's value,
    and the share of matches it ended last, over the contestant's scored matches. The score is
    points + 0.1 x log gap - last_penalty x last share: an explorer that ends last is culled fast."""
    pts, logs, lasts = [], [], 0
    for m in matches:
        if m.get("status") != "done" or m.get("replica_of"):
            continue
        st = m["standings"]
        for rank, f in enumerate(st):
            if f["key"] == key:
                pts.append((len(st) - 1 - rank) / max(1, len(st) - 1))
                logs.append(math.log10(max(1, f["value"]) / max(1, st[0]["value"])))
                lasts += rank == len(st) - 1
    n = len(pts)
    return {"matches": n, "points": round(sum(pts) / n, 4) if n else 0.0,
            "log_gap": round(sum(logs) / n, 4) if n else 0.0,
            "last_share": round(lasts / n, 4) if n else 0.0,
            "score": round(sum(pts) / n + 0.1 * sum(logs) / n - last_penalty * lasts / n, 4) if n else -9.0}


def next_generation(t: dict, state: dict, scored: list[tuple[dict, dict]],
                    live: list[dict] | None = None) -> list[dict]:
    """Elitism (the best survives unchanged), then children of two tournament-chosen parents by
    uniform crossover and mutation. Deterministic per generation. With live evidence (`live`, the
    entries of live_entries): a variant that is a configuration which lost a live game is neither
    the elite nor a parent (live_veto), and the best paired live configuration not yet tried in the
    arena joins as a child (live_seeds)."""
    gen = state["generation"] + 1
    rng = random.Random(f"{t['seed']}:{gen}")
    ranked = [v for v, _ in sorted(scored, key=lambda x: -x[1]["score"])]
    live = live or []
    allowed = [v for v in ranked if not live_veto(t, v["genes"], live)]
    # Every variant vetoed: no elite this generation, and the children breed from the whole field.
    out = [variant(t, gen, 0, dict(allowed[0]["genes"]), [allowed[0]["label"]])] if allowed else []
    ranked = allowed or ranked

    def pick():
        a, b = rng.sample(ranked[:max(2, len(ranked))], 2) if len(ranked) > 1 else (ranked[0], ranked[0])
        return a if ranked.index(a) < ranked.index(b) else b
    # Model-guided offspring: the top proposals of tools/research/battle-brain (an Axon/EXLA ensemble
    # trained on every PrismaticAI result) join as children, clamped into the gene space.
    # A proposal's "kind" (safe: best lower confidence bound; explore: best upper) names it.
    used = set()
    for k, (genes, kind) in enumerate(brain_proposals(t)):
        if len(out) >= t["population"]:
            break
        tag = f"brain{kind}" if kind else f"brain{k}"
        if tag in used:  # two proposals of one kind: labels must stay unique, they are contestant keys
            tag = f"{tag}{k}"
        used.add(tag)
        out.append(variant(t, gen, len(out), genes, [tag]))
        out[-1]["label"] = f"{t['label_prefix']} g{gen}.{tag}"
    tried = [v["genes"] for h in state.get("history", []) for v in h["variants"]] + [v["genes"] for v in ranked]
    for k, (genes, source) in enumerate(live_seeds(t, live, tried)):
        if len(out) >= t["population"]:
            break
        out.append(variant(t, gen, len(out), genes, [source]))
        out[-1]["label"] = f"{t['label_prefix']} g{gen}.live{k or ''}"
    while len(out) < t["population"]:
        p1, p2 = pick(), pick()
        child = {k: (p1["genes"] if rng.random() < 0.5 else p2["genes"]).get(k, t["start"].get(k))
                 for k in t["space"]}
        out.append(variant(t, gen, len(out), mutate(t, child, rng), [p1["label"], p2["label"]]))
    return out


# --------------------------------------------------------------------------- live outcome learning
#
# The owner's live games are recorded by their panel in a DuckDB store. `live-export` turns one game
# into research/results/live-<seed>-<run_ms>.json from that store alone (schema ai-live-outcome/v1),
# and everything below reads only tracked records, so each derived figure is reproducible from the
# repository (rule project.battle-records-are-reproducible). The evidence enters the GA as a veto
# and a seed, not as a score term: a configuration that lost a live game is never the elite or a
# parent again, and the best configuration of a paired live comparison is played in the arena.
# research/ai-battle/README.md ("Live outcomes") describes the rule and its reasons.

LIVE_SCHEMA = "ai-live-outcome/v1"
LIVE_GLOB = "live-[0-9]*.json"
LIVE_SETTINGS = "live-start.json"
LIVE_SETTINGS_SCHEMA = "ai-live-settings/v1"
# Settings that name one map's objects (the planner's industry slots, per-line train counts, fixed
# town and airport pairs) are instance data, not strategy: dropped from every live configuration.
MAP_SETTING = re.compile(r"^(fr\d*_(from|to|cargo)|line\d+_(trains|wagons)|rail_[ab]|air_[ab])$")
LIVE_DEFAULTS = {"reference": "AAAHogEx", "horizon_years": 4, "min_years": 2, "loss_ratio": 0.5,
                 "paired_margin": 0.8, "veto_genes": 0, "loss_veto_genes": 3, "seed_offspring": 1}
START_AI = re.compile(r'^start_ai\s+(?:"([^"]+)"|(\S+))(?:\s+(\S*=\S*))?')
SET_SETTING = re.compile(r"^openttd\.ai\.set_setting\((\{.*?\})\)(?:.*?approval #(\d+))?")
WATCHDOG = re.compile(r"(train|road|ship|aircraft|stations) (\d+)")
LIVE_SQL = {
    "runs": "SELECT m.instance, m.seed, m.run_ms, min(m.month) AS first_month, max(m.month) AS last_month "
            "FROM company_month m GROUP BY ALL ORDER BY m.run_ms",
    "edges": "SELECT run_ms, month, name FROM company_month WHERE company = 0 QUALIFY "
             "month = min(month) OVER (PARTITION BY run_ms) OR month = max(month) OVER (PARTITION BY run_ms)",
    "series": "SELECT run_ms, company, name, month, value, money, loan, vehicles FROM company_month "
              "WHERE run_ms IN ({runs}) ORDER BY run_ms, company, month",
    "events": "SELECT run_ms, seq, game_date, kind, data->>'text' AS text, CAST(data AS VARCHAR) AS data "
              "FROM run_events WHERE run_ms IN ({runs}) AND (kind IN ('company.founded', 'decision.recorded') OR (kind IN "
              "('panel.ai', 'panel.console') AND (data->>'text') LIKE 'start_ai%') OR (kind = 'panel.mcp' "
              "AND (data->>'text') LIKE 'openttd.ai.set_setting(%')) ORDER BY run_ms, seq",
    "watchdog": "SELECT data->>'key' AS key, CAST(data->'value' AS VARCHAR) AS value FROM run_events "
                "WHERE run_ms IN ({runs}) AND kind = 'rhai.shared' AND (data->>'key') LIKE 'watchdog.%' "
                "QUALIFY row_number() OVER (PARTITION BY data->>'key' ORDER BY run_ms DESC, seq DESC) = 1",
    "revision": "SELECT CAST(version AS VARCHAR) AS version FROM plan_decisions WHERE run_ms IN ({runs}) "
                "ORDER BY run_ms, seq LIMIT 1",
}


def panel_query(url: str):
    """A query runner over the panel's read-only SQL endpoint (POST /api/db/query, SELECT only)."""
    import urllib.request

    def run(name: str, sql: str) -> list[dict]:
        req = urllib.request.Request(url.rstrip("/") + "/api/db/query", data=json.dumps({"sql": sql}).encode(),
                                     headers={"content-type": "application/json", "origin": url.rstrip("/")})
        with urllib.request.urlopen(req, timeout=180) as r:
            d = json.loads(r.read())
        if d.get("error") or d.get("capped"):
            raise SystemExit(f"live-export: query {name}: {d.get('error') or 'result capped by the panel'}")
        return [dict(zip(d["columns"], row)) for row in d["rows"]]
    return run


def duckdb_query(path: Path):
    """A query runner over a copy of the store through the duckdb CLI, opened read-only."""
    def run(name: str, sql: str) -> list[dict]:
        p = subprocess.run(["duckdb", "-readonly", "-json", str(path), "-c", sql],
                           capture_output=True, text=True)
        if p.returncode:
            raise SystemExit(f"live-export: query {name}: {p.stderr.strip()}")
        return json.loads(p.stdout or "[]")
    return run


def next_month(month: str) -> str:
    y, m = int(month[:4]), int(month[5:7])
    return f"{y + m // 12}-{m % 12 + 1:02d}"


def settings_dict(text: str) -> dict:
    """A "k=v,k=v" settings text as {k: int}, without the map's own settings (MAP_SETTING)."""
    out = {}
    for kv in (text or "").split(","):
        k, _, v = kv.partition("=")
        if k.strip() and not MAP_SETTING.match(k.strip()) and re.fullmatch(r"-?\d+", v.strip()):
            out[k.strip()] = int(v.strip())
    return out


def game_runs(runs: list[dict], edges: list[dict], seed: int) -> list[dict]:
    """The runs of one game: the newest run started with `seed` (a fresh start under the same seed
    replaces the older one), then every later run of the same panel whose first month continues the
    previous run's last month under the same company 0 name (a `--load` gets a new run seed)."""
    name = {(e["run_ms"], e["month"]): e["name"] for e in edges}
    own = [r for r in runs if r["seed"] == seed]
    if not own:
        return []
    chain = [own[-1]]
    for r in runs:
        last = chain[-1]
        if (r["run_ms"] > last["run_ms"] and r["instance"] == last["instance"]
                and r["first_month"] in (last["last_month"], next_month(last["last_month"]))
                and name.get((r["run_ms"], r["first_month"])) == name.get((last["run_ms"], last["last_month"]))):
            chain.append(r)
    return chain


def live_record(query, seed: int, source: str) -> dict:
    """The live outcome record of the game played with map seed `seed`, from the panel store alone
    (`query(name, sql)` runs LIVE_SQL[name]). Raises SystemExit when the store has no such game."""
    chain = game_runs(query("runs", LIVE_SQL["runs"]), query("edges", LIVE_SQL["edges"]), seed)
    if not chain:
        raise SystemExit(f"live-export: no run with seed {seed} in the store")
    ids = ",".join(str(r["run_ms"]) for r in chain)
    fetched = {k: query(k, LIVE_SQL[k].format(runs=ids)) for k in ("series", "events", "watchdog", "revision")}
    series: dict[str, dict[str, dict]] = {}
    names: dict[int, str] = {}
    for row in fetched["series"]:          # a later run's month replaces an earlier run's
        series.setdefault(str(row["company"]), {})[row["month"]] = {
            k: int(row[k]) for k in ("value", "money", "loan", "vehicles")}
        names[int(row["company"])] = row["name"]
    yearly = {}
    for c, months in series.items():
        last_of_year = {}
        for m in sorted(months):
            last_of_year[m[:4]] = m
        yearly[c] = [dict(month=m, **months[m]) for m in sorted(last_of_year.values())]
    founded, starts, requests, enacted = [], [], {}, {}
    for ev in fetched["events"]:
        if ev["kind"] == "company.founded":
            cid = json.loads(ev["data"])["id"]
            if cid not in founded:
                founded.append(cid)
        elif ev["kind"] == "decision.recorded":
            # The approval server's verdict on an approval of this run: what it enacted, and when.
            d = json.loads(ev["data"])
            if d.get("action_type") == "ai.set_setting" and d.get("command_result") == "succeeded":
                enacted[(ev["run_ms"], int(d["approval_id"]))] = ev["game_date"]
        elif ev["kind"] == "panel.mcp":
            m = SET_SETTING.match(ev["text"])
            args = json.loads(m.group(1))
            requests.setdefault(int(args["company"]), []).append(
                {"date": ev["game_date"], "name": args["name"], "value": int(args["value"]),
                 "key": (ev["run_ms"], int(m.group(2))) if m.group(2) else None})
        else:
            m = START_AI.match(ev["text"])
            starts.append((m.group(1) or m.group(2), m.group(3) or ""))
    ai_of = dict(zip(founded, starts))
    companies = []
    for cid in sorted(names):
        ai, text = ai_of.get(cid, (None, ""))
        start = settings_dict(text)
        current, changes = dict(start), []
        for r in requests.get(cid, []):    # only a request that would change a setting counts
            if not MAP_SETTING.match(r["name"]) and current.get(r["name"]) != r["value"]:
                # Applied: the approval it waited for was enacted (decision.recorded, succeeded).
                applied = enacted.get(r.pop("key"))
                if applied:
                    current[r["name"]] = r["value"]
                changes.append(dict(r, applied=applied))
        companies.append({"company": cid, "name": names[cid], "ai": ai,
                          "settings_start": settings_str(start), "settings_requested": changes})
    mode_mix = {}
    for w in fetched["watchdog"]:
        cid = w["key"].split(".", 1)[1]
        if cid.isdigit():                           # watchdog.<company>; other keys are its own state
            sig = json.loads(w["value"]).get("signature", "")
            mode_mix[cid] = {k: int(v) for k, v in WATCHDOG.findall(sig)}
    rev = fetched["revision"][0]["version"] if fetched["revision"] else None
    first = min(m for months in series.values() for m in months)
    final = max(m for months in series.values() for m in months)
    digest = hashlib.sha256(json.dumps(fetched, sort_keys=True, default=str).encode()).hexdigest()
    rec = {"schema": LIVE_SCHEMA, "id": f"live-{seed}-{chain[0]['run_ms']}",
           "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
           "source": source, "instance": chain[0]["instance"], "seed": seed,
           "runs": [{k: r[k] for k in ("run_ms", "seed", "first_month", "last_month")} for r in chain],
           "revision": json.loads(rev).get("revision") if rev else None, "inputs_sha256": digest,
           "first_month": first, "final_month": final, "companies": companies, "series": yearly,
           "mode_mix": mode_mix or None}
    rec["outcome"] = live_outcome(rec)
    return rec


def live_outcome(rec: dict, reference: str = LIVE_DEFAULTS["reference"]) -> dict:
    """Leader, reference and every PrismaticAI company's final standing, from the record's series."""
    final = {c["company"]: rec["series"][str(c["company"])][-1]["value"] for c in rec["companies"]}
    leader = max(final, key=lambda c: (final[c], -c))
    ref = next((c["company"] for c in rec["companies"] if c["ai"] == reference), None)
    ours = {}
    for c in rec["companies"]:
        if c["ai"] == "PrismaticAI":
            v = final[c["company"]]
            ours[str(c["company"])] = {
                "value": v, "rank": 1 + sum(x > v for x in final.values()),
                "share_of_leader": round(v / max(1, final[leader]), 4),
                "ratio_to_reference": round(v / max(1, final[ref]), 4) if ref is not None else None}
    return {"leader": leader, "leader_value": final[leader], "reference": reference,
            "reference_company": ref, "prismatic": ours}


def validate_live(rec: dict, name: str) -> list[str]:
    errs = [f"{name}: missing {k}" for k in ("schema", "id", "seed", "runs", "companies", "series",
                                              "first_month", "final_month", "outcome") if k not in rec]
    if errs:
        return errs
    if rec["schema"] != LIVE_SCHEMA:
        errs.append(f"{name}: schema {rec['schema']} is not {LIVE_SCHEMA}")
    if name != f"{rec['id']}.json" or not rec["id"].startswith(f"live-{rec['seed']}-"):
        errs.append(f"{name}: id {rec['id']} does not name the file and the seed")
    for c in rec["companies"]:
        if not rec["series"].get(str(c["company"])):
            errs.append(f"{name}: company {c['company']} has no series")
    if not errs and live_outcome(rec, rec["outcome"].get("reference", "AAAHogEx")) != rec["outcome"]:
        errs.append(f"{name}: outcome is not derived from its series")
    return errs


def live_cfg(t: dict) -> dict:
    return {**LIVE_DEFAULTS, **t.get("live", {})}


def live_genes(t: dict, settings: dict) -> dict:
    """A configuration's genes in the GA's space: a gene the settings do not name is what the bot ran
    with, info.nut's default (t["defaults"], set by resolve_genes)."""
    d = t.get("defaults", t["start"])
    return {g: clamp_gene(t["space"], g, settings.get(g, d.get(g, t["space"][g][0]))) for g in t["space"]}


def live_entries(records: list[dict], t: dict) -> list[dict]:
    """Every PrismaticAI company of every live record, judged at a year end against the reference
    AI. A company is judged on one configuration only: its end is the last year end before its first
    applied setting change (a request the approval server enacted; one never enacted changed
    nothing), at most `horizon_years` game years in and at least `min_years`. The companies of one
    game share the earliest of their ends, so they are compared at the same date. Lost: in a game
    with several judged PrismaticAI configurations, worth less than paired_margin x the best of them
    (vetoes genes within `veto_genes`); alone, worth less than loss_ratio x the reference AI (a loss
    to the rival itself, which vetoes genes within `loss_veto_genes`)."""
    lc = live_cfg(t)
    out = []
    for rec in records:
        y0 = int(rec["first_month"][:4])
        ref = next((c["company"] for c in rec["companies"] if c["ai"] == lc["reference"]), None)
        ref_v = {p["month"]: p["value"] for p in rec["series"].get(str(ref), [])}
        game = []
        for c in rec["companies"]:
            if c["ai"] != "PrismaticAI":
                continue
            # A request that was never enacted (unanswered, expired, denied) changed nothing.
            changed = min((r["applied"][:4] for r in c["settings_requested"] if r.get("applied")), default="9999")
            end = min(y0 + lc["horizon_years"] - 1, int(changed) - 1)
            vals = {p["month"]: p["value"] for p in rec["series"][str(c["company"])]}
            reached = [y for y in range(y0, end + 1) if f"{y}-12" in vals and f"{y}-12" in ref_v]
            e = {"record": rec["id"], "seed": rec["seed"], "company": c["company"], "at": None,
                 "settings": c["settings_start"], "genes": live_genes(t, settings_dict(c["settings_start"])),
                 "ratio": None, "evidence": False, "why": "", "vals": vals}
            if not reached or reached[-1] - y0 + 1 < lc["min_years"]:
                e["why"] = (f"one configuration for under {lc.get('min_years', 2)} game year(s) "
                            f"(first setting change {changed if changed != '9999' else 'none'})")
            else:
                e.update(evidence=True, at=f"{reached[-1]}-12")
            game.append(e)
        judged = [e for e in game if e["evidence"]]
        at = min((e["at"] for e in judged), default=None)
        for e in judged:
            e["at"], e["ratio"] = at, round(e["vals"][at] / max(1, ref_v[at]), 4)
        best = max((e["ratio"] for e in judged), default=0)
        for e in judged:
            e["paired"] = len(judged) > 1
            if e["paired"]:
                e["lost"], e["radius"] = e["ratio"] < lc["paired_margin"] * best, lc["veto_genes"]
                e["why"] = f"{e['ratio']:.3f} of {lc['reference']} at {at}, the game's best {best:.3f}"
            else:
                e["lost"], e["radius"] = e["ratio"] < lc["loss_ratio"], lc["loss_veto_genes"]
                e["why"] = f"{e['ratio']:.3f} of {lc['reference']} at {at} (a loss is under {lc['loss_ratio']})"
        for e in game:
            del e["vals"]
        out += game
    return out


def live_veto(t: dict, genes: dict, live: list[dict]) -> dict | None:
    """The live entry that lost with this configuration: genes equal to its genes except in at most
    the entry's radius (live_entries: `veto_genes` for a paired loss, `loss_veto_genes` for a loss to
    the reference AI). None when no lost entry matches."""
    for e in live:
        n = e.get("radius", live_cfg(t)["veto_genes"])
        if e.get("lost") and sum(genes.get(g) != e["genes"][g] for g in e["genes"]) <= n:
            return e
    return None


def live_seeds(t: dict, live: list[dict], tried: list[dict]) -> list[tuple[dict, str]]:
    """Up to `seed_offspring` configurations to play in the arena: the winners of paired live
    comparisons (several PrismaticAI configurations in one game) that did not lose, best first, whose
    genes no variant has had yet. Live play suggests them; the arena measures them."""
    out = []
    for e in sorted((e for e in live if e.get("paired") and not e["lost"]), key=lambda e: -e["ratio"]):
        if e["genes"] not in tried and len(out) < live_cfg(t).get("seed_offspring", 1):
            out.append((dict(e["genes"]), f"{e['record']}:{e['company']}"))
    return out


def live_records(results: Path | None = None) -> list[dict]:
    return [json.loads(f.read_text()) for f in tracked_records(LIVE_GLOB, results)]


def derive_live_settings(t: dict, battle_rec: dict | None, records: list[dict]) -> dict:
    """The next live start's settings, derived from the records alone. Only a configuration with
    live evidence is chosen: one that a live game judged and found not lost (live_entries), best
    ratio to the reference first, unless another game's loss vetoes it (live_veto). The arena's
    PrismaticAI configurations of the newest battle record are listed with their arena ratio and
    their veto, as context; none is chosen on arena evidence alone. Nothing judged and not lost:
    no settings (`chosen` null), so a start never falls back to info.nut's defaults unseen."""
    live = live_entries(records, t)
    ref_key = live_cfg(t)["reference"]
    arena = []
    if battle_rec:
        board = {r["key"]: r for r in battle_rec["leaderboard"]}
        ref = board.get(ref_key, {}).get("avg_value", 0)
        for c in battle_rec["contestants"]:
            row = board.get(c["key"], {})
            if c["ai"] != "PrismaticAI" or not row.get("played") or not ref:
                continue
            veto = live_veto(t, live_genes(t, settings_dict(c.get("settings", ""))), live)
            arena.append({"candidate": c["key"], "settings": c.get("settings", ""),
                          "arena_ratio": round(row["avg_value"] / ref, 4), "played": row["played"],
                          "vetoed_by": f"{veto['record']}:{veto['company']}" if veto else None})
    arena.sort(key=lambda c: (-c["arena_ratio"], c["candidate"]))
    cands = []
    for e in sorted((e for e in live if e["evidence"] and not e["lost"]),
                    key=lambda e: (-e["ratio"], e["record"], e["company"])):
        veto = live_veto(t, e["genes"], live)
        same = next((a for a in arena if live_genes(t, settings_dict(a["settings"])) == e["genes"]), None)
        cands.append({"candidate": f"{e['record']}:{e['company']}", "settings": settings_str(settings_dict(e["settings"])),
                      "live_ratio": e["ratio"], "at": e["at"], "why": e["why"],
                      "arena": same["candidate"] if same else None,
                      "vetoed_by": f"{veto['record']}:{veto['company']}" if veto else None})
    chosen = next((c for c in cands if not c["vetoed_by"]), None)
    return {"schema": LIVE_SETTINGS_SCHEMA,
            "derived_by": "tools/research/ai-battle/battle.py live-settings (battle.json tuning.live)",
            "rule": live_cfg(t), "battle_record": battle_rec["id"] if battle_rec else None,
            "live_records": [r["id"] for r in records],
            "chosen": chosen["candidate"] if chosen else None,
            "settings": chosen["settings"] if chosen else None,
            "reason": (f"the best configuration a live game judged and found not lost ({chosen['why']})"
                       if chosen else "no configuration has live evidence of not losing: no live start is derived"),
            "candidates": cands, "arena": arena,
            "live": [{k: e.get(k) for k in ("record", "company", "at", "ratio", "evidence", "paired", "lost", "radius",
                                            "why", "settings")} for e in live],
            "seeds": [s for _, s in live_seeds(t, live, [live_genes(t, settings_dict(c["settings"]))
                                                         for c in arena])]}


def live_settings_text(t: dict, results: Path | None = None) -> str:
    rec = latest_record()
    return json.dumps(derive_live_settings(t, rec, live_records(results)), indent=1) + "\n"


def live_tuning(cfg_raw: dict) -> dict:
    """battle.json's `tuning` with its genes resolved against info.nut, read-only (no state file)."""
    cfg = {"tuning": json.loads(json.dumps(cfg_raw["tuning"])), "prismatic": cfg_raw.get("prismatic", {})}
    resolve_genes(cfg)
    return cfg["tuning"]


def cmd_live_export(seed: int, panel: str | None, db: Path | None) -> int:
    if not (panel or db) or (panel and db):
        print("live-export: give exactly one of --panel <url> and --db <copy of the store>", file=sys.stderr)
        return 2
    query = panel_query(panel) if panel else duckdb_query(db)
    rec = live_record(query, seed, f"panel:{panel}" if panel else f"duckdb:{db.name}")
    for e in validate_live(rec, f"{rec['id']}.json"):
        print(e, file=sys.stderr)
    (RESULTS_DIR / f"{rec['id']}.json").write_text(json.dumps(rec, indent=1) + "\n")
    print(f"live-export: wrote research/results/{rec['id']}.json ({rec['first_month']}..{rec['final_month']}, "
          f"{len(rec['runs'])} run(s))")
    return 0


def cmd_live_settings(write: bool) -> int:
    t = live_tuning(json.loads(DEFAULT_CONFIG.read_text()))
    text = live_settings_text(t)
    if write:
        (RESULTS_DIR / LIVE_SETTINGS).write_text(text)
    d = json.loads(text)
    print(f"PRISMATIC_BOT_SETTINGS={d['settings'] or ''}  # {d['chosen']}: {d['reason']}")
    return 0 if d["chosen"] else 10


def contestant_key(c: dict) -> str:
    """The identity a contestant scores under: its label, else the AI name plus its settings."""
    if c.get("label"):
        return c["label"]
    return c["ai"] + (f"[{c['settings']}]" if c.get("settings") else "")


def git(*args: str, cwd: Path = REPO) -> str:
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True,
                          check=True).stdout.strip()


# --------------------------------------------------------------------------------------- hosts

def load_hosts(cfg: dict) -> list[dict]:
    """Hosts live in a local, untracked file: names, addresses and users stay off the public repo."""
    if cfg["hosts_file"].is_file():
        return json.loads(cfg["hosts_file"].read_text())["hosts"]
    return [{"name": "local", "ssh": None, "slots": max(1, (os.cpu_count() or 2) // 3)}]


# What an arena must have left before it takes a match (battle.json `resource_floor`, per host
# `resource_floor` in hosts.json). The disk floor is the machine rule's 35 GiB.
FLOOR_DEFAULTS = {"max_load_per_core": 1.0, "min_free_disk_gib": 35, "max_swap_used_gib": 16}
GIB_KIB = 1024 * 1024


def floor_limits(defaults: dict | None, spec: dict) -> dict:
    """The resource floor of one host: built-in defaults < battle.json < the host's own entry."""
    return {**FLOOR_DEFAULTS, **(defaults or {}), **(spec.get("resource_floor") or {})}


def probe_script(binary: str, root: str) -> str:
    """One shell call, POSIX sh on macOS and Linux, one tagged line per fact: the build, a pause
    marker, CPU count, load average, swap (macOS sysctl / Linux /proc/meminfo) and free disk of the
    remote root (its nearest existing parent before the first sync)."""
    return (f'b={binary}; r={root}\n'
            'if [ -x "$b" ]; then echo "VERSION $("$b" -h 2>&1 | head -1)"; else echo NO_BINARY; fi\n'
            'for f in "$r"/PAUSED*; do if [ -f "$f" ]; then echo "PAUSED $f"; '
            'echo "PAUSEDMSG $(head -1 "$f")"; break; fi; done\n'
            'echo "NCPU $(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || '
            'getconf _NPROCESSORS_ONLN 2>/dev/null)"\n'
            'echo "LOAD $(cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg 2>/dev/null || uptime)"\n'
            'echo "SWAP $(sysctl -n vm.swapusage 2>/dev/null)"\n'
            "grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null "
            "| sed 's/^/MEM /'\n"
            'd="$r"; while [ ! -d "$d" ]; do d=$(dirname "$d"); done\n'
            'echo "DISK $(df -Pk "$d" 2>/dev/null | tail -1)"\n'
            'exit 0\n')


UNIT_MIB = {"K": 1 / 1024, "M": 1, "G": 1024, "T": 1024 * 1024}
DF_LINE = re.compile(r"(\d+)\s+(\d+)\s+(\d+)\s+(\d+)%\s+(\S.*)$")


def parse_probe(text: str) -> dict:
    """The facts of `probe_script`'s output. Tolerant: a figure it cannot read stays None (and never
    blocks a host) and the line goes to `unparsed`, for the orchestrator to log."""
    res = {"version": None, "no_binary": False, "pause_file": None, "paused": None, "ncpu": None,
           "load": None, "disk_free_kib": None, "disk_mount": None, "swap_used_mib": None,
           "swap_total_mib": None, "mem_avail_kib": None, "unparsed": []}
    meminfo: dict[str, int] = {}
    for line in text.splitlines():
        tag, _, rest = line.partition(" ")
        rest = rest.strip()
        ok = True
        if tag == "VERSION":
            res["version"] = rest.replace("OpenTTD ", "")
        elif tag == "NO_BINARY":
            res["no_binary"] = True
        elif tag == "PAUSED":
            res["pause_file"] = rest
        elif tag == "PAUSEDMSG":
            res["paused"] = rest
        elif tag == "NCPU":
            ok = rest.isdigit() and int(rest) > 0
            res["ncpu"] = int(rest) if ok else None
        elif tag == "LOAD":
            # /proc/loadavg "0.52 0.58 0.59 1/389 12345"; sysctl "{ 1.2 1.1 1.0 }"; uptime "...
            # load average(s): 0.52, 0.58, 0.59" (some locales write 0,52).
            m = re.search(r"load averages?:\s*(.*)", rest)
            nums = re.findall(r"\d+(?:[.,]\d+)?", m.group(1) if m else rest)[:3]
            ok = bool(nums)
            res["load"] = [float(n.replace(",", ".")) for n in nums] if ok else None
        elif tag == "SWAP":
            if rest:   # macOS: "total = 2048.00M  used = 1024.50M  free = 1023.50M  (encrypted)"
                t = re.search(r"total = ([\d.]+)([KMGT])", rest)
                u = re.search(r"used = ([\d.]+)([KMGT])", rest)
                ok = bool(t and u)
                if ok:
                    res["swap_total_mib"] = float(t.group(1)) * UNIT_MIB[t.group(2)]
                    res["swap_used_mib"] = float(u.group(1)) * UNIT_MIB[u.group(2)]
        elif tag == "MEM":   # Linux: "MemAvailable:   1234 kB"
            m = re.match(r"(\w+):\s*(\d+)", rest)
            ok = bool(m)
            if ok:
                meminfo[m.group(1)] = int(m.group(2))
        elif tag == "DISK":
            m = DF_LINE.search(rest)
            ok = bool(m)
            if ok:
                res["disk_free_kib"], res["disk_mount"] = int(m.group(3)), m.group(5)
        elif line.strip():
            ok = False
        if not ok:
            res["unparsed"].append(line.strip()[:120])
    if "MemAvailable" in meminfo:
        res["mem_avail_kib"] = meminfo["MemAvailable"]
    if "SwapTotal" in meminfo and "SwapFree" in meminfo:
        res["swap_total_mib"] = meminfo["SwapTotal"] / 1024
        res["swap_used_mib"] = (meminfo["SwapTotal"] - meminfo["SwapFree"]) / 1024
    if res["pause_file"] and not res["paused"]:
        res["paused"] = f"{res['pause_file']} present"
    for key, tag in (("ncpu", "NCPU"), ("load", "LOAD"), ("disk_free_kib", "DISK")):
        if res[key] is None and not any(u.startswith(tag) for u in res["unparsed"]):
            res["unparsed"].append(f"no {tag} line")
    return res


def probe_state(res: dict, returncode: int, stderr: str, binary: str) -> tuple[str, str]:
    """(state, detail) of a host from its parsed probe. A pause marker wins over everything but an
    unreachable host: it is a human's order. Output without a build line is an error: a host whose
    probe is not understood is not handed matches."""
    if res["paused"]:
        return "paused", res["paused"]
    if res["no_binary"]:
        return "no-binary", f"{binary} not found"
    if returncode != 0:
        return "error", stderr.strip()[-200:]
    if res["version"] is None:
        return "error", "probe output not understood: " + "; ".join(res["unparsed"])[:200]
    return "online", resource_summary(res)


def resource_summary(res: dict) -> str:
    parts = []
    if res.get("load") and res.get("ncpu"):
        parts.append(f"load {max(res['load'][:2]):.1f}/{res['ncpu']} cores")
    if res.get("disk_free_kib") is not None:
        parts.append(f"disk {res['disk_free_kib'] / GIB_KIB:.1f} GiB free")
    if res.get("swap_used_mib") is not None:
        parts.append(f"swap {res['swap_used_mib'] / 1024:.1f} GiB used")
    if res.get("mem_avail_kib") is not None:
        parts.append(f"mem {res['mem_avail_kib'] / GIB_KIB:.1f} GiB available")
    return ", ".join(parts)


def floor_decision(load: float | None, ncpu: int | None, free_disk_kib: int | None, room: int,
                   limits: dict, swap_used_mib: float | None = None) -> tuple[int, str]:
    """How many new matches a host may take: (count, reason). `room` is its free slots. A figure
    that is None (unreadable) never blocks. Blocked by free disk under the floor, used swap over its
    ceiling, or load per core over its ceiling; otherwise capped so that one more match (one core,
    OpenTTD runs single-threaded) per new match keeps the load at or under the ceiling. The reason
    is empty when nothing but the slots limited the count."""
    room = max(0, room)
    floor_gib = limits.get("min_free_disk_gib")
    if free_disk_kib is not None and floor_gib is not None and free_disk_kib < floor_gib * GIB_KIB:
        return 0, f"free disk {free_disk_kib / GIB_KIB:.1f} GiB < floor {floor_gib} GiB"
    swap_cap = limits.get("max_swap_used_gib")
    if swap_used_mib is not None and swap_cap is not None and swap_used_mib > swap_cap * 1024:
        return 0, f"swap {swap_used_mib / 1024:.1f} GiB used > ceiling {swap_cap} GiB"
    ceiling = limits.get("max_load_per_core")
    if load is None or not ncpu or ceiling is None:
        return room, ""
    per_core = load / ncpu
    if per_core > ceiling:
        return 0, f"load {load:.1f} on {ncpu} cores = {per_core:.2f}/core > ceiling {ceiling}"
    headroom = max(0, math.floor(ceiling * ncpu - load + 1e-9))
    if headroom < room:
        return headroom, (f"load {load:.1f} on {ncpu} cores: room for {headroom} more under "
                          f"{ceiling}/core")
    return room, ""


def stop_requested(root: Path) -> str | None:
    """The orchestrator's STOP file: None when absent, else its first line (or a note that it exists)."""
    f = root / "STOP"
    if not f.is_file():
        return None
    try:
        first = (f.read_text(errors="replace").splitlines() or [""])[0].strip()
    except OSError:
        first = ""
    return first or f"{f} present"


def owner_marker(hostname: str, pid: int) -> str:
    """`<host>:<pid>` of the orchestrator, safe to put unquoted into a shell line and a process title."""
    return f"{re.sub(r'[^A-Za-z0-9._-]', '_', hostname) or 'unknown'}:{pid}"


def orchestrator_text(hostname: str, pid: int, started_at: str, commit: str, arena_rev: str,
                      edition: str, root: str, local_root: str) -> str:
    """The `<remote root>/ORCHESTRATOR` file: who runs the matches on this host and how to stop it."""
    marker = owner_marker(hostname, pid)
    return (f"# AIBattle (OpenTTD research fork, tools/research/ai-battle/battle.py run) schedules\n"
            f"# matches onto this host. Matches carry TTD_BATTLE_ORCHESTRATOR={marker}.\n"
            f"host={hostname}\npid={pid}\nstarted_at={started_at}\ncommit={commit}\n"
            f"arena_rev={arena_rev}\nedition={edition}\nmarker=TTD_BATTLE_ORCHESTRATOR={marker}\n"
            f"how_to_stop=echo '<reason>' > {root}/PAUSED on this host (no new match here; running "
            f"ones finish, or kill them by {root}/matches/*/*/pid), or SIGTERM pid {pid} on host "
            f"{hostname} (kill -TERM {pid}), or create {local_root}/STOP there (no new match "
            f"anywhere)\n")


class Host:
    """One arena machine; `ssh` is None for the machine this orchestrator runs on."""

    def __init__(self, spec: dict, remote_root: str, floor: dict | None = None):
        self.name = spec["name"]
        self.ssh = spec.get("ssh")
        self.ssh_args = [os.path.expanduser(a) for a in spec.get("ssh_args", [])]
        self.binary = spec.get("binary", DEFAULT_BINARY)
        self.slots = int(spec.get("slots", 1))
        self.limits = floor_limits(floor, spec)
        self.root = remote_root
        self.state = "unknown"   # online | busy | paused | offline | no-binary | error
        self.detail = ""
        self.version = ""
        self.res: dict = {}
        self.synced_digest = None
        self.claimed = False     # <root>/ORCHESTRATOR written by this process
        self.next_probe = 0.0

    def sh(self, script: str, timeout: int = 60) -> subprocess.CompletedProcess:
        cmd = (["/bin/sh", "-c", script] if self.ssh is None
               else ["ssh", *SSH_OPTS, *self.ssh_args, self.ssh, script])
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)

    def try_sh(self, script: str, timeout: int = 60) -> subprocess.CompletedProcess:
        """Best-effort `sh`: a host that stops answering yields exit 124 instead of an exception,
        so one unreachable arena can never take the orchestrator down."""
        try:
            return self.sh(script, timeout)
        except subprocess.TimeoutExpired:
            return subprocess.CompletedProcess(script, 124, "", f"timed out after {timeout}s")

    def probe(self) -> None:
        try:
            r = self.sh(probe_script(self.binary, self.root), timeout=15)
        except subprocess.TimeoutExpired:
            self.state, self.detail = "offline", "timeout"
            return
        if r.returncode == 255:
            self.state, self.detail = "offline", (r.stderr.strip().splitlines() or [""])[-1]
            return
        self.res = parse_probe(r.stdout)
        self.state, self.detail = probe_state(self.res, r.returncode, r.stderr, self.binary)
        if self.res["version"] is not None:
            self.version = self.res["version"]

    def admit(self, running: int) -> int:
        """New matches this host may take now, from its last probe: its free slots, held to the
        resource floor. A host the floor blocks becomes `busy`, with the figures as its detail."""
        if self.state != "online":
            return 0
        load = self.res.get("load")
        n, why = floor_decision(max(load[:2]) if load else None, self.res.get("ncpu"),
                                self.res.get("disk_free_kib"), self.slots - running, self.limits,
                                self.res.get("swap_used_mib"))
        if why:
            self.state, self.detail = ("busy" if n == 0 else "online"), why
        return n

    def claim(self, text: str) -> bool:
        """Write <root>/ORCHESTRATOR (who schedules here, how to stop it). True when written."""
        r = self.try_sh(f"mkdir -p {self.root} && cat > {self.root}/ORCHESTRATOR <<'EOS'\n{text}EOS\n",
                        timeout=30)
        self.claimed = r.returncode == 0
        return self.claimed

    def sync_arena(self, cache: Path, digest: str) -> None:
        """Copy the arena bundle to <root>/arena on the host (skipped when it is the cache)."""
        if self.synced_digest == digest:
            return
        if self.ssh is None:
            dest = expand(self.root) / "arena"
            if dest.resolve() != (cache / "arena").resolve():
                dest.mkdir(parents=True, exist_ok=True)
                subprocess.run(["rsync", "-a", "--delete", f"{cache}/arena/", f"{dest}/"],
                               check=True, capture_output=True, timeout=300)
        else:
            self.sh(f"mkdir -p {self.root}")
            rsh = " ".join(["ssh", *SSH_OPTS, *self.ssh_args])
            r = subprocess.run(["rsync", "-a", "--delete", "-e", rsh,
                                f"{cache}/arena/", f"{self.ssh}:{self.root}/arena/"],
                               capture_output=True, text=True, timeout=300)
            if r.returncode != 0:
                raise RuntimeError(f"rsync to {self.name} failed: {r.stderr.strip()}")
        self.synced_digest = digest


def local_ipv4() -> str:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("192.0.2.1", 9))   # TEST-NET-1: no packet is sent for a UDP connect
        return s.getsockname()[0]
    finally:
        s.close()


def ssh_aliases() -> dict[str, str]:
    """HostName -> alias from ~/.ssh/config (the first alias wins)."""
    out, alias = {}, None
    cfg = Path.home() / ".ssh" / "config"
    if not cfg.is_file():
        return out
    for line in cfg.read_text().splitlines():
        parts = line.strip().split()
        if len(parts) >= 2 and parts[0].lower() == "host" and "*" not in parts[1]:
            alias = parts[1]
        elif len(parts) >= 2 and parts[0].lower() == "hostname" and alias:
            out.setdefault(parts[1], alias)
    return out


def cmd_discover(cfg: dict) -> None:
    d = cfg.get("discover", {})
    me = local_ipv4()
    prefix = me.rsplit(".", 1)[0]
    port = int(d.get("port", 22))

    def open22(i: int):
        ip = f"{prefix}.{i}"
        try:
            with socket.create_connection((ip, port), timeout=0.6):
                return ip
        except OSError:
            return None

    with concurrent.futures.ThreadPoolExecutor(64) as pool:
        candidates = [ip for ip in pool.map(open22, range(1, 255)) if ip and ip != me]
    aliases = ssh_aliases()
    ident = d.get("identity")
    # Names and slot counts the owner chose for a known host survive a re-discovery.
    known = {h.get("ssh"): h for h in (json.loads(cfg["hosts_file"].read_text())["hosts"]
                                        if cfg["hosts_file"].is_file() else [])}
    found = [known.get(None) or {"name": d.get("local_name", "local"), "ssh": None,
                                 "slots": max(1, (os.cpu_count() or 2) // 3)}]
    probe = ('hostname -s; uname -m; (sysctl -n hw.ncpu 2>/dev/null || nproc); '
             f'b={DEFAULT_BINARY}; [ -x "$b" ] && "$b" -h 2>&1 | head -1 || echo NO_BINARY')
    for ip in candidates:
        target = aliases.get(ip) or f"{d.get('user', os.environ.get('USER', ''))}@{ip}"
        args = [] if ip in aliases or not ident else ["-i", ident, "-o", "IdentitiesOnly=yes"]
        r = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
                            "-o", "StrictHostKeyChecking=accept-new", *[os.path.expanduser(a)
                                                                        for a in args],
                            target, probe], capture_output=True, text=True, timeout=30)
        lines = r.stdout.strip().splitlines()
        if r.returncode != 0 or len(lines) < 4:
            log(f"{ip:16} ssh refused ({(r.stderr.strip().splitlines() or ['?'])[-1][:60]})")
            continue
        name, arch, cpus, version = lines[0].lower(), lines[1], int(lines[2]), lines[3]
        log(f"{ip:16} {name:24} {arch:7} {cpus:3} cpu  {version}")
        if version == "NO_BINARY":
            continue
        prev = known.get(target, {})
        found.append({"name": prev.get("name", aliases.get(ip, name)), "ssh": target,
                      "ssh_args": args, "slots": prev.get("slots", max(1, cpus // 3)),
                      "arch": arch})
    cfg["hosts_file"].parent.mkdir(parents=True, exist_ok=True)
    cfg["hosts_file"].write_text(json.dumps({"hosts": found}, indent=2) + "\n")
    log(f"{len(found)} arena(s) written to {cfg['hosts_file']}")


# --------------------------------------------------------------------------------- arena bundle

def arena_digest(cache: Path) -> str:
    h = hashlib.sha256()
    for p in sorted((cache / "arena").rglob("*")):
        if p.is_file() and p.name != "PROVENANCE.json":
            h.update(str(p.relative_to(cache)).encode())
            h.update(p.read_bytes())
    return h.hexdigest()[:16]


def edition_of(cfg: dict) -> str:
    """The edition: the arena bundle plus everything else that decides a match (contestants with
    their settings, the match parameters). Changing any of them starts a new edition."""
    rules = json.dumps({"contestants": cfg["contestants"], "match": cfg["match"]}, sort_keys=True)
    return hashlib.sha256((arena_digest(cfg["cache"]) + rules).encode()).hexdigest()[:16]


REFEREE_TAR = "BattleReferee.tar"


def export_tree(path: str, rev: str, dest_tar: Path) -> None:
    """`git archive` a repository directory with its parents stripped: <basename>/... in the tar."""
    raw = subprocess.run(["git", "-C", str(REPO), "archive", "--format=tar", rev, path],
                         capture_output=True, check=True).stdout
    prefix = path.rstrip("/").rsplit("/", 1)[0] + "/" if "/" in path else ""
    with tarfile.open(fileobj=io.BytesIO(raw)) as src, tarfile.open(dest_tar, "w") as dst:
        for m in src.getmembers():
            if not m.name.startswith(prefix) or len(m.name) <= len(prefix):
                continue
            data = src.extractfile(m) if m.isfile() else None
            m.name = m.name[len(prefix):]
            dst.addfile(m, data)


def cmd_prepare(cfg: dict, allow_dirty: bool) -> None:
    """Assemble <cache>/arena/{baseset,ai,game} plus PROVENANCE.json."""
    arena = cfg["cache"] / "arena"
    subprocess.run(["rm", "-rf", str(arena)], check=True)
    for sub in ("baseset", "ai/library", "game"):
        (arena / sub).mkdir(parents=True, exist_ok=True)
    if not cfg["baseset"].is_file():
        sys.exit(f"base graphics set {cfg['baseset']} not found "
                 "(research/baseset-prerequisite-macos.md)")
    subprocess.run(["cp", str(cfg["baseset"]), str(arena / "baseset")], check=True)

    downloaded = cfg["cache"] / "content" / "content_download" / "ai"
    for tar in sorted(downloaded.rglob("*.tar")) if downloaded.is_dir() else []:
        dest = arena / "ai" / tar.relative_to(downloaded)
        dest.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cp", str(tar), str(dest)], check=True)

    # PrismaticAI and the referee come from this repository at a commit, never from a work tree.
    p, ref = cfg["prismatic"], cfg["referee"]
    dirty = git("status", "--porcelain", "--", p["path"], ref["path"])
    if dirty and not allow_dirty:
        sys.exit(f"refusing: uncommitted changes under the exported paths:\n{dirty}\n"
                 "commit them, or pass --allow-dirty for a run whose records `check` refuses")
    rev = git("rev-parse", p.get("rev", "HEAD"))
    # The referee follows the repository (battle.json referee.rev, default HEAD), not PrismaticAI's
    # pin: a bot pinned to an old commit is still refereed by the current fields (loan=, REF S/T).
    ref_rev = git("rev-parse", ref.get("rev", "HEAD"))
    export_tree(p["path"], rev, arena / "ai" / "PrismaticAI.tar")
    if dirty:  # --allow-dirty: the referee as it is on disk, and the edition marked dirty
        subprocess.run(["cp", "-R", str(REPO / ref["path"]), str(arena / "game")], check=True)
    else:
        export_tree(ref["path"], ref_rev, arena / "game" / REFEREE_TAR)

    (arena / "PROVENANCE.json").write_text(json.dumps({
        "prismatic_path": p["path"], "referee_path": ref["path"], "rev": rev,
        "referee_rev": ref_rev, "dirty": bool(dirty), "baseset": cfg["baseset"].name,
        "ais": sorted(str(t.relative_to(arena / "ai")) for t in (arena / "ai").rglob("*.tar")),
        "prepared_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
    }, indent=2) + "\n")
    log(f"arena prepared: rev {rev[:10]}{' (DIRTY)' if dirty else ''}, "
        f"edition {edition_of(cfg)}")


def tar_files(path: Path) -> dict[str, bytes]:
    """The regular files of a tar by name: two tars with the same files hold the same script."""
    with tarfile.open(path) as t:
        return {m.name: t.extractfile(m).read() for m in t.getmembers() if m.isfile()}


def refresh_referee(cfg: dict) -> str | None:
    """Bring a prepared arena's referee to the repository's (battle.json referee.rev, default HEAD)
    and leave the rest of the bundle as it is: PrismaticAI, the AIs and `rev` keep their commit.
    The new tar is written beside the old one and renamed over it, so a match that is running
    keeps the file it opened and only matches that start afterwards read the new one. Returns what
    changed, or None when the arena already had that referee (same files, so the edition stays).
    Raises RuntimeError when there is no prepared arena or the referee has uncommitted changes."""
    arena = cfg["cache"] / "arena"
    prov_file, ref = arena / "PROVENANCE.json", cfg["referee"]
    if not prov_file.is_file():
        raise RuntimeError(f"no prepared arena at {arena} (battle.py prepare)")
    prov = json.loads(prov_file.read_text())
    if prov.get("dirty") or git("status", "--porcelain", "--", ref["path"]):
        raise RuntimeError(f"the referee at {ref['path']} or the arena is uncommitted (dirty); "
                           "commit it and run prepare")
    rev = git("rev-parse", ref.get("rev", "HEAD"))
    dest = arena / "game" / REFEREE_TAR
    tmp = dest.with_name(REFEREE_TAR + ".new")   # no .tar suffix: OpenTTD never scans it
    dest.parent.mkdir(parents=True, exist_ok=True)
    export_tree(ref["path"], rev, tmp)
    if dest.is_file() and tar_files(dest) == tar_files(tmp):
        tmp.unlink()
        return None
    os.replace(tmp, dest)
    old = prov.get("referee_rev", prov.get("rev", ""))
    prov.update(referee_rev=rev, referee_prepared_at=dt.datetime.now(dt.timezone.utc)
                .isoformat(timespec="seconds"))
    prov_file.write_text(json.dumps(prov, indent=2) + "\n")
    return f"arena referee {old[:10] or '?'} -> {rev[:10]}, edition {edition_of(cfg)}"


def follow_referee(cfg: dict) -> None:
    """`run` starts on the repository's referee (battle.json referee.follow, default true): an
    arena prepared before a referee change would otherwise keep logging the old fields for good."""
    if not cfg["referee"].get("follow", True):
        return
    try:
        changed = refresh_referee(cfg)
    except (RuntimeError, subprocess.CalledProcessError) as exc:
        log(f"referee not followed: {exc}")
        return
    log(changed or "arena referee is the repository's")


def cmd_fetch(cfg: dict) -> None:
    """Download the BaNaNaS contestants with a throwaway local dedicated server bound to loopback."""
    home = cfg["cache"] / "content"
    (home / "Documents" / "OpenTTD" / "baseset").mkdir(parents=True, exist_ok=True)
    # Even a dedicated server refuses to start without a base graphics set.
    subprocess.run(["cp", str(cfg["baseset"]), str(home / "Documents" / "OpenTTD" / "baseset")],
                   check=True)
    local = next((h for h in load_hosts(cfg) if h.get("ssh") is None), {})
    binary = str(expand(local.get("binary", DEFAULT_BINARY)))
    # With -c the config dir is the working directory, so content lands in <home>/content_download.
    (home / "fetch.cfg").write_text("[network]\nserver_advertise = false\n")
    proc = subprocess.Popen([binary, "-D", "127.0.0.1:39790", "-c", "fetch.cfg", "-x"],
                            cwd=home, env=dict(os.environ, HOME=str(home)),
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, bufsize=1)
    lines: list[str] = []
    threading.Thread(target=lambda: lines.extend(proc.stdout), daemon=True).start()

    def send(cmd: str, wait: float) -> None:
        proc.stdin.write(cmd + "\n")
        proc.stdin.flush()
        time.sleep(wait)

    time.sleep(5)
    send("content update", 10)
    wanted = fetch_wanted(cfg)
    ids = {}
    for name in dict.fromkeys(wanted):
        mark = len(lines)
        send(f"content state {name}", 2)
        for line in lines[mark:]:
            parts = [x.strip() for x in line.split(",")]
            if len(parts) >= 4 and parts[1] == "AI" and parts[3] == name:
                ids[name] = parts[0]
    for cid in ids.values():
        send(f"content select {cid}", 1)
    send("content download", 30)
    send("quit", 3)
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        proc.kill()
    (home / "fetch.log").write_text("".join(lines))
    missing = sorted(set(wanted) - set(ids))
    done = sum(1 for line in lines if line.startswith("Completed download"))
    log(f"fetched {done} package(s) for {sorted(ids)}; missing: {missing or 'none'}")
    for problem in rival_versions(home / "content_download" / "ai", live_like(cfg).get("rivals", [])):
        log(f"live-like: {problem}")
    if missing:
        sys.exit(12)


# ------------------------------------------------------------------------ live-like arena, replay
#
# The arena's own world (1960, four rivals, the bot unplanned) is not the owner's live game (1950,
# seven BaNaNaS rivals, the bot sometimes planned): in live game 15805 PrismaticAI ran one train
# for eight years, and the arena produced a one-train PrismaticAI in 0.2% of its matches
# (research/incident-2026-11-23-live-game-15805.md). battle.json `live_like` declares the live
# world once; `ablate --live-like` plays it, and `replay` loads a live game's own save headless.

def live_like(cfg: dict) -> dict:
    """battle.json `live_like` (empty when the configuration has none)."""
    return cfg.get("live_like") or {}


def fetch_wanted(cfg: dict) -> list[str]:
    """The BaNaNaS AIs `fetch` downloads: the arena's contestants and the live-like rivals."""
    return list(dict.fromkeys([c["ai"] for c in cfg["contestants"] if c.get("source") == "bananas"]
                              + [r["ai"] for r in live_like(cfg).get("rivals", [])]))


def tar_versions(ai_dir: Path, ai: str) -> list[str]:
    """The versions of `ai` among the content tars of a directory, which the content client names
    <8 hex digits>-<name with _ for spaces>-<version>.tar (484f4745-AAAHogEx-115.tar)."""
    pat = re.compile(rf"[0-9a-fA-F]{{8}}-{re.escape(ai.replace(' ', '_'))}-(.+)\.tar")
    found = (pat.fullmatch(t.name) for t in ai_dir.rglob("*.tar")) if ai_dir.is_dir() else ()
    return sorted(m.group(1) for m in found if m)


def rival_versions(ai_dir: Path, rivals: list[dict]) -> list[str]:
    """What the AI tars of `ai_dir` lack against the live rivals: an AI with no tar, or none of the
    version the live game ran. A replayed save asks for the AI it saved; a missing one is replaced
    by a random AI or left empty, and another version plays another game."""
    out = []
    for r in rivals:
        have = tar_versions(ai_dir, r["ai"])
        if not have:
            out.append(f"{r['ai']}: no tar in {ai_dir} (battle.py fetch && battle.py prepare)")
        elif r.get("version") and r["version"] not in have:
            out.append(f"{r['ai']}: version {', '.join(have)} in {ai_dir}, the live game ran "
                       f"{r['version']}")
    return out


def live_like_ablate(cfg: dict, seed: int | None, variants: list[str]) -> tuple[dict, int, list[str], int, list[str]]:
    """`ablate --live-like`: (config with the live match settings, seed, field, PrismaticAI's slot,
    variants). The seed defaults to the live game's, the variants to the configured arms; a
    planned arm names towns and industries by id, so it holds only on the seed it was planned on."""
    live = live_like(cfg)
    if not live:
        raise SystemExit("ablate --live-like: battle.json has no live_like block")
    out = dict(cfg, match=dict(cfg["match"], **live["match"]))
    arms = [f"{name}={settings}" for name, settings in live.get("arms", {}).items()]
    return (out, live["seed"] if seed is None else seed, [r["ai"] for r in live["rivals"]],
            live.get("slot", 0), variants or arms)


CONSOLE_DATE = re.compile(r"Date: (\d{4})-(\d{2})-(\d{2})")
CONSOLE_COMPANY = re.compile(r"#:(\d+)\([^)]*\) Company Name: '(.*)'  Year Founded: -?\d+  "
                             r"Money: (-?\d+)  Loan: (-?\d+)  Value: (-?\d+)  "
                             r"\(T:(\d+), R:(\d+), P:(\d+), S:(\d+)\)\s*(AI)?\s*$")


def replay_scripts(console_log: Path) -> tuple[str, str]:
    """(game_start.scr, monthly.scr) of a replay. A loaded save keeps its own Game Script (the live
    game runs none), so the referee cannot report: the console does. game_start.scr opens a
    console log (`script`, written without a GUI too), prints the date and `companies`, and
    schedules monthly.scr, which prints them again and schedules itself for the next month."""
    report = "getdate\ncompanies\nschedule on-next-calendar-month scripts/monthly.scr\n"
    return f'script "{console_log}"\n' + report, report


def parse_console(text: str) -> dict[int, list[dict]]:
    """{company id: [one sample per `companies` report]} from a replay's console log. `companies`
    numbers companies from 1; the id here is the game's (from 0). Only AI companies count: a save
    loaded without a network game adds a company for the local player."""
    series: dict[int, list[dict]] = {}
    t = ""
    for line in text.splitlines():
        d = CONSOLE_DATE.search(line)
        if d:
            t = f"{d.group(1)}-{d.group(2)}-{d.group(3)}"
            continue
        m = CONSOLE_COMPANY.search(line)
        if not m or not m.group(10):
            continue
        rail, road, air, water = (int(m.group(i)) for i in (6, 7, 8, 9))
        series.setdefault(int(m.group(1)) - 1, []).append({
            "t": t, "name": m.group(2), "money": int(m.group(3)), "loan": int(m.group(4)),
            "value": int(m.group(5)), "rail": rail, "road": road, "air": air, "water": water,
            "vehicles": rail + road + air + water})
    return series


AI_GONE = re.compile(r"The savegame has an AI by the name '(.*)', version (\d+) which is no longer available")


def replay_substitutions(text: str) -> list[str]:
    """Every AI the save names that the replay could not load (the game then starts a random AI in
    its place or none): such a replay plays another game, so each one fails it."""
    return [f"invalid replay: the save's AI {m.group(1)} version {m.group(2)} is not available"
            for m in AI_GONE.finditer(text)]


def replay_copy(src: Path, dst: Path) -> bool:
    """Copy a save for a replay, an lzma one (tag OTTX, OpenTTD's default) unpacked to an
    uncompressed one (tag OTTN, the same 4-byte savegame version, then the chunks): a binary built
    without liblzma (lundra's) loads only that, and the chunks are byte for byte the same. True
    when it unpacked."""
    data = src.read_bytes()
    if data[:4] != b"OTTX":
        dst.write_bytes(data)
        return False
    import lzma
    dst.write_bytes(b"OTTN" + data[4:8] + lzma.decompress(data[8:]))
    return True


def replay_verdict(series: dict[int, list[dict]], company: int, years: int, rules: dict) -> list[str]:
    """The replay's failures for the bot's company (empty: PASS). `min_trains`: trains at the end;
    `one_train_years`: fails when it ran at most one train in every monthly report of the last N
    years; `min_leader_ratio`: its value over the best other company's at the end. A replay that
    did not reach `years` of monthly reports fails too: it says nothing about those years."""
    mine = series.get(company, [])
    if len(mine) < 12 * years:
        return [f"company {company}: {len(mine)} monthly report(s), {12 * years} needed: the replay "
                "did not run its years"]
    fails, end = [], mine[-1]
    if end["rail"] < rules.get("min_trains", 2):
        fails.append(f"company {company}: {end['rail']} train(s) at {end['t']}, at least "
                     f"{rules.get('min_trains', 2)} wanted")
    window = mine[-12 * rules.get("one_train_years", 3):]
    if max(s["rail"] for s in window) <= 1:
        fails.append(f"company {company}: at most one train from {window[0]['t']} to {end['t']}")
    rivals = [s[-1] for c, s in series.items() if c != company and s and s[-1]["t"] == end["t"]]
    leader = max(rivals, key=lambda s: s["value"], default=None)
    if leader and leader["value"] > 0:
        ratio = end["value"] / leader["value"]
        if ratio < rules.get("min_leader_ratio", 0.0):
            fails.append(f"company {company}: value {end['value']:,} is {ratio:.3f} of the leader "
                         f"{leader['name']} ({leader['value']:,}), at least "
                         f"{rules['min_leader_ratio']} wanted")
    return fails


def replay_table(series: dict[int, list[dict]], company: int) -> list[str]:
    """One line per game year of the replay (its last report): the bot's trains, road vehicles,
    aircraft and value against the leader's."""
    out = []
    mine = series.get(company, [])
    last = {s["t"][:4]: i for i, s in enumerate(mine)}
    for year, i in last.items():
        s = mine[i]
        others = [o[i] for c, o in series.items() if c != company and len(o) > i]
        lead = max(others, key=lambda o: o["value"], default={"value": 0, "name": "-", "rail": 0})
        ratio = s["value"] / lead["value"] if lead["value"] > 0 else 0.0
        out.append(f"replay: {s['t']}  trains {s['rail']:>3} road {s['road']:>3} air {s['air']:>3}  "
                   f"value {s['value']:>11,}  leader {lead['name']} {lead['value']:,} "
                   f"({lead['rail']} trains)  ratio {ratio:.3f}")
    return out


def cmd_replay(cfg: dict, save: str | None, years: int | None, rev: str | None,
               binary: str | None, log_dir: str | None) -> int:
    """Load a copy of a live game's save headless (scratch HOME, null drivers) with this tree's
    PrismaticAI (or the one at `rev`) and the arena's rival tars, run it `years` game years and
    judge the bot's company by battle.json `replay` rules. The live save is never written."""
    rep = cfg.get("replay") or {}
    save_path = expand(save or rep.get("save", ""))
    years = years or rep.get("years", 4)
    exe = Path(binary) if binary else REPO / "build" / "openttd"
    arena = cfg["cache"] / "arena"
    if not save_path.is_file() or not exe.is_file() or not (arena / "baseset").is_dir():
        print(f"replay: NOT RUN: needs the save {save_path}, the binary {exe} and an arena cache "
              f"with a base set at {arena} (battle.py fetch && battle.py prepare)")
        return 12
    for problem in rival_versions(arena / "ai", live_like(cfg).get("rivals", [])):
        print(f"replay: warning: {problem}")
    import tempfile
    with tempfile.TemporaryDirectory(prefix="battle-replay-") as tmp:
        home = Path(tmp) / "home"
        personal = home / "Documents" / "OpenTTD"
        for sub in ("scripts", "ai", "game"):
            (personal / sub).mkdir(parents=True)
        os.symlink(arena / "baseset", personal / "baseset")
        (home / ".local" / "share").mkdir(parents=True)
        os.symlink(personal, home / ".local" / "share" / "openttd")   # Linux (XDG)
        ref_tars = [str(t) for t in (arena / "ai").glob("*.tar") if t.name != "PrismaticAI.tar"]
        subprocess.run(["cp", "-R", str(arena / "ai" / "library"), *ref_tars, str(personal / "ai")],
                       check=True)
        if rev:
            export_tree(cfg["prismatic"]["path"], rev, personal / "ai" / "PrismaticAI.tar")
        else:
            subprocess.run(["cp", "-R", str(REPO / cfg["prismatic"]["path"]), str(personal / "ai")],
                           check=True)
        console_log = Path(tmp) / "console.log"
        start, monthly = replay_scripts(console_log)
        (personal / "scripts" / "game_start.scr").write_text(start)
        (personal / "scripts" / "monthly.scr").write_text(monthly)
        (Path(tmp) / "replay.cfg").write_text("[gui]\nautosave = off\n")
        copy = Path(tmp) / save_path.name
        if replay_copy(save_path, copy):
            print(f"replay: {save_path.name}: lzma save unpacked for a binary built without liblzma")
        t0 = time.time()
        proc = subprocess.run(
            [str(exe), "-c", "replay.cfg", "-x", "-v", f"null:ticks={match_ticks(years)}", "-s", "null",
             "-m", "null", "-g", str(copy), "-d", "script=1"], cwd=tmp, env=dict(os.environ, HOME=str(home)),
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=rep.get("timeout_s", 3600))
        text = console_log.read_text() if console_log.is_file() else ""
        if log_dir:
            Path(log_dir).mkdir(parents=True, exist_ok=True)
            (Path(log_dir) / "console.log").write_text(text)
            (Path(log_dir) / "game.log").write_text(proc.stdout)
    series = parse_console(text)
    company = rep.get("company", 0)
    print(f"replay: {save_path.name}, PrismaticAI {'at ' + rev if rev else 'of the working tree'}, "
          f"company {company}, {years} year(s), exit {proc.returncode}, {time.time() - t0:.0f} s")
    for line in replay_table(series, company):
        print(line)
    fails = replay_substitutions(proc.stdout) + replay_verdict(series, company, years, rep.get("rules", {}))
    if proc.returncode != 0:
        fails.insert(0, f"the game exited with {proc.returncode}")
    for f in fails:
        print(f"replay: FAIL {f}", file=sys.stderr)
    print(f"replay: {'FAIL' if fails else 'PASS'}")
    return 10 if fails else 0

# -------------------------------------------------------------------------------------- matches

def lineup_for(contestants: list[dict], n: int, block: int = 1) -> list[dict]:
    """Match n's lineup. Contestants sharing a `group` (the PrismaticAI configurations) take turns,
    one per match: two copies of one bot on one map share its best airport towns and industries
    and starve each other. Everyone else plays every match. The start order rotates once per
    `block` matches, the matches that share one seed. So every member of a group starts from the
    same company slot on a seed, and a paired comparison does not measure the slot. On seed
    371319927 the same genes scored 11.8M from slot 0 and 2.9M from slot 2."""
    groups: dict[str, list[dict]] = {}
    for c in contestants:
        if c.get("group"):
            groups.setdefault(c["group"], []).append(c)
    picked = {g: members[n % len(members)]["key"] for g, members in groups.items()}
    chosen = [c for c in contestants if not c.get("group") or picked[c["group"]] == c["key"]]
    rot = (n // max(1, block)) % len(chosen)
    return chosen[rot:] + chosen[:rot]


def match_ticks(years: int) -> int:
    """Start on Jan 1 and run past Jan 3 of the final year, so its Q4 report is printed."""
    return int((years * 365.25 + 6) * 74)


def battle_cfg_text(mc: dict) -> str:
    return ("[difficulty]\nmax_no_competitors = 0\nmax_loan = 300000\ndisasters = false\n"
            f"vehicle_breakdowns = {mc.get('breakdowns', 1)}\n"
            f"[game_creation]\nlandscape = {mc['landscape']}\nstarting_year = {mc['start_year']}\n"
            f"map_x = {mc['map_x']}\nmap_y = {mc['map_y']}\n"
            "[gui]\nautosave = off\n[graphicsset]\nname = OpenGFX\n"
            "[game_scripts]\nBattleReferee =\n"
            # The live stack's script budget (mcp-rhai panel.rs SCRIPT_OPCODES; engine default
            # 10000): a live-like match gives every AI the opcodes per tick it has live.
            + (f"[script]\nscript_max_opcode_till_suspend = {mc['script_opcodes']}\n"
               if mc.get("script_opcodes") else ""))


def start_script_text(lineup: list[dict]) -> str:
    # The console tokenizer understands double quotes only; AI names may contain spaces.
    return "\n".join(f'start_ai "{c["ai"]}"' + (f" {c['settings']}" if c.get("settings") else "")
                     for c in lineup) + "\n"


def parse_start_script(text: str) -> tuple[list[str], int, str]:
    """Read a match's game_start.scr back into (opponents in start order, PrismaticAI's slot, its
    settings), so an ablate replays the arena's exact lineup: who starts first gets the first sites,
    and a replay against another field is a different game."""
    field, slot, settings = [], 0, ""
    for line in text.splitlines():
        m = re.match(r'\s*start_ai\s+"([^"]+)"\s*(\S*)', line)
        if not m:
            continue
        if m.group(1) == "PrismaticAI":
            slot, settings = len(field), m.group(2)
        else:
            field.append(m.group(1))
    return field, slot, settings


def match_replay_inputs(results: Path, ref: str) -> dict:
    """An arena match's replay inputs from its record (`results/<edition>/matches/<id>.json`):
    `ref` is "<edition prefix>/<m-NNNN>". Returns seed, years, field, slot, settings, the
    contestant's key and the host's build, so `ablate --match` replays the match as it was played."""
    edition, _, mid = ref.partition("/")
    found = sorted(results.glob(f"{edition}*/matches/{mid}.json"))
    if len(found) != 1:
        raise SystemExit(f"ablate: --match {ref}: {len(found)} records found under {results}")
    m = json.loads(found[0].read_text())
    field, slot, settings, key = [], 0, "", ""
    for c in m["lineup"]:
        if c["ai"] == "PrismaticAI":
            slot, settings, key = len(field), c.get("settings", ""), c.get("key", "")
        else:
            field.append(c["ai"])
    return {"seed": m["seed"], "years": m.get("years", 20), "field": field, "slot": slot,
            "settings": settings, "key": key, "host": m.get("host"),
            "host_version": m.get("host_version")}


def match_dir(root: str, m: dict) -> str:
    """Match ids restart in every edition, so the directory is per edition."""
    return f"{root}/matches/{m['edition']}/{m['id']}"


def reap_script(root: str, keep_edition: str) -> str:
    """Stop, through its pid file, every still-running match of another edition on a host: after a
    restart into a new edition (a new bot, config or GA generation) those would only hold slots."""
    return (f'n=0; for d in {root}/matches/*/m-*; do case "$d" in */{keep_edition}/*) continue;; esac; '
            '[ -f "$d/pid" ] || continue; [ -f "$d/exit_code" ] && continue; '
            'kill -0 "$(cat "$d/pid")" 2>/dev/null || continue; kill "$(cat "$d/pid")" && n=$((n+1)); '
            'done; echo "reaped $n"')


def launch_script(host: Host, m: dict, mc: dict, owner: str = "") -> str:
    """The shell that starts one match on its host. `owner` (`<host>:<pid>` of the orchestrator)
    goes into the match's environment (TTD_BATTLE_ORCHESTRATOR, for `ps eww`), the wrapper's argv
    (`ps` shows `ttd-battle[<owner>]`) and <match dir>/orchestrator: a match outlives its ssh session
    and is reparented to init/launchd, so this is how a target's user finds whose it is."""
    root, mdir = host.root, match_dir(host.root, m)
    personal = f"{mdir}/home/Documents/OpenTTD"
    owner = owner or "unknown"
    # A leftover directory would hand its old exit_code and log to this match: refuse it.
    return f"""set -e
export TTD_BATTLE_ORCHESTRATOR={owner}
test ! -e {mdir}
mkdir -p {personal}/scripts
cd {mdir}
echo "$TTD_BATTLE_ORCHESTRATOR" > orchestrator
for d in baseset ai game; do ln -sfn {root}/arena/$d {personal}/$d; done
# Linux (XDG) reads the personal directory from ~/.local/share/openttd: point it at the same one.
mkdir -p {mdir}/home/.local/share && ln -sfn {personal} {mdir}/home/.local/share/openttd
test -f {root}/arena/baseset/*.tar
cat > {personal}/scripts/game_start.scr <<'EOS'
{start_script_text(m['lineup'])}EOS
cat > battle.cfg <<'EOS'
{battle_cfg_text(mc)}EOS
nohup sh -c 'HOME={mdir}/home {host.binary} -c battle.cfg -x -v null:ticks={m['ticks']} \\
  -s null -m null -g -G {m['seed']} -d script=1 > match.log 2>&1 < /dev/null &
  echo $! > pid; wait $!; echo $? > exit_code' 'ttd-battle[{owner}]' > /dev/null 2>&1 < /dev/null &
echo launched
"""


def parse_log(text: str) -> tuple[dict, dict]:
    """({company id: [quarterly samples]}, {company id: Squirrel error count}) from a match log."""
    by_key: dict[tuple, dict] = {}
    errors: dict[int, int] = {}
    for line in text.splitlines():
        m = REF_LINE.search(line)
        if m:
            y, mo, cid, fields, name = m.groups()
            sample = {"t": f"{y}-{int(mo):02d}", "name": name.strip()}
            sample.update({k: int(v) for k, v in REF_FIELD.findall(fields)})
            # loan: since the referee logs it (2026-11-23); older logs and records read 0.
            for k in ("value", "money", "loan", "income", "perf", "vehicles", "stations"):
                sample.setdefault(k, 0)
            by_key[(int(cid), int(y), int(mo))] = sample
            continue
        e = SQ_ERROR.search(line)
        if e:
            errors[int(e.group(1))] = errors.get(int(e.group(1)), 0) + 1
    series: dict[int, list] = {}
    for (cid, _y, _mo), s in sorted(by_key.items()):
        series.setdefault(cid, []).append(s)
    return series, errors


def parse_shared(text: str) -> dict[int, dict]:
    """{industry id: {"name", "owners": the company ids, "first": the first quarter seen,
    "cargoes": {cargo id (None for a line without one): {"owners", "first"}}}} from a match log's
    REF S lines: every producer one of whose cargoes rail stations of two or more companies picked
    up in some quarter. `owners` joins every cargo and quarter; a cargo's own owners are the ones
    that shared that cargo."""
    shared: dict[int, dict] = {}
    for line in text.splitlines():
        m = REF_SHARED.search(line)
        if not m:
            continue
        y, mo, ind, cargo, owners, name = m.groups()
        quarter = f"{y}-{int(mo):02d}"
        ids = {int(o) for o in owners.split(",") if o}
        e = shared.setdefault(int(ind), {"name": name.strip(), "owners": set(), "first": quarter, "cargoes": {}})
        e["owners"] |= ids
        c = e["cargoes"].setdefault(int(cargo) if cargo is not None else None, {"owners": set(), "first": quarter})
        c["owners"] |= ids
    return shared


def parse_trainless(text: str) -> dict[int, list[dict]]:
    """{company id: [{"date", "rail", "trainless", "ids"}, ...] in report order} from a match log's
    REF T lines: per quarter, a company's rail stations, how many of them no train of its own calls
    at, and which (a set of station ids)."""
    out: dict[int, list[dict]] = {}
    for line in text.splitlines():
        m = REF_TRAINLESS.search(line)
        if not m:
            continue
        y, mo, cid, rail, bare = map(int, m.groups()[:5])
        ids = {int(i) for i in m.group(6).split(",") if i}
        out.setdefault(cid, []).append({"date": f"{y}-{mo:02d}", "rail": rail, "trainless": bare, "ids": ids})
    return out


def parse_funnel(text: str) -> dict[int, list[dict]]:
    """{company id: [{"year", "mode", <stage>: count, ...}, ...] by year, then mode} from a match
    log's FUNNEL lines (PrismaticAI/funnel.nut). A repeated year and mode keeps the last line."""
    by_key: dict[tuple, dict] = {}
    for line in text.splitlines():
        m = AI_FUNNEL.search(line)
        if not m:
            continue
        cid, year, mode, fields = m.groups()
        row = {"year": int(year), "mode": mode}
        row.update({k: int(v) for k, v in REF_FIELD.findall(fields)})
        by_key[(int(cid), int(year), mode)] = row
    out: dict[int, list[dict]] = {}
    for (cid, _y, _mode), row in sorted(by_key.items()):
        out.setdefault(cid, []).append(row)
    return out


def funnel_most_lost(row: dict) -> list[tuple[str, str, int, int]]:
    """Per population of one FUNNEL row (FUNNEL_DROPS, in its order), the stage that lost the largest
    share of what reached it: [(group, stage, lost, of), ...], ties to the larger loss; a group
    whose stages lost nothing is left out."""
    best: dict[str, tuple[str, str, int, int]] = {}
    for group, stage, lost_terms, of_terms in FUNNEL_DROPS.get(row["mode"], []):
        lost = sum(sign * row.get(k, 0) for k, sign in lost_terms)
        of = sum(sign * row.get(k, 0) for k, sign in of_terms)
        if lost <= 0 or of <= 0:
            continue
        b = best.get(group)
        if b is None or (lost / of, lost) > (b[2] / b[3], b[2]):
            best[group] = (group, stage, lost, of)
    return list(best.values())


def funnel_text(row: dict) -> str:
    """One FUNNEL row as the fitness check and the post-mortem print it."""
    counts = " ".join(f"{k}={v}" for k, v in row.items() if k not in ("year", "mode"))
    lost = ", ".join(f"{g} {st} {n}/{of}" for g, st, n, of in funnel_most_lost(row))
    return f"{row['mode']} {row['year']}: {counts}; most lost: {lost or 'none'}"


def funnel_gaps(m: dict, keys: list[str]) -> list[str]:
    """The fitness failures of contestants in `keys` (fitness.must_funnel) whose rail funnel misses a
    game year the match played whole: the line for year Y comes at the first heartbeat of Y+1, so a
    match of N years from start_year owes start_year .. start_year+N-2 (`m["funnel"]` by key)."""
    fails = []
    for c in m["lineup"]:
        if c["key"] not in keys:
            continue
        start, years = m["match_cfg"]["start_year"], m["years"]
        seen = {r["year"] for r in m.get("funnel", {}).get(c["key"], []) if r["mode"] == "rail"}
        missing = [y for y in range(start, start + years - 1) if y not in seen]
        if missing:
            fails.append(f"{c['key']}: no_funnel: no FUNNEL rail line for {', '.join(map(str, missing))}")
    return fails


def parse_capital(text: str) -> dict[int, list[dict]]:
    """{company id: [{"year", "quarter", "frozen", "days", ...}, ...] in report order} from a match
    log's PrismaticAI capital reports (CAP_LINE): per game quarter, the days its construction stood
    frozen (main.nut BuildFrozen, counted by capital.nut from heartbeat to heartbeat) out of the days
    counted, and the allocator's other counters."""
    out: dict[int, list[dict]] = {}
    for line in text.splitlines():
        m = CAP_LINE.search(line)
        if not m:
            continue
        rep = {"year": int(m.group(2)), "quarter": int(m.group(3))}
        rep.update({k: int(v) for k, v in REF_FIELD.findall(m.group(4))})
        out.setdefault(int(m.group(1)), []).append(rep)
    return out


def frozen_construction(m: dict, fit: dict) -> list[str]:
    """The fitness failures of healthy PrismaticAI contestants (`must_be_healthy`) whose construction
    stood frozen on more than `frozen_max` (default 0.15) of the days of their first `frozen_years`
    (default 3) game years, from `m["capital"]` (parse_capital, keyed by company id in start order).
    In the owner's live game of seed 12609 company 0 was frozen on 525 of the 1,096 days of 1950-1952
    (47.9%): RepaySurplus left the balance just above StayAfloat's floor, a few days of costs later
    the company "borrowed to stay afloat", and BuildFrozen stopped every build for 90 days while the
    loan was above half its limit. A contestant whose log has no capital report there (a bot without
    capital.nut) is not judged; one that reports fewer days than the window holds is judged on the
    days it reported."""
    rules = fit["rules"]
    limit, years = rules.get("frozen_max", 0.15), rules.get("frozen_years", 3)
    start = m.get("match_cfg", {}).get("start_year", 1950)
    fails = []
    for i, c in enumerate(m["lineup"]):
        if not c["ai"].startswith("PrismaticAI") or c["key"] not in fit.get("must_be_healthy", []):
            continue
        reps = [r for r in m.get("capital", {}).get(i, []) if r["year"] < start + years]
        days = sum(r.get("days", 0) for r in reps)
        frozen = sum(r.get("frozen", 0) for r in reps)
        if days > 0 and frozen > limit * days:
            fails.append(f"{c['key']}: frozen:{frozen} of {days} days ({frozen / days:.1%}) in {start}-{start + years - 1} "
                         f"(max {limit:.0%})")
    return fails


def trainless_stations(m: dict, rules: dict) -> list[str]:
    """The fitness failures of PrismaticAI companies that keep rail stations no train of theirs calls
    at (`m["trainless"]` from parse_trainless, keyed by company id in start order): more than
    `trainless_max` (default 0) stations that were trainless in each of the last `trainless_reports`
    (default 4, a game year) quarterly reports. The same station for a year, so the termini of a line
    still being laid do not count: on the fitness seed a double-track line took three quarters from
    its stations to its first train. In the owner's live game of seed 15805 10 of company 0's 12 rail
    stations had no train by 1958, two of them the termini of finished lines whose locomotive was
    refused for money (research/incident-2026-11-23-trainless-lines.md). A contestant in
    `trainless_skip` is not judged: the test_trainless lineup refuses each line's first train on
    purpose and is judged by `must_run_trains` instead."""
    limit, n = rules.get("trainless_max", 0), rules.get("trainless_reports", 4)
    fails = []
    for i, c in enumerate(m["lineup"]):
        if not c["ai"].startswith("PrismaticAI") or c["key"] in rules.get("trainless_skip", []):
            continue
        last = m.get("trainless", {}).get(i, [])[-n:]
        if len(last) < n:
            continue
        kept = set.intersection(*(r["ids"] for r in last))
        if len(kept) > limit:
            fails.append(f"{c['key']}: trainless_stations:{len(kept)} rail station(s) without a train from "
                         f"{last[0]['date']} to {last[-1]['date']} (stations {', '.join(map(str, sorted(kept)))}; "
                         f"max {limit})")
    return fails


def sibling_producers(m: dict) -> list[str]:
    """The fitness failures of producers that two or more PrismaticAI companies of one match serve
    by rail (`m["shared"]` from parse_shared, keyed by company id in start order). Two lines at one
    mine split its output: the owner's live game of 2026-11-22 had four PrismaticAI companies stall
    from 1952 on exactly this (research/incident-2026-11-22-sibling-industries.md)."""
    prismatic = {i: c["key"] for i, c in enumerate(m["lineup"]) if c["ai"].startswith("PrismaticAI")}
    fails = []
    for ind, e in sorted(m.get("shared", {}).items()):
        # Per cargo: two companies taking different cargoes of one farm split nothing.
        groups = e.get("cargoes") or {None: {"owners": e["owners"], "first": e["first"]}}
        for cargo, g in sorted(groups.items(), key=lambda kv: (kv[0] is not None, kv[0] or 0)):
            ours = sorted(prismatic[o] for o in g["owners"] if o in prismatic)
            if len(ours) >= 2:
                what = "" if cargo is None else f", cargo {cargo}"
                fails.append(f"shared_producer:{e['name']} (industry {ind}{what}, from {g['first']}): "
                             + " + ".join(ours))
    return fails


def standings_of(m: dict, rules: dict | None = None) -> list[dict]:
    final = []
    for c in m["lineup"]:
        s = m["series"].get(c["key"], [])
        last = s[-1] if s else {}
        errors = m["errors"].get(c["key"], 0)
        final.append({"key": c["key"], "value": last.get("value", 0),
                      "vehicles": last.get("vehicles", 0), "stations": last.get("stations", 0),
                      "income": last.get("income", 0), "errors": errors,
                      "anomalies": anomalies(s, errors, rules or {})})
    final.sort(key=lambda f: (-f["value"], f["key"]))
    return final


def match_status(m: dict, exit_code: int) -> str:
    """done only when the binary exited 0 and the final year's January report exists."""
    last_t = max((s[-1]["t"] for s in m["series"].values() if s), default="")
    expected = f"{m['match_cfg']['start_year'] + m['years']}-01"
    return "done" if exit_code == 0 and last_t >= expected else "incomplete"


SCORING = "points-per-match/v2"


def leaderboard(contestants: list[str], matches: list[dict], scoring: str = SCORING) -> list[dict]:
    """Points per match: last place 0, each place above +1. Replays and unfinished matches don't count."""
    rows = {k: {"key": k, "played": 0, "wins": 0, "points": 0, "rank_sum": 0, "value_sum": 0,
                "errors": 0, "anomalies": 0} for k in contestants}
    for m in matches:
        if m["status"] != "done" or m.get("replica_of"):
            continue
        n = len(m["standings"])
        for rank, f in enumerate(m["standings"]):
            r = rows.setdefault(f["key"], {"key": f["key"], "played": 0, "wins": 0, "points": 0,
                                           "rank_sum": 0, "value_sum": 0, "errors": 0,
                                           "anomalies": 0})
            r["played"] += 1
            r["wins"] += rank == 0
            r["points"] += n - 1 - rank
            r["rank_sum"] += rank + 1
            r["value_sum"] += f["value"]
            r["errors"] += f["errors"]
            r["anomalies"] += bool(f.get("anomalies"))
    out = list(rows.values())
    for r in out:
        r["avg_rank"] = round(r["rank_sum"] / r["played"], 2) if r["played"] else None
        r["avg_value"] = r["value_sum"] // r["played"] if r["played"] else 0
        del r["rank_sum"], r["value_sum"]
        if scoring == SCORING:
            # Grouped contestants play one match in N: rank per match played, not by total.
            r["avg_points"] = round(r["points"] / r["played"], 3) if r["played"] else 0
    if scoring == SCORING:
        out.sort(key=lambda r: (-r["avg_points"], -r["avg_value"], r["key"]))
    else:  # v1 (records without "scoring"): total points
        out.sort(key=lambda r: (-r["points"], -r["avg_value"], r["key"]))
    return out


def determinism(matches: list[dict]) -> list[dict]:
    """Every replay against its original: same seed, lineup and length on another arena."""
    by_id = {m["id"]: m for m in matches}
    out = []
    for m in matches:
        o = by_id.get(m.get("replica_of") or "")
        if m["status"] != "done" or o is None or o["status"] != "done":
            continue
        a = [(f["key"], f["value"]) for f in o["standings"]]
        b = [(f["key"], f["value"]) for f in m["standings"]]
        out.append({"seed": m["seed"], "original": o["id"], "replay": m["id"],
                    "hosts": [o["host"], m["host"]],
                    "versions": [o.get("host_version", ""), m.get("host_version", "")],
                    "identical": a == b})
    return out


def postmortem(m: dict) -> list[dict]:
    """Where each contestant lost to the winner of a finished match: the final gap, the year the
    winner pulled away (twice the value), and the largest deficits by transport mode, fleet size,
    income per vehicle and cargo delivered. Derived from the referee's quarterly samples only."""
    st = m.get("standings") or []
    if not st:
        return []
    win = st[0]["key"]
    ws = m["series"].get(win, [])
    wlast = ws[-1] if ws else {}
    wby_t = {s["t"]: s for s in ws}
    out = []
    for f in st[1:]:
        s = m["series"].get(f["key"], [])
        if not s:
            continue
        last = s[-1]
        behind = next((x["t"] for x in s if x["t"] in wby_t and x["value"] > 0
                       and wby_t[x["t"]]["value"] > 2 * x["value"]), None)
        findings = []
        if wlast.get("vehicles", 0) > 1.5 * max(1, last.get("vehicles", 0)):
            findings.append(f"fleet {last.get('vehicles', 0)} vs {wlast['vehicles']}")
        ipv, wipv = (last.get("income", 0) / max(1, last.get("vehicles", 0)),
                     wlast.get("income", 0) / max(1, wlast.get("vehicles", 0)))
        if wipv > 1.5 * max(1, ipv):
            findings.append(f"income per vehicle {ipv:,.0f} vs {wipv:,.0f}")
        if "cargo" in last and wlast.get("cargo", 0) > 1.5 * max(1, last["cargo"]):
            findings.append(f"cargo delivered/q {last['cargo']:,} vs {wlast['cargo']:,}")
        for mode in MODES:
            mine, theirs = last.get("p" + mode), wlast.get("p" + mode)
            if mine is None or theirs is None:
                continue
            if theirs - mine > 0.2 * max(1, wlast.get("income", 0)) * 4:
                findings.append(f"{mode} profit/yr {mine:,} vs {theirs:,} "
                                f"({last.get(mode, 0)} vs {wlast.get(mode, 0)} vehicles)")
        findings += [f"anomaly {a}" for a in f.get("anomalies", [])]
        # The growth funnel's last year per mode, where the contestant logs one (funnel.nut).
        last_rows: dict[str, dict] = {}
        for r in m.get("funnel", {}).get(f["key"], []):
            last_rows[r["mode"]] = r
        findings += [f"funnel {funnel_text(r)}" for r in last_rows.values()]
        out.append({"contestant": f["key"], "winner": win,
                    "gap": round(wlast.get("value", 0) / max(1, last.get("value", 1)), 2),
                    "behind_since": behind,
                    "mix": {mode: last.get(mode) for mode in MODES if mode in last},
                    "winner_mix": {mode: wlast.get(mode) for mode in MODES if mode in wlast},
                    "findings": findings})
    return out


SCHEMA = """
CREATE TABLE IF NOT EXISTS matches (id TEXT, edition TEXT, host TEXT, host_version TEXT,
  seed INTEGER, years INTEGER, ticks INTEGER, status TEXT, exit_code INTEGER, started_at REAL,
  finished_at REAL, replica_of TEXT, lineup TEXT, PRIMARY KEY (edition, id));
CREATE TABLE IF NOT EXISTS samples (edition TEXT, match_id TEXT, contestant TEXT, t TEXT,
  value INTEGER, money INTEGER, income INTEGER, perf INTEGER, vehicles INTEGER, stations INTEGER,
  rail INTEGER, road INTEGER, air INTEGER, water INTEGER, prail INTEGER, proad INTEGER,
  pair INTEGER, pwater INTEGER, cargo INTEGER, expenses INTEGER, loan INTEGER,
  PRIMARY KEY (edition, match_id, contestant, t));
CREATE TABLE IF NOT EXISTS standings (edition TEXT, match_id TEXT, rank INTEGER, contestant TEXT,
  value INTEGER, vehicles INTEGER, stations INTEGER, errors INTEGER, anomalies TEXT,
  PRIMARY KEY (edition, match_id, contestant));
CREATE TABLE IF NOT EXISTS postmortems (edition TEXT, match_id TEXT, contestant TEXT, winner TEXT,
  gap REAL, behind_since TEXT, findings TEXT, mix TEXT, winner_mix TEXT,
  PRIMARY KEY (edition, match_id, contestant));
CREATE TABLE IF NOT EXISTS host_perf (edition TEXT, match_id TEXT, host TEXT, host_version TEXT,
  game_years REAL, wall_s REAL, years_per_min REAL, PRIMARY KEY (edition, match_id));
CREATE TABLE IF NOT EXISTS events (at REAL, edition TEXT, msg TEXT);
"""
# loan: since the referee logs it (2026-11-23). Without it a stored `money` (the bank balance, the
# loan included) cannot be judged net of the loan, as the hoard rules judge the live series.
SAMPLE_COLS = ("value", "money", "income", "perf", "vehicles", "stations", "rail", "road", "air",
               "water", "prail", "proad", "pair", "pwater", "cargo", "expenses", "loan")


class Store:
    """Every sample, standing, post-mortem, throughput figure and event, in SQLite (machine-friendly;
    DuckDB reads it with ATTACH '<file>' (TYPE sqlite)). One writer: the orchestrator's loop."""

    def __init__(self, path: Path):
        self.path = path
        # WAL: readers (the API, battle-postmortem, a notebook) never block the writer or each other,
        # and a busy timeout instead of "database is locked" when they meet a checkpoint.
        self.db = sqlite3.connect(path, check_same_thread=False, timeout=30)
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA busy_timeout=30000")
        self.db.executescript(SCHEMA)
        # A store made before a sample column existed gets it added (columns are only ever added,
        # never renamed); its older rows read NULL there, which says "not recorded", not 0.
        have = {r[1] for r in self.db.execute("PRAGMA table_info(samples)")}
        for col in SAMPLE_COLS:
            if col not in have:
                self.db.execute(f"ALTER TABLE samples ADD COLUMN {col} INTEGER")
        self.db.commit()
        self.lock = threading.Lock()

    def match(self, m: dict) -> None:
        with self.lock, self.db:
            self.db.execute("INSERT OR REPLACE INTO matches VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", (
                m["id"], m["edition"], m["host"], m.get("host_version", ""), m["seed"], m["years"],
                m["ticks"], m["status"], m.get("exit_code"), m["started_at"], m.get("finished_at"),
                m.get("replica_of"), json.dumps([{"key": c["key"], "ai": c["ai"],
                                                  "settings": c.get("settings", "")}
                                                 for c in m["lineup"]])))
            rows = [(m["edition"], m["id"], key, s["t"], *[s.get(c) for c in SAMPLE_COLS])
                    for key, series in m["series"].items() for s in series]
            # By name: an added column sits last in an old store and in its place in a new one.
            cols = ("edition", "match_id", "contestant", "t", *SAMPLE_COLS)
            self.db.executemany(f"INSERT OR REPLACE INTO samples ({','.join(cols)}) VALUES "
                                f"({','.join('?' * len(cols))})", rows)
            for rank, f in enumerate(m.get("standings") or [], 1):
                self.db.execute("INSERT OR REPLACE INTO standings VALUES (?,?,?,?,?,?,?,?,?)", (
                    m["edition"], m["id"], rank, f["key"], f["value"], f["vehicles"],
                    f["stations"], f["errors"], json.dumps(f.get("anomalies", []))))
            for pm in m.get("postmortem") or []:
                self.db.execute("INSERT OR REPLACE INTO postmortems VALUES (?,?,?,?,?,?,?,?,?)", (
                    m["edition"], m["id"], pm["contestant"], pm["winner"], pm["gap"],
                    pm["behind_since"], json.dumps(pm["findings"]), json.dumps(pm["mix"]),
                    json.dumps(pm["winner_mix"])))
            if m.get("perf"):
                p = m["perf"]
                self.db.execute("INSERT OR REPLACE INTO host_perf VALUES (?,?,?,?,?,?,?)", (
                    m["edition"], m["id"], m["host"], m.get("host_version", ""), p["game_years"],
                    p["wall_s"], p["years_per_min"]))

    def event(self, edition: str, msg: str) -> None:
        with self.lock, self.db:
            self.db.execute("INSERT INTO events VALUES (?,?,?)", (time.time(), edition, msg))

    def query(self, sql: str, cap: int = 5000) -> dict:
        """Read-only SQL for the API: one SELECT/WITH statement on a read-only connection."""
        if not re.match(r"\s*(select|with)\b", sql, re.I) or ";" in sql.strip().rstrip(";"):
            return {"error": "one read-only SELECT or WITH statement only"}
        ro = sqlite3.connect(f"file:{self.path}?mode=ro", uri=True, timeout=30)
        try:
            cur = ro.execute(sql)
            cols = [d[0] for d in cur.description or []]
            rows = cur.fetchmany(cap + 1)
            return {"columns": cols, "rows": rows[:cap], "count": min(len(rows), cap),
                    "capped": len(rows) > cap}
        except sqlite3.Error as exc:
            return {"error": str(exc)}
        finally:
            ro.close()


OPENAPI = {
    "openapi": "3.0.3",
    "info": {"title": "LAN AI battle", "version": "1",
             "description": "RESEARCH-ONLY. Read-only view of the running tournament "
                            "(tools/research/ai-battle/battle.py run). See research/ai-battle/README.md."},
    "paths": {
        "/api/state": {"get": {"summary": "Everything the dashboard shows: edition, arenas, live "
                                          "matches with quarterly series, leaderboard, replays, events",
                               "responses": {"200": {"description": "state"}}}},
        "/api/stream": {"get": {"summary": "The same state as server-sent events (event: state), "
                                           "pushed every stream_every_s",
                                "responses": {"200": {"description": "text/event-stream"}}}},
        "/api/leaderboard": {"get": {"summary": "Leaderboard and replay comparison of the edition",
                                     "responses": {"200": {"description": "leaderboard"}}}},
        "/api/hosts": {"get": {"summary": "Arenas: state, build, slots, running and done matches",
                               "responses": {"200": {"description": "hosts"}}}},
        "/api/matches": {"get": {"summary": "Every match of the edition: status, anomalies, standings",
                                 "responses": {"200": {"description": "matches"}}}},
        "/api/postmortems": {"get": {"summary": "Where each contestant lost to the winner, per "
                                                 "finished match of the edition (newest first)",
                                      "responses": {"200": {"description": "post-mortems"}}}},
        "/api/tuning": {"get": {"summary": "Self-tuning: the generation, every variant's settings, "
                                           "parents and fitness, the anchors, and the history",
                                "responses": {"200": {"description": "tuning state"}}}},
        "/api/promotions": {"get": {"summary": "Every contestant paired with the default on common seeds, "
                                               "and the configurations that qualify as its successor",
                                    "responses": {"200": {"description": "paired, promotions"}}}},
        "/api/perf": {"get": {"summary": "Throughput per arena: game years per wall-clock minute",
                              "responses": {"200": {"description": "per-host throughput"}}}},
        "/api/query": {"get": {"summary": "Read-only SQL over the store (tables matches, samples, "
                                          "standings, postmortems, host_perf, events)",
                               "parameters": [{"name": "sql", "in": "query", "required": True,
                                               "schema": {"type": "string"}}],
                               "responses": {"200": {"description": "{columns, rows, count, capped}"}}}},
        "/api/matches/{id}": {"get": {"summary": "One match with its full quarterly series",
                                      "parameters": [{"name": "id", "in": "path", "required": True,
                                                      "schema": {"type": "string"}}],
                                      "responses": {"200": {"description": "match"},
                                                    "404": {"description": "no such match"}}}},
    },
}

SWAGGER_PAGE = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>LAN AI Battle API</title>
<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/swagger-ui/5.17.14/swagger-ui.min.css">
</head><body><div id="ui"></div>
<script src="https://cdnjs.cloudflare.com/ajax/libs/swagger-ui/5.17.14/swagger-ui-bundle.min.js"></script>
<script>SwaggerUIBundle({url: '/api/openapi.json', dom_id: '#ui'});</script></body></html>"""


class Orchestrator:
    def __init__(self, cfg: dict, edition: str | None = None):
        self.cfg = cfg
        self.digest = edition or edition_of(cfg)
        self.provenance = json.loads((cfg["cache"] / "arena" / "PROVENANCE.json").read_text())
        self.mdir = cfg["results"] / self.digest / "matches"
        self.mdir.mkdir(parents=True, exist_ok=True)
        # Which code this edition runs: GA generations are new editions of the same bot, and pairing
        # for promotion pools every edition of the same arena revision.
        rev = json.loads((cfg["cache"] / "arena" / "PROVENANCE.json").read_text())["rev"]
        (self.mdir.parent / "edition.json").write_text(json.dumps({"edition": self.digest, "rev": rev}) + "\n")
        self.rev = rev
        self.store = Store(cfg["results"] / "battle.sqlite")
        self.hosts = [Host(h, cfg.get("remote_root", "$HOME/ttd-battle"), cfg.get("resource_floor"))
                      for h in load_hosts(cfg)]
        self.contestants = [dict(c, key=contestant_key(c)) for c in cfg["contestants"]]
        self.matches: dict[str, dict] = {}
        for f in sorted(self.mdir.glob("*.json")):
            m = json.loads(f.read_text())
            self.matches[m["id"]] = m
        if edition and self.matches:
            # A past edition: its contestants are every one its matches were played with (with groups,
            # one match holds only one member of a group), in first-seen order.
            seen: dict[str, dict] = {}
            for m in sorted(self.matches.values(), key=lambda m: m["id"]):
                for c in m["lineup"]:
                    seen.setdefault(c["key"], dict(c))
            self.contestants = list(seen.values())
        self.counter = max([int(m["id"].split("-")[1]) for m in self.matches.values()] + [0])
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.events: list[dict] = []
        self.promoted: set[str] = set()
        # Who this orchestrator is, on every host it touches (ORCHESTRATOR file, match environment).
        self.hostname, self.pid = socket.gethostname(), os.getpid()
        self.owner = owner_marker(self.hostname, self.pid)
        self.started_at = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
        try:
            self.commit = git("rev-parse", "HEAD")
        except (subprocess.CalledProcessError, OSError):
            self.commit = "unknown"
        self.held_by_stop: str | None = None

    def save(self, m: dict) -> None:
        tmp = self.mdir / f"{m['id']}.json.tmp"
        tmp.write_text(json.dumps(m))
        tmp.replace(self.mdir / f"{m['id']}.json")
        self.store.match(m)

    def event(self, msg: str) -> None:
        log(msg)
        with self.lock:
            self.events.append({"at": time.strftime("%H:%M:%S"), "msg": msg})
            del self.events[:-80]
        self.store.event(self.digest, msg)

    # ---- scheduling
    def replay_candidate(self, host: Host) -> dict | None:
        """A finished, never-replayed match from another arena, to replay here."""
        replayed = {m.get("replica_of") for m in self.matches.values()}
        for m in sorted(self.matches.values(), key=lambda m: m["id"]):
            if (m["status"] == "done" and not m.get("replica_of") and m["host"] != host.name
                    and m["id"] not in replayed):
                return m
        return None

    def new_match(self, host: Host) -> dict:
        self.counter += 1
        mc, every = self.cfg["match"], int(self.cfg.get("replay_every", 0))
        original = self.replay_candidate(host) if every and self.counter % every == 0 else None
        if original:
            seed, lineup = original["seed"], original["lineup"]
        else:
            # Common random numbers: the members of a group take turns on the SAME seed, so every
            # variant meets the same maps (one seed per round of the group), from the same slot.
            sizes = {}
            for c in self.contestants:
                if c.get("group"):
                    sizes[c["group"]] = sizes.get(c["group"], 0) + 1
            block = max(sizes.values(), default=1)
            lineup = lineup_for(self.contestants, self.counter, block)
            # The edition is part of the seed: every GA generation meets new maps instead of being
            # judged (and tuned) on the same handful of seeds again and again.
            seed = int(hashlib.sha256(f"{self.cfg['seed']}:{self.digest}:{self.counter // block}".encode())
                       .hexdigest()[:8], 16) % 2_000_000_000
        return {"id": f"m-{self.counter:04d}", "host": host.name, "host_version": host.version,
                "seed": seed, "years": mc["years"], "ticks": match_ticks(mc["years"]),
                "lineup": lineup, "replica_of": original["id"] if original else None,
                "status": "starting", "started_at": time.time(), "finished_at": None,
                "series": {}, "errors": {}, "standings": None, "edition": self.digest,
                "match_cfg": mc}

    def running_on(self, host: Host) -> list[dict]:
        return [m for m in self.matches.values()
                if m["host"] == host.name and m["status"] in ("starting", "running")]

    def reap(self) -> None:
        for host in self.hosts:
            try:
                r = host.sh(reap_script(host.root, self.digest), timeout=30)
            except subprocess.TimeoutExpired:
                continue
            n = r.stdout.strip().replace("reaped ", "")
            if r.returncode == 0 and n not in ("", "0"):
                self.event(f"host {host.name}: stopped {n} match(es) of an older edition")

    def reload_hosts(self) -> None:
        """Pick up hosts added to the local hosts file (e.g. by `discover`), and slot counts changed
        in it (e.g. a host handed back after another experiment), without a restart."""
        known = {h.name: h for h in self.hosts}
        for spec in load_hosts(self.cfg):
            if spec["name"] in known:
                h, slots = known[spec["name"]], int(spec.get("slots", 1))
                if h.slots != slots:
                    self.event(f"host {h.name}: slots {h.slots} -> {slots} from the hosts file")
                    h.slots = slots
                limits = floor_limits(self.cfg.get("resource_floor"), spec)
                if h.limits != limits:
                    self.event(f"host {h.name}: resource floor {limits} from the hosts file")
                    h.limits = limits
            else:
                self.hosts.append(Host(spec, self.cfg.get("remote_root", "$HOME/ttd-battle"),
                                       self.cfg.get("resource_floor")))
                self.event(f"host {spec['name']}: added from the hosts file")

    def tuning_step(self) -> bool:
        """Self-tuning: when every variant of the generation has played its matches, publish the
        round's record, breed the next generation and restart into its edition. True: restarting."""
        t = self.cfg.get("tuning")
        if not t or not t.get("enabled"):
            return False
        state = tuning_state(self.cfg)
        variants = state["variants"]
        ms = list(self.matches.values())
        scored = [(v, fitness_of(v["label"], ms, t.get("last_penalty", 0.0))) for v in variants]
        if any(f["matches"] < t["matches_per_variant"] for _, f in scored):
            return False
        anchors = {k: fitness_of(k, ms) for k in t.get("anchors", [])}
        self.event(f"TUNING generation {state['generation']} done: " + "; ".join(
            f"{v['label']} {f['score']}" for v, f in sorted(scored, key=lambda x: -x[1]["score"])))
        try:
            cmd_report(self.cfg, False, self.digest)
        except Exception as exc:  # the record is a by-product; tuning goes on
            self.event(f"TUNING report failed: {exc}")
        state["history"].append({"generation": state["generation"], "edition": self.digest,
                                 "at": time.time(), "anchors": anchors,
                                 "variants": [{"label": v["label"], "genes": v["genes"],
                                               "parents": v["parents"], "fitness": f}
                                              for v, f in scored]})
        # Live games (tracked research/results/live-*.json) veto the configurations that lost them.
        live = live_entries(live_records(), t) if t.get("live") else []
        for v, _ in scored:
            veto = live_veto(t, v["genes"], live)
            if veto:
                self.event(f"TUNING {v['label']}: lost live game {veto['record']} ({veto['why']}): vetoed")
        state["variants"] = next_generation(t, state, scored, live)
        state["generation"] += 1
        tuning_path(self.cfg).write_text(json.dumps(state, indent=1) + "\n")
        # Matches of the finished generation that still run are stopped through their pid files.
        for m in ms:
            if m["status"] == "running":
                host = next((h for h in self.hosts if h.name == m["host"]), None)
                if host:
                    host.try_sh(f"kill $(cat {match_dir(host.root, m)}/pid) 2>/dev/null; true")
                m["status"] = "abandoned"
                self.save(m)
        self.event(f"TUNING generation {state['generation']}: " + ", ".join(
            f"{v['label']} [{v['settings']}]" for v in state["variants"]) + "; restarting")
        return True

    def stop_file_holds(self) -> bool:
        """True while <local root>/STOP exists: no new match anywhere. Logs when it appears and goes."""
        why = stop_requested(self.cfg["cache"])
        if why is not None and self.held_by_stop is None:
            self.event(f"STOP: {self.cfg['cache'] / 'STOP'} exists ({why}): no new match on any host; "
                       "running matches finish and are still read")
        elif why is None and self.held_by_stop is not None:
            self.event(f"STOP: {self.cfg['cache'] / 'STOP'} removed: scheduling resumes")
        self.held_by_stop = why
        return why is not None

    def orchestrator_file(self, host: Host) -> str:
        return orchestrator_text(self.hostname, self.pid, self.started_at, self.commit, self.rev,
                                 self.digest, host.root, str(self.cfg["cache"]))

    def schedule_pass(self) -> None:
        self.reload_hosts()
        if self.stop_file_holds():
            return
        for host in self.hosts:
            if time.time() < host.next_probe:
                continue
            prev = host.state
            host.probe()
            n = host.admit(len(self.running_on(host)))
            if host.state != prev:
                self.event(f"host {host.name}: {prev} -> {host.state} {host.detail}".strip())
            unparsed = tuple(host.res.get("unparsed", [])) if host.state != "offline" else ()
            if unparsed and unparsed != getattr(host, "unparsed_logged", ()):
                self.event(f"host {host.name}: probe figures not understood, not blocking on them: "
                           + "; ".join(unparsed)[:300])
            host.unparsed_logged = unparsed
            if host.state in ("offline", "no-binary", "error"):
                host.next_probe = time.time() + self.cfg.get("offline_probe_every_s", 300)
                continue
            if n <= 0:   # paused, busy, or every slot taken: probed again next pass
                continue
            try:
                host.sync_arena(self.cfg["cache"], self.digest)
            except Exception as exc:  # one broken host must not stop the others
                host.state, host.detail = "error", str(exc)[:200]
                self.event(f"host {host.name}: arena sync failed: {exc}")
                continue
            if not host.claimed:
                # A match nobody on the target can trace back to this process is not started.
                if not host.claim(self.orchestrator_file(host)):
                    host.state, host.detail = "error", f"cannot write {host.root}/ORCHESTRATOR"
                    self.event(f"host {host.name}: {host.detail}; no match started there")
                    continue
                self.event(f"host {host.name}: wrote {host.root}/ORCHESTRATOR ({self.owner})")
            for _ in range(n):
                if self.stop.is_set() or self.stop_file_holds():
                    break
                m = self.new_match(host)
                r = host.try_sh(launch_script(host, m, self.cfg["match"], self.owner))
                m["status"] = "running" if "launched" in r.stdout else "failed"
                tag = f" (replay of {m['replica_of']})" if m["replica_of"] else ""
                self.event(f"{m['id']} started on {host.name}{tag}, seed {m['seed']}"
                           if m["status"] == "running" else
                           f"{m['id']} failed to launch on {host.name}: {r.stderr.strip()[:200]}")
                with self.lock:
                    self.matches[m["id"]] = m
                self.save(m)
                if m["status"] == "failed":
                    break

    # ---- observation
    def poll_host(self, host: Host) -> None:
        running = self.running_on(host)
        # A paused or busy host takes no new match, but the ones it runs are still read to the end.
        if not running or host.state not in ("online", "busy", "paused"):
            return
        parts = []
        for m in running:
            d = match_dir(host.root, m)
            parts.append(f"echo '@@@ {m['id']}'; cat {d}/exit_code 2>/dev/null || echo RUNNING; "
                         f"grep -a -E 'REF Q|\\] \\[S\\] |\\] \\[E\\] FUNNEL ' {d}/match.log 2>/dev/null | tail -n 4000")
        try:
            r = host.sh("; ".join(parts), timeout=30)
        except subprocess.TimeoutExpired:
            return
        if r.returncode == 255:
            return
        for chunk in r.stdout.split("@@@ ")[1:]:
            mid, _, rest = chunk.partition("\n")
            status_line, _, body = rest.partition("\n")
            m = self.matches.get(mid.strip())
            if m is None:
                continue
            series, errors = parse_log(body)
            samples = sum(len(v) for v in series.values())
            if samples > m.get("samples", 0):
                m["samples"], m["progress_at"] = samples, time.time()
            lineup = m["lineup"]
            # Company ids follow the start order: a `-g` null-driver game has no human company.
            m["series"] = {lineup[c]["key"]: s for c, s in series.items() if c < len(lineup)}
            m["errors"] = {lineup[c]["key"]: n for c, n in errors.items() if c < len(lineup)}
            m["funnel"] = {lineup[c]["key"]: r for c, r in parse_funnel(body).items() if c < len(lineup)}
            self.watch(m)
            if status_line.strip() != "RUNNING":
                self.finish(m, int(status_line.strip() or -1))
            elif time.time() - m.get("progress_at", m["started_at"]) > self.cfg.get("stall_s", 600):
                # A hung binary (e.g. a blocking OS error dialog) never writes exit_code.
                d = match_dir(host.root, m)
                host.try_sh(f"kill $(cat {d}/pid) 2>/dev/null; echo 137 > {d}/exit_code")
                self.event(f"{m['id']} on {host.name}: no referee output for "
                           f"{self.cfg.get('stall_s', 600)}s, killed")
            else:
                self.save(m)

    def watch(self, m: dict) -> None:
        """Self-monitoring: every contestant's anomalies, live; a new one goes to the event log."""
        rules = self.cfg.get("anomaly_rules", {})
        live = {c["key"]: anomalies(m["series"].get(c["key"], []), m["errors"].get(c["key"], 0), rules)
                for c in m["lineup"]}
        seen = m.setdefault("alerted", {})
        for key, found in live.items():
            for a in found:
                kind = a.split(":")[0]
                if kind not in seen.get(key, []):
                    seen.setdefault(key, []).append(kind)
                    self.event(f"ALERT {m['id']} on {m['host']}: {key} {a}")
        m["anomalies"] = {k: v for k, v in live.items() if v}

    def sibling_matches(self) -> list[dict]:
        """Finished matches of every edition that runs the same arena revision (GA generations)."""
        out = [m for m in self.matches.values() if m.get("status") == "done"]
        for ed in self.cfg["results"].glob("*/edition.json"):
            try:
                info = json.loads(ed.read_text())
            except ValueError:
                continue
            if info.get("rev") != self.rev or info.get("edition") == self.digest:
                continue
            for f in (ed.parent / "matches").glob("*.json"):
                m = json.loads(f.read_text())
                if m.get("status") == "done":
                    out.append(m)
        return out

    def announce_promotions(self) -> None:
        for p in promotions(self.sibling_matches(), self.cfg.get("promotion", {})):
            key = p["contestant"]
            if key not in self.promoted:
                self.promoted.add(key)
                self.event(f"PROMOTE {key}: beat {p['base']} on {p['wins']}/{p['pairs']} common seeds, "
                           f"mean ratio {p['ratio']}; candidate for the next default")

    def finish(self, m: dict, code: int) -> None:
        self.watch(m)
        m["standings"] = standings_of(m, self.cfg.get("anomaly_rules", {}))
        m["exit_code"], m["finished_at"] = code, time.time()
        m["status"] = match_status(m, code)
        m["postmortem"] = postmortem(m)
        wall = m["finished_at"] - m["started_at"]
        last_t = max((s[-1]["t"] for s in m["series"].values() if s), default="")
        years = (int(last_t[:4]) + (int(last_t[5:]) - 1) / 12 - m["match_cfg"]["start_year"]) if last_t else 0
        m["perf"] = {"game_years": round(years, 2), "wall_s": round(wall, 1),
                     "years_per_min": round(years / max(wall, 1) * 60, 2)}
        self.save(m)
        self.announce_promotions()
        for pm in m["postmortem"]:
            if pm["contestant"].startswith("PrismaticAI") and pm["findings"]:
                self.event(f"POSTMORTEM {m['id']}: {pm['contestant']} {pm['gap']}x behind "
                           f"{pm['winner']}" + (f" since {pm['behind_since']}" if pm["behind_since"] else "")
                           + ": " + "; ".join(pm["findings"][:3]))
        w = m["standings"][0] if m["standings"] else None
        self.event(f"{m['id']} on {m['host']} {m['status']}"
                   + (f": winner {w['key']} ({w['value']:,})" if w else ""))

    # ---- views
    def tuning_view(self) -> dict | None:
        t = self.cfg.get("tuning")
        if not t or not t.get("enabled"):
            return None
        state = tuning_state(self.cfg)
        ms = list(self.matches.values())
        return {"generation": state["generation"], "matches_per_variant": t["matches_per_variant"],
                "space": t["space"],
                "variants": [{"label": v["label"], "settings": v["settings"], "parents": v["parents"],
                              "fitness": fitness_of(v["label"], ms, t.get("last_penalty", 0.0))}
                             for v in state["variants"]],
                "anchors": {k: fitness_of(k, ms) for k in t.get("anchors", [])},
                "history": [{"generation": h["generation"], "best": max(
                    (v["fitness"]["score"] for v in h["variants"]), default=None),
                    "anchors": {k: a["score"] for k, a in h["anchors"].items()}}
                    for h in state["history"]]}

    def perf(self) -> dict:
        """Game years per wall-clock minute per arena, over its finished matches of the edition."""
        out = {}
        for m in self.matches.values():
            p = m.get("perf")
            if p and m["status"] == "done":
                h = out.setdefault(m["host"], {"matches": 0, "game_years": 0.0, "wall_s": 0.0})
                h["matches"] += 1
                h["game_years"] += p["game_years"]
                h["wall_s"] += p["wall_s"]
        for h in out.values():
            h["years_per_min"] = round(h["game_years"] / max(h["wall_s"], 1) * 60, 2)
        return out

    def state(self) -> dict:
        now = time.time()
        if now - getattr(self, "_pair_at", 0) > 30:  # pooled over sibling editions: cached 30 s
            pool = self.sibling_matches()
            self._paired = paired_vs(pool, self.cfg.get("promotion", {}).get("base", "PrismaticAI"))
            self._promos = promotions(pool, self.cfg.get("promotion", {}))
            self._pair_at = now
        paired, promos = self._paired, self._promos
        with self.lock:
            matches = sorted(self.matches.values(), key=lambda m: m["id"], reverse=True)
            keys = [c["key"] for c in self.contestants]
            return {
                "now": time.time(), "edition": self.digest, "provenance": self.provenance,
                "match_cfg": self.cfg["match"], "contestants": keys,
                "hosts": [{"name": h.name, "state": h.state, "detail": h.detail,
                           "version": h.version, "slots": h.slots,
                           "running": len(self.running_on(h)),
                           "done": sum(1 for m in matches
                                       if m["host"] == h.name and m["status"] == "done")}
                          for h in self.hosts],
                "live": [m for m in matches if m["status"] == "running"],
                "recent": [{k: m.get(k) for k in ("id", "host", "seed", "status", "standings",
                                                  "replica_of", "years")}
                           for m in matches if m["status"] not in ("running", "starting")][:30],
                "leaderboard": leaderboard(keys, matches),
                "determinism": determinism(matches),
                "perf": self.perf(),
                "tuning": self.tuning_view(),
                "paired": paired, "promotions": promos,
                "postmortems": [{"match": m["id"], "postmortem": m["postmortem"]} for m in matches
                                if m.get("postmortem")][:12],
                "events": list(self.events),
            }

    def run(self, port: int) -> None:
        orch = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def send_body(self, body: bytes, ctype: str, code: int = 200):
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Cache-Control", "no-store")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # the client (a poller or a closed tab) left before the reply

            def json(self, data, code: int = 200):
                self.send_body(json.dumps(data).encode(), "application/json", code)

            def do_GET(self):  # noqa: N802 (http.server's naming)
                path = self.path.split("?", 1)[0]
                if path in ("/", "/index.html"):
                    self.send_body((HERE / "dashboard" / "index.html").read_bytes(),
                                   "text/html; charset=utf-8")
                elif path == "/docs":
                    self.send_body(SWAGGER_PAGE.encode(), "text/html; charset=utf-8")
                elif path == "/api/openapi.json":
                    self.json(OPENAPI)
                elif path == "/api/state":
                    self.json(orch.state())
                elif path == "/api/leaderboard":
                    st = orch.state()
                    self.json({"edition": st["edition"], "leaderboard": st["leaderboard"],
                               "determinism": st["determinism"]})
                elif path == "/api/hosts":
                    self.json(orch.state()["hosts"])
                elif path == "/api/tuning":
                    self.json(orch.tuning_view() or {"enabled": False})
                elif path == "/api/promotions":
                    st = orch.state()
                    self.json({"paired": st["paired"], "promotions": st["promotions"]})
                elif path == "/api/perf":
                    self.json(orch.perf())
                elif path == "/api/postmortems":
                    self.json([{"match": m["id"], "host": m["host"], "seed": m["seed"],
                                "postmortem": m.get("postmortem", [])}
                               for m in sorted(orch.matches.values(), key=lambda m: m["id"],
                                               reverse=True) if m.get("postmortem")])
                elif path == "/api/query":
                    from urllib.parse import parse_qs, urlsplit
                    sql = (parse_qs(urlsplit(self.path).query).get("sql") or [""])[0]
                    res = orch.store.query(sql)
                    self.json(res, 400 if "error" in res else 200)
                elif path.startswith("/api/matches/"):
                    m = orch.matches.get(path.rsplit("/", 1)[1])
                    self.json(m if m else {"error": "no such match"}, 200 if m else 404)
                elif path == "/api/matches":
                    self.json([{k: m.get(k) for k in ("id", "host", "seed", "status", "replica_of",
                                                      "anomalies", "standings")}
                               for m in sorted(orch.matches.values(), key=lambda m: m["id"])])
                elif path == "/api/stream":
                    # Server-sent events: the whole state every stream_every_s, until the client goes.
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Cache-Control", "no-store")
                    self.send_header("Access-Control-Allow-Origin", "*")
                    self.end_headers()
                    try:
                        while not orch.stop.is_set():
                            self.wfile.write(b"event: state\ndata: " + json.dumps(orch.state()).encode()
                                             + b"\n\n")
                            self.wfile.flush()
                            orch.stop.wait(orch.cfg.get("stream_every_s", 2))
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                else:
                    self.json({"error": "not found", "see": "/api/openapi.json"}, 404)

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.event(f"dashboard on http://127.0.0.1:{port}/ (edition {self.digest}, "
                   f"rev {self.provenance['rev'][:10]})")
        signal.signal(signal.SIGTERM, lambda *_: self.stop.set())
        signal.signal(signal.SIGINT, lambda *_: self.stop.set())
        self.reap()
        last = 0.0
        restart = False
        while not self.stop.is_set():
            if time.time() - last > self.cfg.get("schedule_every_s", 30):
                if self.tuning_step():
                    restart = True
                    break
                self.schedule_pass()
                last = time.time()
            for host in self.hosts:
                self.poll_host(host)
            self.stop.wait(self.cfg.get("poll_every_s", 4))
        server.shutdown()
        server.server_close()
        if restart:
            # The next generation is a new edition: start over in a fresh process.
            os.execv(sys.executable, [sys.executable, *sys.argv])
        self.event("stopped; running matches keep going and are re-attached by the next `run`")


# ---------------------------------------------------------------------------- records and docs

def build_record(cfg: dict, orch: Orchestrator) -> dict:
    matches = sorted((m for m in orch.matches.values() if m["status"] in ("done", "incomplete")),
                     key=lambda m: m["id"])
    keys = [c["key"] for c in orch.contestants]
    now = dt.datetime.now(dt.timezone.utc)
    hosts = {}
    for m in matches:
        hosts.setdefault(m["host"], set()).add(m.get("host_version", ""))
    return {
        "schema": RECORD_SCHEMA,
        "scoring": SCORING,
        "id": f"battle-{now.strftime('%Y%m%dT%H%M%S')}-{orch.digest[:8]}",
        "generated_at": now.isoformat(timespec="seconds"),
        "edition": orch.digest,
        "provenance": {k: orch.provenance[k] for k in
                       ("prismatic_path", "referee_path", "rev", "referee_rev", "dirty", "baseset",
                        "ais") if k in orch.provenance},
        "match_cfg": cfg["match"],
        "contestants": [{"key": c["key"], "ai": c["ai"], "settings": c.get("settings", "")}
                        for c in orch.contestants],
        "hosts": [{"name": h, "versions": sorted(v)} for h, v in sorted(hosts.items())],
        "matches": [{"id": m["id"], "host": m["host"], "host_version": m.get("host_version", ""),
                     "seed": m["seed"], "years": m["years"], "ticks": m["ticks"],
                     "lineup": [c["key"] for c in m["lineup"]], "replica_of": m.get("replica_of"),
                     "status": m["status"], "exit_code": m.get("exit_code"),
                     "standings": [{k: f.get(k, []) for k in ("key", "value", "vehicles",
                                                              "stations", "errors", "anomalies")}
                                   for f in m["standings"] or []]}
                    for m in matches],
        "leaderboard": leaderboard(keys, matches),
        "determinism": determinism(matches),
    }


def money(v: int) -> str:
    for div, unit in ((1e9, " B"), (1e6, " M"), (1e3, " k")):
        if abs(v) >= div:
            return f"{v / div:.1f}{unit}"
    return str(v)


# ---------------------------------------------------------------------------- charts
# A chart is a Mermaid pie or xychart-beta block: GitHub and the wiki draw it, and the site reads it
# back and draws it with ECharts (tools/surfaces/README.md "Charts"). `%% series:` names the bar and
# line series in order, `%% links:` gives each bar or slice the row it summarises
# (<page>?col=<column>&filter=<value>, which the site filters and GitHub ignores). The formatting
# mirrors tools/surfaces/surfaces.py mermaid_bars/mermaid_pie, which a generator does not import.

def chart_palette() -> str:
    import tomllib  # Python 3.11+, needed only where a page is rendered
    pal = tomllib.loads(STYLE.read_text())["palette"]
    return f"{pal['signal']}, #8a8a8a, {pal['ink']}, {pal['paper']}"


def mermaid_num(v: float) -> str:
    """Plain digits, never an exponent (Mermaid cannot parse 1.0e+08)."""
    if float(v).is_integer():
        return str(int(v))
    return f"{v:.6f}".rstrip("0").rstrip(".")


def mermaid_label(text) -> str:
    return str(text).replace('"', "'").replace("\n", " ")


def mark_link(page: str, column: str, value) -> str:
    return f"{page}?{urllib.parse.urlencode({'col': column, 'filter': str(value)})}"


def mermaid_xy(title: str, labels: list, series: list[tuple[str, list[float]]], y_title: str,
               links: list[str] | None = None) -> str:
    """A bar per label for the first series and a line for each further one."""
    values = [v for _, vals in series for v in vals]
    lo, top = min(values + [0]), max(values + [1])
    whole = all(float(v).is_integer() for v in values)  # a count's axis ends on a whole number

    def span(v: float) -> str:
        edge = float(f"{v * 1.1:.3g}")
        return mermaid_num((math.ceil(edge) if edge > 0 else math.floor(edge)) if whole else edge) if v else "0"
    lines = ["```mermaid", "---", "config:", "  xyChart:", "    width: 900", "    height: 360",
             "  themeVariables:", "    xyChart:", f'      plotColorPalette: "{chart_palette()}"', "---",
             "xychart-beta", f'    title "{mermaid_label(title)}"',
             "    x-axis [" + ", ".join(f'"{mermaid_label(x)}"' for x in labels) + "]",
             f"    %% series: {' | '.join(mermaid_label(n) for n, _ in series)}"]
    if links:
        lines.append(f"    %% links: {' | '.join(links)}")
    lines.append(f'    y-axis "{mermaid_label(y_title)}" {span(lo)} --> {span(top)}')
    for i, (_, vals) in enumerate(series):
        lines.append(f"    {'bar' if i == 0 else 'line'} [{', '.join(mermaid_num(v) for v in vals)}]")
    return "\n".join(lines + ["```"])


def mermaid_pie_chart(title: str, items: list[tuple[str, float]], links: list[str] | None = None) -> str:
    kept = [(k, v) for k, v in items if v]
    lines = ["```mermaid", f"pie showData title {mermaid_label(title)}"]
    if links:
        lines.append(f"    %% links: {' | '.join(l for (k, v), l in zip(items, links) if v)}")
    lines += [f'    "{mermaid_label(k)}" : {mermaid_num(v)}' for k, v in kept]
    return "\n".join(lines + ["```"])


def home_keys(rec: dict) -> tuple[str, set[str]]:
    """The repository's own AI and its contestant keys: the AI the edition exported from
    provenance.prismatic_path (every variant of it, whatever its label)."""
    home = Path(rec["provenance"]["prismatic_path"]).name
    return home, {c["key"] for c in rec["contestants"] if c["ai"] == home}


def render_charts(rec: dict) -> str:
    """The record's charts, every number taken from its JSON: the leaderboard per contestant
    (points, average final value), each scored match (winner against the best of the repository's
    own AI), match wins by contestant, and the scored matches per arena."""
    page = f"{rec['id']}.md"
    board = [r for r in rec["leaderboard"] if r["played"]]
    scored = [m for m in rec["matches"] if m["status"] == "done" and not m["replica_of"] and m["standings"]]
    home, mine = home_keys(rec)
    who = [r["key"] for r in board]
    per_contestant = [mark_link(page, "Contestant", k) for k in who]
    per_match = rec.get("scoring") == SCORING
    points = [r.get("avg_points", 0) if per_match else r["points"] for r in board]
    out = [CHARTS_BEGIN,
           f"<!-- derived from {rec['id']}.json by `battle.py render`; `battle.py check` fails when it is "
           "stale -->", ""]
    if board:
        what = "Points per match" if per_match else "Points"
        out += [mermaid_xy(f"{what} by contestant ({rec.get('scoring', 'points/v1')}, {len(scored)} scored matches)",
                           who, [(what, points)], what, per_contestant), "",
                mermaid_xy("Average final company value by contestant", who,
                           [("Average final value", [r["avg_value"] for r in board])], "Company value",
                           per_contestant), ""]
    if scored:
        best = [max((f["value"] for f in m["standings"] if f["key"] in mine), default=0) for m in scored]
        out += [mermaid_xy(f"Final company value per scored match: winner vs best {home}",
                           [m["id"] for m in scored],
                           [("Winner", [m["standings"][0]["value"] for m in scored]), (f"Best {home}", best)],
                           "Company value", [mark_link(page, "Match", m["id"]) for m in scored]), ""]
        wins: dict[str, int] = {}
        for m in scored:
            wins[m["standings"][0]["key"]] = wins.get(m["standings"][0]["key"], 0) + 1
        ranked = sorted(wins.items(), key=lambda kv: (-kv[1], kv[0]))
        out += [mermaid_pie_chart(f"Match wins by contestant ({len(scored)} scored matches)", ranked,
                                  [mark_link(page, "Winner", k) for k, _ in ranked]), ""]
        arenas = sorted({m["host"] for m in scored})
        out += [mermaid_xy(f"Scored matches per arena, and how many {home} won", arenas,
                           [("Scored matches", [sum(m["host"] == h for m in scored) for h in arenas]),
                            (f"Won by {home}", [sum(m["host"] == h and m["standings"][0]["key"] in mine
                                                    for m in scored) for h in arenas])],
                           "Matches", [mark_link(page, "Arena", h) for h in arenas]), ""]
    if not board and not scored:
        out += ["No scored match: nothing to chart.", ""]
    return "\n".join(out + [CHARTS_END])


def code_at(p: dict) -> str:
    """Which commit PrismaticAI and the referee came from (one commit unless the referee followed
    the repository on its own: prepare --referee-only, run)."""
    if p.get("referee_rev", p["rev"]) == p["rev"]:
        return f"PrismaticAI and referee at `{p['rev']}`"
    return f"PrismaticAI at `{p['rev']}`, referee at `{p['referee_rev']}`"


def render_md(rec: dict, charts: bool = True) -> str:
    p, mc = rec["provenance"], rec["match_cfg"]
    done = [m for m in rec["matches"] if m["status"] == "done" and not m["replica_of"]]
    det = rec["determinism"]
    same = sum(d["identical"] for d in det)
    lines = [
        f"# {rec['id']}", "",
        f"Derived from `{rec['id']}.json` by `tools/research/ai-battle/battle.py report`; "
        "do not edit.", "",
        f"- Edition `{rec['edition']}`: {code_at(p)}"
        f"{' (DIRTY)' if p['dirty'] else ''}, base set `{p['baseset']}`",
        f"- {mc['years']}-year free-for-all from {mc['start_year']}, map "
        f"{2 ** mc['map_x']}x{2 ** mc['map_y']} {mc['landscape']}, breakdowns "
        f"{mc.get('breakdowns', 1)}",
        f"- {len(done)} scored match(es) on "
        + ", ".join(f"`{h['name']}` ({', '.join(h['versions'])})" for h in rec["hosts"]),
        f"- Replays: {same} of {len(det)} identical to their original on another arena", "",
    ]
    v2 = rec.get("scoring") == SCORING
    if v2:
        lines += ["| # | Contestant | Played | Wins | Points | Points/match | Avg rank | Avg final value "
                  "| Squirrel errors | Matches with anomalies |",
                  "|---|---|---|---|---|---|---|---|---|---|"]
    else:
        lines += ["| # | Contestant | Played | Wins | Points | Avg rank | Avg final value | Squirrel errors "
                  "| Matches with anomalies |", "|---|---|---|---|---|---|---|---|---|"]
    for i, r in enumerate(rec["leaderboard"], 1):
        lines.append(f"| {i} | {r['key']} | {r['played']} | {r['wins']} | {r['points']} | "
                     + (f"{r.get('avg_points', '-')} | " if v2 else "")
                     + f"{r['avg_rank'] if r['avg_rank'] is not None else '-'} | "
                     f"{money(r['avg_value'])} | {r['errors']} | {r.get('anomalies', 0)} |")
    if charts:
        lines += ["", render_charts(rec)]
    if det:
        lines += ["", "| Seed | Original | Replay | Arenas | Identical |", "|---|---|---|---|---|"]
        for d in det:
            lines.append(f"| {d['seed']} | {d['original']} | {d['replay']} | "
                         f"{' / '.join(d['hosts'])} | {'yes' if d['identical'] else 'NO'} |")
    lines += ["", "| Match | Arena | Seed | Winner | Value | Last | Anomalies |",
              "|---|---|---|---|---|---|---|"]
    for m in rec["matches"]:
        s = m["standings"]
        flags = "; ".join(f"{f['key']}: {', '.join(f['anomalies'])}" for f in s if f.get("anomalies"))
        lines.append(f"| {m['id']}{' (replay)' if m['replica_of'] else ''} | {m['host']} | "
                     f"{m['seed']} | {s[0]['key'] if s else '-'} | "
                     f"{money(s[0]['value']) if s else '-'} | {s[-1]['key'] if s else '-'} | "
                     f"{flags or '-'} |")
    return "\n".join(lines) + "\n"


def render_region(rec: dict | None) -> str:
    if rec is None:
        return f"{BEGIN}\nNo battle record yet: run `battle.py report`.\n{END}"
    det = rec["determinism"]
    # The README carries the tables only: the charts link the record's own rows, from its page.
    body = render_md(rec, charts=False).split("\n| Match |")[0].split("\n", 4)[4].rstrip()
    return (f"{BEGIN}\n<!-- derived from research/results/{rec['id']}.json by "
            f"`battle.py report`; `battle.py check` fails when it is stale -->\n"
            f"Latest record: [`{rec['id']}`](../results/{rec['id']}.md) "
            f"({sum(d['identical'] for d in det)}/{len(det)} replays identical).\n\n"
            f"{body}\n{END}")


def tracked_records(pattern: str = RECORD_GLOB, results: Path | None = None,
                    root: Path | None = None) -> list[Path]:
    """The records the next commit can hold, counted like tools/research/readme-scoreboard
    records(): what git tracks plus new files it would not ignore, so a rewrite and `check` agree
    before and after a `git add`; inside a git hook (git exports GIT_INDEX_FILE to pre-commit) only
    the index, because a record written but never added would pass the hook, become the latest in
    the README region and break CI, whose checkout does not have it. Outside a git checkout of the
    repository every record on disk counts."""
    results, root = results or RESULTS_DIR, root or REPO
    env = dict(os.environ)
    if env.get("GIT_INDEX_FILE"):  # pre-commit may export it relative to the hook's directory
        env["GIT_INDEX_FILE"] = os.path.abspath(env["GIT_INDEX_FILE"])
    sources = ["--cached"]
    if not env.get("GIT_INDEX_FILE"):
        sources += ["--others", "--exclude-standard"]

    def run_git(*args: str) -> str:
        return subprocess.run(["git", *args], cwd=results, env=env, check=True,
                              capture_output=True, text=True).stdout
    try:
        if Path(run_git("rev-parse", "--show-toplevel").strip()).resolve() != root.resolve():
            raise ValueError("not a checkout of the repository")
        listed = run_git("ls-files", "-z", *sources, "--", ".").split("\0")
    except (OSError, ValueError, subprocess.CalledProcessError):
        return sorted(results.glob(pattern))
    return sorted(results / f for f in listed
                  if f and "/" not in f and Path(f).match(pattern) and (results / f).exists())


def latest_record() -> dict | None:
    files = tracked_records()
    return json.loads(files[-1].read_text()) if files else None


def replace_region(text: str, region: str) -> str:
    a, b = text.index(BEGIN), text.index(END) + len(END)
    return text[:a] + region + text[b:]


REQUIRED = {"schema": str, "id": str, "generated_at": str, "edition": str, "provenance": dict,
            "match_cfg": dict, "contestants": list, "hosts": list, "matches": list,
            "leaderboard": list, "determinism": list}
REQUIRED_MATCH = ("id", "host", "host_version", "seed", "years", "ticks", "lineup", "status",
                  "standings")


def validate_record(rec: dict, name: str) -> list[str]:
    """What makes a record reproducible: every match names its arena build, seed, start order and
    length, and the edition names a clean commit for PrismaticAI and the referee."""
    errs = [f"{name}: '{k}' missing or not {t.__name__}" for k, t in REQUIRED.items()
            if not isinstance(rec.get(k), t)]
    if errs:
        return errs
    if rec["schema"] != RECORD_SCHEMA:
        errs.append(f"{name}: schema is {rec['schema']!r}, expected {RECORD_SCHEMA!r}")
    if name != f"{rec['id']}.json":
        errs.append(f"{name}: file name does not match id {rec['id']!r}")
    p = rec["provenance"]
    if p.get("dirty") is not False:
        errs.append(f"{name}: edition built from uncommitted PrismaticAI/referee (prepare --allow-dirty)")
    for k in ("rev", "referee_rev") if "referee_rev" in p else ("rev",):
        if not re.fullmatch(r"[0-9a-f]{40}", str(p.get(k, ""))):
            errs.append(f"{name}: provenance.{k} is not a full commit hash")
    keys = {c["key"] for c in rec["contestants"]}
    for m in rec["matches"]:
        missing = [k for k in REQUIRED_MATCH if k not in m]
        if missing:
            errs.append(f"{name}: match {m.get('id')} lacks {missing}")
            continue
        if not m["host_version"]:
            errs.append(f"{name}: match {m['id']} does not name its arena build")
        if set(m["lineup"]) - keys:
            errs.append(f"{name}: match {m['id']} lineup names an unknown contestant")
        if m["ticks"] != match_ticks(m["years"]):
            errs.append(f"{name}: match {m['id']} ticks do not match its length")
    if rec["leaderboard"] != leaderboard([c["key"] for c in rec["contestants"]],
                                         [dict(m, standings=m["standings"]) for m in rec["matches"]],
                                         rec.get("scoring", "points/v1")):
        errs.append(f"{name}: leaderboard is not derived from its matches")
    return errs


def cmd_report(cfg: dict, check_only: bool, edition: str | None = None) -> int:
    if check_only:
        return cmd_check()
    if edition and json.loads((cfg["cache"] / "arena" / "PROVENANCE.json").read_text())["rev"] \
            and not (cfg["results"] / edition / "matches").is_dir():
        log(f"no matches of edition {edition}")
        return 12
    orch = Orchestrator(cfg, edition)
    rec = build_record(cfg, orch)
    if not [m for m in rec["matches"] if m["status"] == "done"]:
        log("no finished match in this edition yet; nothing written")
        return 12
    errs = validate_record(rec, f"{rec['id']}.json")
    if errs:
        print("\n".join(errs), file=sys.stderr)
        return 10
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    (RESULTS_DIR / f"{rec['id']}.json").write_text(json.dumps(rec, indent=1) + "\n")
    (RESULTS_DIR / f"{rec['id']}.md").write_text(render_md(rec))
    README.write_text(replace_region(README.read_text(), render_region(rec)))
    log(f"wrote research/results/{rec['id']}.{{json,md}} and the README leaderboard")
    return 0


# ---------------------------------------------------------------------------- ablation records
# `ablate --write` records research/results/ablation-<utc>-seed<S>.{json,md}: one local match per
# variant of PrismaticAI on one seed. The page (tables and charts) is derived from the JSON alone by
# render_ablation_md; `render` rewrites every tracked page and `check` fails while one is stale.
ABLATION_SCHEMA = "ai-battle-ablation/v1"
ABLATION_GLOB = "ablation-*.json"
ABLATION_CHARTS_BEGIN, ABLATION_CHARTS_END = "<!-- ablation-charts:begin -->", "<!-- ablation-charts:end -->"
ABLATION_REQUIRED = {"schema": str, "id": str, "generated_at": str, "seed": int, "years": int,
                     "field": list, "match_cfg": dict, "rev": str, "tree_dirty": bool, "rows": list}
ABLATION_ROW = ("variant", "settings", "exit_code", "value", "money", "vehicles", "rail", "road", "air",
                "errors", "field_best", "t", "vs_first")
# Optional per row (records from 2026-11-23 on): `standings`, every company of the variant's match in
# company-id order, keyed as its lineup names it, with its final value (and fleet, profit and errors),
# so a reader derives rank and strongest rival.
# Optional per record: `imported`, the provenance of a record built after the run from its kept log.


def ablation_vs_first(rows: list[dict]) -> list[float]:
    """Each variant's final value against the first (the baseline), as cmd_ablate records it."""
    base = rows[0]["value"] or 1
    return [round(r["value"] / base - 1, 4) for r in rows]


MODE_KEYS = ("rail", "road", "air", "water", "prail", "proad", "pair", "pwater")


def ablation_row(name: str, settings: str, exit_code: int | None, text: str, field: list[str],
                 slot: int) -> dict:
    """One variant's ablation row from its match log: its last quarterly referee sample, the best
    opponent's final value, and `standings`, every company of the lineup in company-id order (the
    opponents keyed by AI, the variant by its name, at `slot`) with its final value, fleet per mode,
    last year's profit per mode and Squirrel errors. `ablate_rows` builds every row with it, and
    `tools/research/strategy-surfaces import` builds one from a kept log.
    `exit_code` is None when the run's exit status was not kept (a record imported from a log)."""
    series, errors = parse_log(text)
    me = min(slot, len(field))
    lineup = list(field)
    lineup.insert(me, name)
    mine = series.get(me, [])
    last = mine[-1] if mine else {}
    field_best = max((s[-1]["value"] for c, s in series.items() if c != me and s), default=0)
    standings = []
    for cid, key in enumerate(lineup):
        f = (series.get(cid) or [{}])[-1]
        standings.append({"company": cid, "key": key, "value": f.get("value", 0),
                          "vehicles": f.get("vehicles", 0), "errors": errors.get(cid, 0),
                          **{k: f.get(k, 0) for k in MODE_KEYS}})
    return {"variant": name, "settings": settings, "exit_code": exit_code,
            "value": last.get("value", 0), "money": last.get("money", 0),
            "vehicles": last.get("vehicles", 0), "rail": last.get("rail"), "road": last.get("road"),
            "air": last.get("air"), "errors": errors.get(me, 0), "field_best": field_best,
            "t": last.get("t", ""), "standings": standings}


def standings_errors(rec: dict, r: dict, name: str) -> list[str]:
    """What a row's optional `standings` must hold: one entry per company, each key named once
    (the variant and every AI of the field), integer values, and the row's own value and best
    opponent agreeing with them."""
    st, where = r["standings"], f"{name}: row {r['variant']} standings"
    if not isinstance(st, list) or not all(isinstance(e, dict) for e in st):
        return [f"{where}: not a list of companies"]
    if any(not isinstance(e.get("key"), str) or type(e.get("value")) is not int for e in st):
        return [f"{where}: an entry lacks a string key or an integer value"]
    keys = [e["key"] for e in st]
    if len(set(keys)) != len(keys):
        return [f"{where}: a company is named twice"]
    want = set(rec["field"]) | {r["variant"]}
    if set(keys) != want:
        return [f"{where}: names {sorted(set(keys))}, the match held {sorted(want)}"]
    value = {e["key"]: e["value"] for e in st}
    errs = []
    if value[r["variant"]] != r["value"]:
        errs.append(f"{where}: the variant's value {value[r['variant']]} is not the row's {r['value']}")
    best = max((v for k, v in value.items() if k != r["variant"]), default=0)
    if best != r["field_best"]:
        errs.append(f"{where}: the best opponent {best} is not the row's field_best {r['field_best']}")
    return errs


def validate_ablation(rec: dict, name: str) -> list[str]:
    """What an ablation page is derived from: the schema, the id its file is named by, and a row per
    variant (each named once, as the charts link it) whose ratio to the baseline is its own."""
    errs = [f"{name}: '{k}' missing or not {t.__name__}" for k, t in ABLATION_REQUIRED.items()
            if not isinstance(rec.get(k), t)]
    if errs:
        return errs
    if rec["schema"] != ABLATION_SCHEMA:
        errs.append(f"{name}: schema is {rec['schema']!r}, expected {ABLATION_SCHEMA!r}")
    if name != f"{rec['id']}.json":
        errs.append(f"{name}: file name does not match id {rec['id']!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", rec["rev"]):
        errs.append(f"{name}: rev is not a full commit hash")
    rows = rec["rows"]
    if not rows:
        return errs + [f"{name}: no variant row"]
    for r in rows:
        missing = [k for k in ABLATION_ROW if k not in r]
        if missing:
            errs.append(f"{name}: row {r.get('variant')} lacks {missing}")
    if errs:
        return errs
    names = [r["variant"] for r in rows]
    if len(set(names)) != len(names):
        errs.append(f"{name}: a variant is named twice")
    if [r["vs_first"] for r in rows] != ablation_vs_first(rows):
        errs.append(f"{name}: vs_first is not derived from the values")
    for r in rows:
        if "standings" in r:
            errs += standings_errors(rec, r, name)
    return errs


def render_ablation_charts(rec: dict) -> str:
    """The ablation's charts, every number taken from its JSON: each variant's final value with
    the best opponent in its match, its change against the baseline (the first variant), its share
    of that best opponent, and its rail, road and air fleet at the end. Each mark opens the
    variant's row."""
    page, rows = f"{rec['id']}.md", rec["rows"]
    names = [r["variant"] for r in rows]
    links = [mark_link(page, "Variant", n) for n in names]
    base = rows[0]
    change = [round(r["vs_first"] * 100, 1) for r in rows]
    share = [round(100 * r["value"] / r["field_best"], 1) if r["field_best"] else 0 for r in rows]
    lo = min(rows, key=lambda r: r["value"])
    hi = max(rows, key=lambda r: r["value"])
    spread = hi["value"] / lo["value"] - 1 if lo["value"] else 0
    # Three series at most: the palette's fourth colour is the paper the chart is drawn on.
    fleet = [(k.capitalize(), [r[k] for r in rows]) for k in ("rail", "road", "air")
             if all(isinstance(r[k], int) for r in rows)] or [("Vehicles", [r["vehicles"] for r in rows])]
    out = [ABLATION_CHARTS_BEGIN,
           f"<!-- derived from {rec['id']}.json by `battle.py render`; `battle.py check` fails when it is "
           "stale -->", "",
           f"Highest final value: {hi['variant']} ({hi['vs_first']:+.1%} against {base['variant']}); "
           f"lowest: {lo['variant']} ({lo['vs_first']:+.1%}). Spread between them: {spread:.1%} of the "
           f"lowest, on one seed ({rec['seed']}).", "",
           mermaid_xy(f"Final company value by variant, and the best opponent in its match (seed {rec['seed']}, "
                      f"{rec['years']} years)", names,
                      [("Final value", [r["value"] for r in rows]),
                       ("Best opponent", [r["field_best"] for r in rows])], "Company value", links), "",
           mermaid_xy(f"Final company value against the baseline {base['variant']} (%)", names,
                      [("Change against the baseline (%)", change)], "Change (%)", links), "",
           mermaid_xy("Final company value as a share of the best opponent (%)", names,
                      [("Share of the best opponent (%)", share)], "Share (%)", links), "",
           mermaid_xy(f"Fleet at the end by variant: {', '.join(n.lower() for n, _ in fleet)} vehicles",
                      names, fleet, "Vehicles", links), "",
           ABLATION_CHARTS_END]
    return "\n".join(out)


def imported_line(imp: dict) -> str:
    """The provenance sentence of a record built after the run from its kept log."""
    binary = f", binary `{imp['binary']}`" if imp.get("binary") else ""
    return (f"Imported after the run from its kept log `{imp['log']}` (sha256 `{imp['log_sha256'][:16]}`)"
            f"{binary}, by `{imp['by']}`; the run's exit status was not kept. {imp.get('note', '')}").rstrip()


def render_ablation_md(rec: dict, charts: bool = True) -> str:
    mc, rows = rec["match_cfg"], rec["rows"]
    md = [f"# {rec['id']}", "",
          "Derived from its .json by `tools/research/ai-battle/battle.py ablate --write`; do not edit.", "",
          f"One local match per variant of PrismaticAI against {', '.join(rec['field'])}, seed {rec['seed']}, "
          f"{rec['years']} years from {mc['start_year']}, at `{rec['rev'][:10]}`"
          f"{' (tree dirty)' if rec['tree_dirty'] else ''}. "
          "Each value is the variant's last quarterly referee sample. One seed: a direction, not a verdict.", "",
          *([imported_line(rec["imported"]), ""] if rec.get("imported") else []),
          "| Variant | Value | vs first | Vehicles | Rail | Road | Air | Squirrel errors | Settings |",
          "|---|---|---|---|---|---|---|---|---|"]
    md += [f"| {r['variant']} | {r['value']:,} | {r['vs_first']:+.1%} | {r['vehicles']} | {r['rail']} | "
           f"{r['road']} | {r['air']} | {r['errors']} | `{r['settings']}` |" for r in rows]
    if charts:
        md += ["", render_ablation_charts(rec)]
    return "\n".join(md) + "\n"


def cmd_render() -> int:
    """Rewrite every tracked record's .md (battle and ablation) and the README leaderboard from the
    records' JSON. Pure: it reads only the records (tracked_records) and the chart palette, so
    `surfaces sync` may run it, and a change of render_md or render_ablation_md reaches every page
    in the commit that makes it."""
    files, ablations, written = tracked_records(), tracked_records(ABLATION_GLOB), 0
    for f, render in [(f, render_md) for f in files] + [(f, render_ablation_md) for f in ablations]:
        md, text = f.with_suffix(".md"), render(json.loads(f.read_text()))
        if not md.is_file() or md.read_text() != text:
            md.write_text(text)
            written += 1
    readme = README.read_text()
    if BEGIN not in readme or END not in readme:
        print(f"{README.relative_to(REPO)}: battle region markers missing", file=sys.stderr)
        return 10
    region = replace_region(readme, render_region(latest_record()))
    if region != readme:
        README.write_text(region)
    # The next live start's settings follow every new battle or live record (live-settings).
    if tracked_records(LIVE_GLOB):
        live = RESULTS_DIR / LIVE_SETTINGS
        text = live_settings_text(live_tuning(json.loads(DEFAULT_CONFIG.read_text())))
        if not live.is_file() or live.read_text() != text:
            live.write_text(text)
            written += 1
    print(f"battle render: {written} of {len(files) + len(ablations)} record page(s) "
          f"({len(files)} battle, {len(ablations)} ablation) rewritten, README leaderboard "
          + ("rewritten" if region != readme else "current"))
    return 0


def cmd_check() -> int:
    errs, drift = [], []
    files = tracked_records()
    for f in files:
        rec = json.loads(f.read_text())
        errs += validate_record(rec, f.name)
        md = f.with_suffix(".md")
        if not md.is_file() or md.read_text() != render_md(rec):
            drift.append(f"{md.name}: not derived from {f.name} (run battle.py render)")
    ablations = tracked_records(ABLATION_GLOB)
    for f in ablations:
        rec = json.loads(f.read_text())
        found = validate_ablation(rec, f.name)
        errs += found
        md = f.with_suffix(".md")
        if not found and (not md.is_file() or md.read_text() != render_ablation_md(rec)):
            drift.append(f"{md.name}: not derived from {f.name} (run battle.py render)")
    latest = latest_record()
    cfg_raw = json.loads(DEFAULT_CONFIG.read_text())
    errs += config_errors(cfg_raw)  # every GA gene is a setting of info.nut, within its bounds
    lives = tracked_records(LIVE_GLOB)
    for f in lives:
        errs += validate_live(json.loads(f.read_text()), f.name)
    live = RESULTS_DIR / LIVE_SETTINGS
    if lives and not errs and (not live.is_file() or live.read_text() != live_settings_text(live_tuning(cfg_raw))):
        drift.append(f"{LIVE_SETTINGS}: not derived from the battle and live records (run battle.py render)")
    cfg_never = cfg_raw.get("never_last", [])
    if latest and latest["leaderboard"]:
        board = [r for r in latest["leaderboard"] if r["played"]]
        if board and board[-1]["key"] in cfg_never:
            errs.append(f"{latest['id']}: {board[-1]['key']} is last on the leaderboard "
                        "(battle.json never_last; rule project.prismatic-ai-passes-the-headless-fitness-check)")
    text = README.read_text()
    if BEGIN not in text or END not in text:
        errs.append(f"{README.relative_to(REPO)}: battle region markers missing")
    elif replace_region(text, render_region(latest_record())) != text:
        drift.append(f"{README.relative_to(REPO)}: leaderboard is not derived from the latest record")
    for line in errs + drift:
        print(line, file=sys.stderr)
    if errs:
        return 10
    if drift:
        return 11
    n = len(files)
    print(f"battle check: {n} record(s), {len(ablations)} ablation and {len(lives)} live record(s) valid, "
          "live-start.json, every page and "
          "the README leaderboard derived, "
          f"{len(gene_space(cfg_raw.get('tuning', {}), info_settings(info_nut_path(cfg_raw))))} "
          "GA gene(s) derived from info.nut")
    return 0


def fleet_peak_final(series: list[dict]) -> tuple[int, int]:
    """(peak, final) vehicle count over a contestant's quarterly samples."""
    counts = [s["vehicles"] for s in series] or [0]
    return max(counts), counts[-1]


def hoarding(series: list[dict], rules: dict) -> str | None:
    """The first window of `hoard_window` quarterly samples (default 8, two game years) in which the
    fleet stays flat or shrinks (its maximum is at most `hoard_fleet_slack` above the count at the
    window's start), and so do its stations (nothing is built), while its money net of the loan
    (the bank balance less the loan, both from the referee) rises at least `hoard_factor`-fold
    (default 2) and ends at or above `hoard_floor` (default 500000), as
    "<t0>..<t1>:<money0>-><money1>@<vehicles>v" with the net money. In the owner's live co-play
    game (2026-11-22) company 0 kept 35-37 vehicles from 1955 to 1960 while its balance grew from
    0.6M to 3.77M, and no other anomaly fired. Every window is judged, not only the last, so the
    fitness check sees a hoard the bot later spent its way out of. The bank balance alone counted
    borrowed money as hoarded: on the fitness seed 930967890 the self-steward contestant's last
    sample (1966-01) came while a new rail line was being built on the whole loan, 348k in 1965-10
    and 795k then, and the check read 321k -> 795k as a hoard. A sample without a loan (a log or
    record from before the referee logged it) counts the loan as 0. A window that starts with net
    money at or below 0 is skipped: a factor of a non-positive start says nothing, and such a
    company is paying back its loan. Judging net money means that, for a company holding the same
    loan L throughout, the factor test reads bank_1 >= factor * bank_0 - (factor - 1) * L in bank
    terms (looser) and the floor reads bank_1 >= floor + L (stricter). That is intended: the test
    judges what the company owns."""
    window = rules.get("hoard_window", 8)
    factor, floor = rules.get("hoard_factor", 2.0), rules.get("hoard_floor", 500_000)
    slack = rules.get("hoard_fleet_slack", 0.1)
    for i in range(len(series) - window + 1):
        w = series[i:i + window]
        v0 = w[0]["vehicles"]
        m0 = w[0].get("money", 0) - w[0].get("loan", 0)
        m1 = w[-1].get("money", 0) - w[-1].get("loan", 0)
        # A window that starts in debt (net money at most 0) is not judged. Any rise is then at
        # least "factor-fold", so only the floor would decide, and a company paying back its loan
        # (100k on a 300k loan, then 520k with none) would read as a hoard. A company that goes on
        # holding its money is caught by the next window that starts above 0.
        if m0 <= 0:
            continue
        # Building is spending: a company whose stations grow while its fleet waits is laying
        # lines (the planned fitness contestant, 9 vehicles, 283k -> 697k, one rail line a
        # heartbeat), not hoarding. Samples without a station count judge the fleet alone.
        st0 = w[0].get("stations")
        builds = st0 is not None and max(s.get("stations", st0) for s in w) > st0 * (1 + slack)
        if (max(s["vehicles"] for s in w) <= v0 * (1 + slack) and not builds and m1 >= floor
                and m1 >= factor * m0):
            return f"{w[0]['t']}..{w[-1]['t']}:{m0}->{m1}@{v0}v"
    return None


def anomalies(series: list[dict], errors: int, rules: dict) -> list[str]:
    """What looks wrong with one contestant in one match. The orchestrator attaches these to every
    match and the fitness check fails on them. `rules`: fleet_min_peak, fleet_keep (fraction), and
    the hoard_* thresholds of `hoarding`."""
    out = []
    if errors:
        out.append(f"squirrel_errors:{errors}")
    peak, final = fleet_peak_final(series)
    if peak >= rules.get("fleet_min_peak", 10) and final < rules.get("fleet_keep", 0.7) * peak:
        out.append(f"fleet_drop:{peak}->{final}")
    if series and len(series) >= 8 and series[-1]["vehicles"] == 0:
        out.append("no_vehicles")
    hoard = hoarding(series, rules)
    if hoard:
        out.append(f"hoards:{hoard}")
    return out


def match_regimes(m: dict, key: str, fit: dict) -> list[dict]:
    """The failure regimes of contestant `key` in one match, against the contestants that are not
    PrismaticAI (a sibling is not a rival: two copies are judged against the same field)."""
    rivals = [c["key"] for c in m["lineup"] if not c["ai"].startswith("PrismaticAI")]
    return regimes.detect(m["series"], key, [k for k in rivals if k != key], fit.get("regimes"))


def fitness_verdict(m: dict, fit: dict) -> list[str]:
    """The fitness check's failures for one finished local match (empty: PASS)."""
    fails = []
    if m["status"] != "done":
        fails.append(f"match {m['status']} (exit {m.get('exit_code')}): the bot or the referee broke")
    for c in m["lineup"]:
        if not c["ai"].startswith("PrismaticAI"):
            continue
        bad = anomalies(m["series"].get(c["key"], []), m["errors"].get(c["key"], 0), fit["rules"])
        # Every PrismaticAI must not crash; only the ones named healthy must also keep their fleet
        # and spend their money (fleet_drop, no_vehicles, hoards).
        if c["key"] not in fit.get("must_be_healthy", []):
            bad = [b for b in bad if b.startswith("squirrel")]
        # Hoarding fails only the contestants held to spend (must_not_hoard, default: every healthy
        # one). The self-steward lineup was left out while a pre-merge tree of the rich growth
        # hoarded on the fitness seed; the merged bot does not (research/ai-battle/README.md).
        elif c["key"] not in fit.get("must_not_hoard", fit.get("must_be_healthy", [])):
            bad = [b for b in bad if not b.startswith("hoards")]
        fails += [f"{c['key']}: {b}" for b in bad]
    fails += sibling_producers(m)
    fails += trainless_stations(m, fit["rules"])
    # Failure regimes (research/failure-regimes.md): a contestant held healthy must not stop
    # expanding while its rivals grow, nor fall behind the strongest rival faster than the field.
    for c in m["lineup"]:
        if c["ai"].startswith("PrismaticAI") and c["key"] in fit.get("must_be_healthy", []):
            fails += regimes.failures(match_regimes(m, c["key"], fit), c["key"], fit.get("regimes"))
    fails += funnel_gaps(m, fit.get("must_funnel", []))
    fails += frozen_construction(m, fit)
    # A contestant held to run trains ends with at least one: the test_trainless lineup refuses every
    # new line's first train, so a train at the end proves KeepTrainlessLine, the adoption and
    # KeepLinesServed's first train (research/incident-2026-11-23-trainless-lines.md).
    for c in m["lineup"]:
        s = m["series"].get(c["key"], [])
        if c["key"] in fit.get("must_run_trains", []) and (s[-1].get("rail", 0) if s else 0) < 1:
            fails.append(f"{c['key']}: no_trains: no train at the end of the match")
    # The owner's order (2026-11-19): PrismaticAI is never last. By final value on the fitness seed.
    finals = sorted(((s[-1]["value"] if s else 0), k) for k, s in
                    ((c["key"], m["series"].get(c["key"], [])) for c in m["lineup"]))
    if finals and finals[0][1] in fit.get("never_last", []):
        fails.append(f"{finals[0][1]}: last of {len(finals)} (value {finals[0][0]:,})")
    return fails


def cmd_fitness(cfg: dict) -> int:
    """A short local match of the working tree's PrismaticAI (not an exported commit: this is the
    check that runs before a commit) against a reference AI, on a fixed seed. Deterministic: the null
    driver runs on ticks, so the same tree and binary give the same verdict."""
    fit = cfg["fitness"]
    arena = cfg["cache"] / "arena"
    binary = REPO / "build" / "openttd"
    ref_tars = [t for t in (arena / "ai").glob("*.tar") if t.name != "PrismaticAI.tar"] if (
        arena / "ai").is_dir() else []
    if not binary.is_file() or not (arena / "baseset").is_dir() or not ref_tars:
        print(f"fitness: NOT RUN: needs {binary.relative_to(REPO)} and an arena cache with a base set "
              f"and reference AIs at {arena} (battle.py fetch && battle.py prepare)")
        return 0
    import tempfile
    with tempfile.TemporaryDirectory(prefix="battle-fitness-") as tmp:
        home = Path(tmp) / "home"
        personal = home / "Documents" / "OpenTTD"
        (personal / "scripts").mkdir(parents=True)
        (personal / "ai").mkdir()
        (personal / "game").mkdir()
        os.symlink(arena / "baseset", personal / "baseset")
        (home / ".local" / "share").mkdir(parents=True)
        os.symlink(personal, home / ".local" / "share" / "openttd")   # Linux (XDG)
        subprocess.run(["cp", "-R", str(REPO / cfg["prismatic"]["path"]), str(personal / "ai")],
                       check=True)
        subprocess.run(["cp", "-R", str(arena / "ai" / "library"), *map(str, ref_tars),
                        str(personal / "ai")], check=True)
        subprocess.run(["cp", "-R", str(REPO / cfg["referee"]["path"]), str(personal / "game")],
                       check=True)
        mc = dict(cfg["match"], years=fit["years"])
        (Path(tmp) / "battle.cfg").write_text(battle_cfg_text(mc))
        t0 = time.time()
        fails: list[str] = []
        # One match per lineup, run in parallel (one single-threaded OpenTTD process each).
        lineups = [[dict(c, key=contestant_key(c)) for c in lu] for lu in fit["lineups"]]
        procs = []
        for i, lineup in enumerate(lineups):
            mhome = Path(tmp) / f"m{i}"
            mper = mhome / "Documents" / "OpenTTD"
            (mper / "scripts").mkdir(parents=True)
            for d in ("baseset", "ai", "game"):
                os.symlink(personal / d, mper / d)
            (mhome / ".local" / "share").mkdir(parents=True)
            os.symlink(mper, mhome / ".local" / "share" / "openttd")
            (mper / "scripts" / "game_start.scr").write_text(start_script_text(lineup))
            procs.append(subprocess.Popen(
                [str(binary), "-c", "battle.cfg", "-x", "-v", f"null:ticks={match_ticks(fit['years'])}",
                 "-s", "null", "-m", "null", "-g", "-G", str(fit["seed"]), "-d", "script=3"],
                cwd=tmp, env=dict(os.environ, HOME=str(mhome)), stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, text=True))
        for i, (lineup, proc) in enumerate(zip(lineups, procs)):
            out, _ = proc.communicate(timeout=fit.get("timeout_s", 600))
            series, errors = parse_log(out)
            m = {"lineup": lineup, "years": fit["years"], "match_cfg": mc,
                 "exit_code": proc.returncode,
                 "series": {lineup[c]["key"]: s for c, s in series.items() if c < len(lineup)},
                 "errors": {lineup[c]["key"]: n for c, n in errors.items() if c < len(lineup)}}
            m["status"] = match_status(m, proc.returncode)
            m["shared"] = parse_shared(out)
            m["trainless"] = parse_trainless(out)
            m["funnel"] = {lineup[c]["key"]: r for c, r in parse_funnel(out).items() if c < len(lineup)}
            m["capital"] = parse_capital(out)
            for c in lineup:
                s = m["series"].get(c["key"], [])
                peak, final = fleet_peak_final(s)
                print(f"fitness[{i}]: {c['key']:26} value {s[-1]['value'] if s else 0:>12,}  vehicles "
                      f"peak {peak:>4} final {final:>4}  "
                      f"{' '.join(anomalies(s, m['errors'].get(c['key'], 0), fit['rules'])) or 'ok'}")
                for r in match_regimes(m, c["key"], fit):
                    print(f"fitness[{i}]: {c['key']:26} regime {regimes.describe(r)}")
            for cid, reps in sorted(m["capital"].items()):
                if cid < len(lineup):
                    early = [r for r in reps if r["year"] < mc["start_year"] + fit["rules"].get("frozen_years", 3)]
                    print(f"fitness[{i}]: {lineup[cid]['key']:26} construction frozen "
                          f"{sum(r.get('frozen', 0) for r in early)} of {sum(r.get('days', 0) for r in early)} days "
                          f"in its first {fit['rules'].get('frozen_years', 3)} years")
            for ind, e in sorted(m["shared"].items()):
                owners = ", ".join(lineup[o]["key"] if o < len(lineup) else f"company {o}"
                                   for o in sorted(e["owners"]))
                print(f"fitness[{i}]: shared producer {e['name']} (industry {ind}) from {e['first']}: {owners}")
            for cid, reps in sorted(m["trainless"].items()):
                if cid < len(lineup) and reps and lineup[cid]["ai"].startswith("PrismaticAI"):
                    print(f"fitness[{i}]: {lineup[cid]['key']:26} rail stations {reps[-1]['rail']:>3}, "
                          f"without a train {reps[-1]['trainless']:>3} ({reps[-1]['date']})")
            for key, rows in m["funnel"].items():
                for r in rows:
                    print(f"fitness[{i}]: {key:26} funnel {funnel_text(r)}")
            fails += [f"[{i}] {f}" for f in fitness_verdict(m, fit)]
    for f in fails:
        print(f"fitness: FAIL {f}", file=sys.stderr)
    print(f"fitness: {'FAIL' if fails else 'PASS'} ({len(lineups)} match(es), {fit['years']} years, "
          f"seed {fit['seed']}, {time.time() - t0:.0f} s)")
    return 10 if fails else 0


def kendall_tau(x: list[float], y: list[float]) -> float:
    """Kendall's tau-a of two equally long lists (ties count as neither concordant nor discordant)."""
    n, s = len(x), 0
    for i in range(n):
        for j in range(i + 1, n):
            d = (x[i] - x[j]) * (y[i] - y[j])
            s += (d > 0) - (d < 0)
    return s / (n * (n - 1) / 2) if n > 1 else 0.0


def recorded_prismatic_matches(records: list[dict]) -> dict[str, dict[tuple[str, int], tuple[float, int]]]:
    """Every PrismaticAI configuration (ai|settings, pooled across labels as paired_vs pools them) of
    the battle records: {configuration: {(edition, seed): (points, value)}}, the first finished match
    of each edition and seed in record order (a replay is not counted)."""
    out: dict[str, dict[tuple[str, int], tuple[float, int]]] = {}
    for rec in records:
        ident = {c["key"]: c.get("ai", c["key"]) + "|" + c.get("settings", "") for c in rec.get("contestants", [])}
        for m in rec.get("matches", []):
            if m.get("status") != "done" or m.get("replica_of"):
                continue
            st = m.get("standings") or []
            for rank, f in enumerate(st):
                cid = ident.get(f["key"], f["key"])
                if cid.startswith("PrismaticAI"):
                    out.setdefault(cid, {}).setdefault((rec.get("edition", ""), m["seed"]),
                                                       ((len(st) - 1 - rank) / max(1, len(st) - 1), f["value"]))
    return out


def power(records: list[dict], margin: float = 1.10, alpha: float = 0.05, power_: float = 0.80,
          min_pairs: int = 16, min_seeds: int = 8, base: str = "PrismaticAI|",
          first_seeds: tuple[int, ...] = (4242, 31337, 20208)) -> dict:
    """The noise of the recorded battle and the held-start pairs a strategy comparison needs
    (research/battlebrain/experiment-freight-margin.md). Derived from the records alone:
    - `split_half`: Kendall's tau between the mean points of each configuration (>= min_seeds seeds)
      on its odd-ranked and its even-ranked seeds (seeds in ascending order), and the seeds a GA
      score would need for a rank reliability of 0.8 (Greiner's r = sin(pi tau / 2), Spearman-Brown);
    - `paired`: the standard deviation across seeds of ln(value / base value) for every configuration
      with >= min_pairs (edition, seed) cells in common with `base` (paired_vs's pairs);
    - `pairs`: the paired, one-sided non-inferiority sample size for a margin `margin` (a ratio) on
      the median of those deviations: n = ((z_{1-alpha} + z_{power}) sigma / ln margin)^2 + z_{1-alpha}^2 / 2
      (Guenther's correction for a t test), rounded up, never under three (rule
      project.a-tuning-claim-needs-held-start-pairs), and the seeds of those pairs: `first_seeds`
      (the rule's three), then the distinct seeds `base` played in the records, in record order."""
    from statistics import NormalDist, median, quantiles, stdev
    by_cfg = recorded_prismatic_matches(records)
    halves = []
    for cid, cells in by_cfg.items():
        seeds: dict[int, float] = {}
        for (_, s), (pts, _) in cells.items():
            seeds.setdefault(s, pts)            # one score per seed: its first recorded match
        if len(seeds) >= min_seeds:
            pts = [seeds[s] for s in sorted(seeds)]
            halves.append((sum(pts[0::2]) / len(pts[0::2]), sum(pts[1::2]) / len(pts[1::2]), len(pts)))
    tau = kendall_tau([h[0] for h in halves], [h[1] for h in halves]) if len(halves) > 1 else None
    reliability = None
    if tau is not None and tau > 0:
        r_half = math.sin(math.pi * tau / 2)
        k_half = median(h[2] for h in halves) / 2
        r_one = r_half / (k_half - (k_half - 1) * r_half)
        reliability = {"r_half": round(r_half, 4), "seeds_per_half": k_half, "r_one_seed": round(r_one, 4),
                       "seeds_for_r_0_8": math.ceil(0.8 * (1 - r_one) / (r_one * 0.2))}
    base_cells = by_cfg.get(base, {})
    sds = []
    for cid, cells in by_cfg.items():
        if cid == base:
            continue
        logs = [math.log(max(1, v) / max(1, base_cells[k][1])) for k, (_, v) in cells.items() if k in base_cells]
        if len(logs) >= min_pairs:
            sds.append(stdev(logs))
    sigma = median(sds) if sds else None
    z = NormalDist().inv_cdf
    za, zb = z(1 - alpha), z(power_)

    def n_for(m: float) -> int | None:
        if sigma is None:
            return None
        return max(3, math.ceil(((za + zb) * sigma / math.log(m)) ** 2 + za ** 2 / 2))
    n = n_for(margin)
    seeds = list(first_seeds)
    for (_, s) in base_cells:
        if n is not None and len(seeds) >= n:
            break
        if s not in seeds:
            seeds.append(s)
    return {"schema": "battle-power/v1", "records": len(records),
            "split_half": {"configurations": len(halves), "min_seeds": min_seeds,
                           "kendall_tau": None if tau is None else round(tau, 4), "reliability": reliability},
            "paired": {"base": base, "configurations": len(sds), "min_pairs": min_pairs,
                       "sigma_median": None if sigma is None else round(sigma, 4),
                       "sigma_quartiles": [round(q, 4) for q in quantiles(sds, n=4)] if len(sds) > 3 else None},
            "pairs": {"margin": margin, "delta_log": round(math.log(margin), 4), "alpha_one_sided": alpha,
                      "power": power_, "n": n, "seeds": seeds[:n] if n is not None else [],
                      "sensitivity": {f"{m:.2f}": n_for(m) for m in (1.05, 1.10, 1.15, 1.20, 1.25)}}}


def cmd_power(margin: float, alpha: float, power_: float) -> int:
    records = [json.loads(p.read_text()) for p in sorted(RESULTS_DIR.glob(RECORD_GLOB))]
    out = power(records, margin, alpha, power_)
    out["source"] = f"research/results/{RECORD_GLOB} at {git('rev-parse', '--short', 'HEAD')}"
    print(json.dumps(out, indent=1))
    return 0 if out["pairs"]["n"] is not None else 12


def ablate_rows(cfg: dict, seed: int, years: int, variants: list[str], field: list[str],
                log_dir: str | None = None, slot: int = 0, binary: Path | None = None,
                owner: str = "", timeout: int = 1800) -> list[dict] | None:
    """One local null-driver match per variant of the working tree's PrismaticAI (settings
    "name=a=1,b=2") against the same field on the same seed, in parallel; a row per variant with its
    final standing and, under `standings`, every company's final value, fleet per mode and last
    year's profit per mode (company-id order, keyed as the lineup names them). None when the binary
    or the prepared arena is missing. `owner` goes into every match's environment as
    TTD_BATTLE_ORCHESTRATOR (rule project.long-runs-are-owned-bounded-and-stoppable). A match that
    outlives `timeout` is killed, as are the others when one fails, so no game process outlives
    the call."""
    arena = cfg["cache"] / "arena"
    binary = binary or REPO / "build" / "openttd"
    if not Path(binary).is_file() or not (arena / "baseset").is_dir():
        return None
    import tempfile
    mc = dict(cfg["match"], years=years)
    rows = []
    env_owner = {"TTD_BATTLE_ORCHESTRATOR": owner} if owner else {}
    with tempfile.TemporaryDirectory(prefix="battle-ablate-") as tmp:
        ai = Path(tmp) / "ai"
        ai.mkdir()
        subprocess.run(["cp", "-R", str(REPO / cfg["prismatic"]["path"]), str(ai)], check=True)
        subprocess.run(["cp", "-R", str(arena / "ai" / "library"), *[str(t) for t in (arena / "ai").glob("*.tar")
                        if t.name != "PrismaticAI.tar"], str(ai)], check=True)
        (Path(tmp) / "battle.cfg").write_text(battle_cfg_text(mc))
        procs = []
        try:
            for v in variants:
                name, _, settings = v.partition("=")
                home = Path(tmp) / f"h-{name}"
                per = home / "Documents" / "OpenTTD"
                (per / "scripts").mkdir(parents=True)
                os.symlink(ai, per / "ai")
                os.symlink(arena / "baseset", per / "baseset")
                os.symlink(arena / "game", per / "game")
                (home / ".local" / "share").mkdir(parents=True)
                os.symlink(per, home / ".local" / "share" / "openttd")
                # `slot`: PrismaticAI's place in the start order (company id), as in the arena's
                # match; who starts first gets the first airport sites, so a replay must keep it.
                lineup = [{"ai": f, "key": f} for f in field]
                lineup.insert(min(slot, len(lineup)), {"ai": "PrismaticAI", "key": name, "settings": settings})
                (per / "scripts" / "game_start.scr").write_text(start_script_text(lineup))
                procs.append((name, settings, lineup, subprocess.Popen(
                    [str(binary), "-c", "battle.cfg", "-x", "-v", f"null:ticks={match_ticks(years)}", "-s", "null",
                     "-m", "null", "-g", "-G", str(seed), "-d", f"script={4 if log_dir else 1}"], cwd=tmp,
                    env=dict(os.environ, HOME=str(home), **env_owner), stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT, text=True)))
            for name, settings, lineup, proc in procs:
                out, _ = proc.communicate(timeout=timeout)
                if log_dir:
                    Path(log_dir).mkdir(parents=True, exist_ok=True)
                    (Path(log_dir) / f"{name}.log").write_text(out)
                rows.append(ablation_row(name, settings, proc.returncode, out, field, slot))
        finally:
            for *_, proc in procs:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
                proc.stdout.close()
    return rows


def ablation_record(cfg: dict, seed: int, years: int, field: list[str], slot: int, rows: list[dict]) -> dict:
    """The ablation record of one seed's rows (research/results/ablation-<utc>-seed<seed>.json)."""
    now = dt.datetime.now(dt.timezone.utc)
    return {"schema": ABLATION_SCHEMA, "id": f"ablation-{now.strftime('%Y%m%dT%H%M%S')}-seed{seed}",
            "generated_at": now.isoformat(timespec="seconds"), "seed": seed, "years": years, "field": field,
            "slot": slot, "match_cfg": dict(cfg["match"], years=years), "rev": git("rev-parse", "HEAD"),
            "tree_dirty": bool(git("status", "--porcelain", "--", cfg["prismatic"]["path"])), "rows": rows}


def write_ablation(rec: dict) -> None:
    for e in validate_ablation(rec, f"{rec['id']}.json"):
        log(f"warning: {e}")
    (RESULTS_DIR / f"{rec['id']}.json").write_text(json.dumps(rec, indent=1) + "\n")
    # The page (table and charts) is derived from the JSON alone, as `render` re-derives it.
    (RESULTS_DIR / f"{rec['id']}.md").write_text(render_ablation_md(rec))
    log(f"wrote research/results/{rec['id']}.{{json,md}}")


def cmd_ablate(cfg: dict, seed: int, years: int, variants: list[str], field: list[str],
               write: bool, log_dir: str | None = None, slot: int = 0) -> int:
    """Run one local match per variant of the working tree's PrismaticAI (ablate_rows), print (and
    with --write record under research/results/) every variant's final standing. Deterministic:
    same tree, binary and seed, same table."""
    rows = ablate_rows(cfg, seed, years, variants, field, log_dir, slot)
    if rows is None:
        print("ablate: NOT RUN: needs build/openttd and a prepared arena cache")
        return 12
    for r, vs in zip(rows, ablation_vs_first(rows)):
        r["vs_first"] = vs
        print(f"ablate: {r['variant']:12} value {r['value']:>12,} ({r['vs_first']:+.1%})  veh {r['vehicles']:>4} "
              f"rail {r['rail']} road {r['road']} air {r['air']}  err {r['errors']}  [{r['settings']}]")
    if write:
        write_ablation(ablation_record(cfg, seed, years, field, slot, rows))
    return 0


# ------------------------------------------------------------------------- config evolution (evolve)
#
# The closed loop from "we fall behind" to a tested settings proposal (research/config-evolution-loop.md):
# a trigger (the gap watch's gap.<company>, the strategy review's review.<company>.month, or a live
# outcome record live-*.json) -> the gap attributed per transport mode from the referee's profit per
# mode -> candidate settings: the GA genes of the losing modes and of capital, ranked and directed by
# their recorded association with the ratio to the reference AI -> every candidate and the baseline in
# paired null-driver matches on the same seeds -> the best candidate confirmed on fresh seeds -> a
# proposal record, and with --propose a blackboard key. The loop never applies a setting and never
# approves anything (rule project.decision-authority-boundaries): the owner, or the executor ADR 0042
# names, files it as one approval.

EVOLVE_SCHEMA = "config-evolution/v1"
# The referee's transport modes and the blackboard's names for them (gap_watch, ADR 0042 reviews).
MODE_ALIASES = {"rail": "rail", "train": "rail", "road": "road", "bus": "road", "truck": "road",
                "air": "air", "aircraft": "air", "plane": "air", "water": "water", "ship": "water"}
EVOLVE_LIFECYCLE = ["proposed", "requested", "approved|denied|retired", "applied", "verified|failed"]


def evolve_cfg(cfg: dict) -> dict:
    """The loop's policy: battle.json's `evolve`, under the configuration's own when it has one."""
    base = json.loads(DEFAULT_CONFIG.read_text()).get("evolve", {})
    return {**base, **cfg.get("evolve", {})}


def gene_mode(gene: str, ev: dict) -> str:
    """The transport mode a GA gene steers: battle.json evolve.gene_modes when it names the gene, else
    the first token of its name that evolve.mode_tokens lists (air_routes_max: air; max_routes: road),
    else "capital" (reserve, investment pace, aggressiveness: they move every mode)."""
    if gene in ev.get("gene_modes", {}):
        return ev["gene_modes"][gene]
    for tok in gene.split("_"):
        for mode, toks in ev.get("mode_tokens", {}).items():
            if tok in toks:
                return mode
    return "capital"


def mode_gaps(ours: dict, rival: dict) -> dict:
    """The gap to the rival per mode from two final referee samples (ADR 0042, section 2): the profit
    gap P_R - P_O, its volume term (n_R - n_O) x p_R and its yield term n_O x (p_R - p_O), where n is
    the fleet of the mode and p its profit last year per vehicle. A mode the rival does not run has
    no volume term; one we do not run has no yield term."""
    out = {}
    for mode in MODES:
        n_o, n_r = ours.get(mode) or 0, rival.get(mode) or 0
        p_o, p_r = ours.get("p" + mode) or 0, rival.get("p" + mode) or 0
        y_o, y_r = (p_o / n_o if n_o else 0.0), (p_r / n_r if n_r else 0.0)
        out[mode] = {"gap": p_r - p_o, "volume": round((n_r - n_o) * y_r), "yield": round(n_o * (y_r - y_o)),
                     "ours": n_o, "rival": n_r}
    return out


def attributed_modes(gaps: dict, cover: float) -> list[str]:
    """The modes that make up `cover` of the positive volume gap, largest first; when no mode has a
    positive volume term (we run as many vehicles but each earns less), the same over the positive
    profit gap."""
    for term in ("volume", "gap"):
        pos = sorted(((g[term], m) for m, g in gaps.items() if g[term] > 0), reverse=True)
        total = sum(v for v, _ in pos)
        if total > 0:
            out, acc = [], 0.0
            for v, m in pos:
                out.append(m)
                acc += v
                if acc >= cover * total:
                    break
            return out
    return []


def sum_gaps(per_seed: list[dict]) -> dict:
    out = {m: {"gap": 0, "volume": 0, "yield": 0, "ours": 0, "rival": 0} for m in MODES}
    for g in per_seed:
        for m, v in g.items():
            for k in out[m]:
                out[m][k] += v[k]
    return out


def trigger_gaps(value) -> dict | None:
    """A review record's per-type gap terms (`gaps: [{type, volume, yield}]`, ADR 0042) as mode_gaps'
    shape, None when the trigger carries none."""
    if not isinstance(value, dict) or not isinstance(value.get("gaps"), list):
        return None
    out = {m: {"gap": 0, "volume": 0, "yield": 0, "ours": 0, "rival": 0} for m in MODES}
    for g in value["gaps"]:
        m = MODE_ALIASES.get(str(g.get("type", "")).lower())
        if m:
            vol, yld = int(g.get("volume") or 0), int(g.get("yield") or 0)
            out[m].update(volume=out[m]["volume"] + vol, gap=out[m]["gap"] + vol + yld,
                          **{"yield": out[m]["yield"] + yld})
    return out


def _ranks(xs: list[float]) -> list[float]:
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    ranks = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for k in range(i, j + 1):
            ranks[order[k]] = (i + j) / 2
        i = j + 1
    return ranks


def spearman(x: list[float], y: list[float]) -> float:
    rx, ry = _ranks(x), _ranks(y)
    n = len(x)
    mx, my = sum(rx) / n, sum(ry) / n
    sxy = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    sxx, syy = sum((a - mx) ** 2 for a in rx), sum((b - my) ** 2 for b in ry)
    return 0.0 if sxx == 0 or syy == 0 else sxy / math.sqrt(sxx * syy)


def lever_evidence(records: list[dict], t: dict, reference: str) -> dict:
    """What the recorded battles say about each gene: {gene: {rho, n, values}}, rho being Spearman's
    correlation between the gene's value in a PrismaticAI configuration and log(value / reference's
    value) in the same match, the log ratio centred per arena edition (code changed between them).
    Finished, non-replay matches only, each configuration once per edition and seed across records. Hypothesis
    generation only: the GA's genes co-vary, so a correlation is a direction to try, never a cause."""
    obs: list[tuple[str, dict, float]] = []
    seen = set()
    for rec in records:
        ident = {c["key"]: c for c in rec.get("contestants", [])}
        for m in rec.get("matches", []):
            if m.get("status") != "done" or m.get("replica_of"):
                continue
            vals = {f["key"]: f["value"] for f in m.get("standings") or []}
            if not vals.get(reference):
                continue
            for k, v in vals.items():
                c = ident.get(k, {})
                if c.get("ai") != "PrismaticAI":
                    continue
                key = (rec.get("edition", ""), c.get("settings", ""), m.get("seed"))
                if key in seen:
                    continue
                seen.add(key)
                genes = live_genes(t, settings_dict(c.get("settings", "")))
                obs.append((rec.get("edition", ""), genes, math.log(max(1, v) / vals[reference])))
    by_ed: dict[str, list[float]] = {}
    for ed, _, y in obs:
        by_ed.setdefault(ed, []).append(y)
    mean = {ed: sum(ys) / len(ys) for ed, ys in by_ed.items()}
    ys = [y - mean[ed] for ed, _, y in obs]
    out = {}
    for g in t["space"]:
        xs = [genes[g] for _, genes, _ in obs]
        out[g] = {"rho": round(spearman(xs, ys), 4) if len(xs) > 2 else 0.0, "n": len(xs),
                  "values": len(set(xs))}
    return out


def evolve_candidates(base: dict, space: dict, evidence: dict, modes: list[str], ev: dict) -> list[dict]:
    """Candidate gene settings from the data: the genes of the attributed modes, then those of
    capital; within each, those with a recorded direction (|rho| >= min_rho over >= min_history
    matches, over three distinct values or every value a switch has) first by |rho|. Each is moved
    step_frac of its range from the baseline in that direction (both directions for a gene without
    one): at most `levers` single-gene candidates, then one with every directed move together. A move
    a bound swallows is skipped."""
    pool = [g for g in space if gene_mode(g, ev) in set(modes) | {"capital"}]

    def directed(g):
        e = evidence.get(g, {})
        lo, hi, step = space[g]
        ok = (abs(e.get("rho", 0)) >= ev["min_rho"] and e.get("n", 0) >= ev["min_history"]
              and e.get("values", 0) >= min(3, (hi - lo) // step + 1))
        return (1 if e["rho"] > 0 else -1) if ok else 0
    order = sorted(pool, key=lambda g: (directed(g) == 0, gene_mode(g, ev) == "capital",
                                        -abs(evidence.get(g, {}).get("rho", 0)), g))
    out, moves = [], {}
    for g in order:
        lo, hi, step = space[g]
        delta = max(step, round(ev["step_frac"] * (hi - lo) / step) * step)
        d = directed(g)
        for sign in ([d] if d else [1, -1]):
            if len(out) >= ev["levers"]:
                break
            b = base.get(g, lo)
            v = clamp_gene(space, g, b + sign * delta)
            if v == b:
                continue
            out.append({"name": f"c{len(out) + 1}", "changed": {g: [b, v]},
                        "why": f"{g} ({gene_mode(g, ev)}) {'up' if v > b else 'down'}: "
                               + (f"rho {evidence[g]['rho']:+.3f} over {evidence[g]['n']} recorded matches"
                                  if d else "no recorded direction, both tried")})
            if d:
                moves[g] = [b, v]
    if len(moves) >= 2:
        out.append({"name": "combo", "changed": dict(moves), "why": "every directed move together"})
    return out


def apply_changes(settings: dict, changed: dict) -> str:
    return settings_str({**settings, **{g: v for g, (_, v) in changed.items()}})


def rival_of(row: dict, reference: str) -> int:
    return next((s["value"] for s in row.get("standings", []) if s["key"] == reference), 0)


def pair_of(seed: int, base: dict, cand: dict, reference: str) -> dict:
    """One held pair: the candidate against the baseline on one seed. `ours` is the value ratio,
    `spread` how far the unchanged reference AI moved between the two arms (what the change did to the
    field, read as the pair's noise), `relative` the change in our ratio to the reference. It favours
    the candidate only beyond the spread (rule project.a-tuning-claim-needs-held-start-pairs)."""
    rb, rc = rival_of(base, reference), rival_of(cand, reference)
    ours = max(1, cand["value"]) / max(1, base["value"])
    spread = abs(max(1, rc) / max(1, rb) - 1)
    rel = (max(1, cand["value"]) / max(1, rc)) / (max(1, base["value"]) / max(1, rb))
    ok = cand["exit_code"] == 0 and cand["errors"] <= base["errors"]
    return {"seed": seed, "base": base["value"], "cand": cand["value"], "rival_base": rb, "rival_cand": rc,
            "ours": round(ours, 4), "spread": round(spread, 4), "relative": round(rel, 4),
            "errors": [base["errors"], cand["errors"]], "favours": ok and ours > 1 + spread}


def geo(xs: list[float]) -> float:
    return round(math.exp(sum(math.log(max(1e-9, x)) for x in xs) / len(xs)), 4) if xs else 0.0


def judge(pairs: list[dict], ev: dict) -> tuple[bool, str]:
    """A candidate is kept when it has min_pairs pairs, every pair favours it beyond the reference's
    spread, and its geometric mean value ratio reaches min_gain."""
    if len(pairs) < ev["min_pairs"]:
        return False, f"{len(pairs)} pair(s), {ev['min_pairs']} needed"
    lost = [p["seed"] for p in pairs if not p["favours"]]
    if lost:
        return False, f"not beyond the reference's spread (or more errors) on seed(s) {', '.join(map(str, lost))}"
    g = geo([p["ours"] for p in pairs])
    if g < ev["min_gain"]:
        return False, f"geometric mean {g:.3f} under min_gain {ev['min_gain']}"
    return True, f"every pair favours it, geometric mean {g:.3f}"


class EvolveGuard:
    """Owned, bounded and stoppable (rule project.long-runs-are-owned-bounded-and-stoppable): a lock
    directory with an OWNER file (host, pid, start, commit, how to stop), refused while a live process
    holds it; before every batch the STOP and PAUSED files of the cache root, the match budget, the
    wall-clock cap and this machine's load and disk floor (battle.json resource_floor)."""

    def __init__(self, root: Path, budget: int, max_minutes: float, floor: dict, clock=time.time):
        self.root, self.budget, self.floor, self.clock = root, budget, floor, clock
        self.deadline = clock() + max_minutes * 60
        self.used = 0
        self.lock = root / "evolve" / "lock"
        self.marker = "evolve:" + owner_marker(socket.gethostname(), os.getpid())

    def acquire(self) -> str | None:
        self.lock.parent.mkdir(parents=True, exist_ok=True)
        try:
            self.lock.mkdir()
        except FileExistsError:
            held = (self.lock / "OWNER").read_text() if (self.lock / "OWNER").is_file() else ""
            m = re.search(r"^pid=(\d+)$", held, re.M)
            h = re.search(r"^host=(.*)$", held, re.M)
            alive = False
            if m and h and h.group(1) == socket.gethostname():
                try:
                    os.kill(int(m.group(1)), 0)
                    alive = True
                except (OSError, ValueError):
                    alive = False
            if alive or not m:
                return f"held: {held.strip() or self.lock}"
            log(f"evolve: taking over the lock of dead pid {m.group(1)}")
        (self.lock / "OWNER").write_text(
            f"# battle.py evolve (tools/research/ai-battle/battle.py): headless matches for a settings proposal\n"
            f"host={socket.gethostname()}\npid={os.getpid()}\nstarted_at={dt.datetime.now(dt.timezone.utc).isoformat(timespec='seconds')}\n"
            f"commit={git('rev-parse', 'HEAD')}\nmarker=TTD_BATTLE_ORCHESTRATOR={self.marker}\n"
            f"how_to_stop=create {self.root}/STOP (no new batch; the running one finishes) or "
            f"kill -TERM {os.getpid()}\n")
        return None

    def release(self) -> None:
        for f in (self.lock / "OWNER",):
            if f.is_file():
                f.unlink()
        if self.lock.is_dir():
            self.lock.rmdir()

    def admit(self, want: int) -> tuple[int, str]:
        """How many matches the next batch may start: 0 with the reason when it must not."""
        why = stop_requested(self.root)
        if why:
            return 0, f"STOP: {why}"
        paused = sorted(self.root.glob("PAUSED*"))
        if paused:
            first = (paused[0].read_text(errors="replace").splitlines() or [""])[0].strip()
            return 0, f"PAUSED: {first or paused[0]}"
        if self.clock() >= self.deadline:
            return 0, "wall-clock cap reached"
        left = self.budget - self.used
        if left <= 0:
            return 0, f"match budget {self.budget} spent"
        try:
            load = os.getloadavg()[0]
        except OSError:
            load = None
        import shutil
        free = shutil.disk_usage(self.root).free // 1024
        n, why = floor_decision(load, os.cpu_count(), free, min(want, left), self.floor)
        return n, ("" if n else why or "no room")


def evolve_batches(run, guard: EvolveGuard, seeds: list[int], variants: list[str],
                   have: dict | None = None) -> tuple[dict, str]:
    """{seed: {variant: row}} of every variant on every seed, a batch at a time (each as large as the
    guard admits), and why it stopped early ("" when complete). `have` holds rows already played:
    the null-driver match is deterministic, so a baseline played once per seed is not played again."""
    rows = {s: dict(v) for s, v in (have or {}).items()}
    for seed in seeds:
        rows.setdefault(seed, {})
        todo = [v for v in variants if v.partition("=")[0] not in rows[seed]]
        while todo:
            n, why = guard.admit(len(todo))
            if n <= 0:
                return rows, why
            batch, todo = todo[:n], todo[n:]
            got = run(seed, batch)
            if got is None:
                return rows, "NOT RUN: needs build/openttd and a prepared arena cache"
            guard.used += len(batch)
            for r in got:
                rows[seed][r["variant"]] = r
    return rows, ""


def http_json(url: str, body: dict | None = None, timeout: int = 30):
    """GET (or POST `body`) JSON from a panel; None for a 404."""
    import urllib.error
    import urllib.request
    base = re.match(r"^(https?://[^/]+)", url)
    req = urllib.request.Request(url, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"content-type": "application/json",
                                          "origin": base.group(1) if base else url})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read() or b"null")
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise


def shared_value(panel: str, key: str):
    """A blackboard key's value (GET /api/rhai/shared?key=), None when absent."""
    d = http_json(f"{panel.rstrip('/')}/api/rhai/shared?key={urllib.parse.quote(key)}")
    return d.get("value") if isinstance(d, dict) else None


def evolve_trigger(a, ev: dict, t_cfg: dict) -> dict:
    """What set the loop off, read-only: {fired, kind, source, company, why, baseline, gaps, levers}.
    A live outcome record fires when live_entries judges the company's configuration lost (battle.json
    tuning.live: under loss_ratio of the reference, or of the game's best PrismaticAI); a panel fires on gap.<company> at level >= trigger.gap_level or an alarm in review.<company>.month;
    --force fires with --baseline alone. The baseline is --baseline, else the record's start settings,
    else the running bot's settings (GET /api/bots)."""
    tr = ev["trigger"]
    t = {"fired": False, "kind": "manual", "source": "", "company": a.company, "why": "", "baseline": None,
         "gaps": None, "levers": []}
    if a.live:
        rec = json.loads(Path(a.live).read_text())
        t.update(kind="live", source=rec.get("id", str(a.live)))
        e = next((e for e in live_entries([rec], t_cfg) if e["company"] == a.company), None)
        if e is None:
            t["why"] = f"company {a.company} is not a PrismaticAI company of {t['source']}"
        else:
            t.update(baseline=e["settings"], fired=bool(e.get("lost")),
                     why=("lost: " if e.get("lost") else "not lost: ") + e["why"])
    elif a.panel:
        gap = shared_value(a.panel, f"gap.{a.company}")
        review = shared_value(a.panel, f"review.{a.company}.month")
        t.update(kind="panel", source=a.panel)
        why = []
        if isinstance(gap, dict) and (gap.get("level") or 0) >= tr["gap_level"]:
            why.append(f"gap.{a.company} level {gap['level']} ({', '.join(sorted(gap.get('flags') or {}))})")
            t["levers"] = [lv.get("name") for lv in gap.get("levers") or [] if isinstance(lv, dict)]
        if isinstance(review, dict) and review.get("alarms"):
            why.append(f"review.{a.company}.month alarms {review['alarms']}")
            t["gaps"] = trigger_gaps(review)
        t["fired"] = bool(why)
        t["why"] = "; ".join(why) or (f"gap.{a.company} level {(gap or {}).get('level', 'absent')}, "
                                      f"review.{a.company}.month {'without alarms' if review else 'absent'}")
        if t["fired"] and not a.baseline:
            bots = http_json(f"{a.panel.rstrip('/')}/api/bots") or {}
            run = next((b for b in bots.get("running", []) if b.get("id") == a.company), {})
            t["baseline"] = ",".join(f"{s['name']}={s['value']}" for s in run.get("settings", [])
                                     if isinstance(s, dict) and "name" in s and "value" in s)
    if a.force:
        t["fired"], t["why"] = True, (t["why"] + "; " if t["why"] else "") + "forced (--force)"
    if a.baseline:
        t["baseline"] = a.baseline
    return t


def cmd_evolve(cfg: dict, a, run=None, records: list[dict] | None = None, live: list[dict] | None = None) -> int:
    """The config-evolution loop, once (research/config-evolution-loop.md). Exit 0 when it ran to a
    verdict or had nothing to do, 10 when another evolve holds the lock, 12 when it could not run a
    match, 2 on a trigger without a baseline."""
    ev = evolve_cfg(cfg)
    # load_config has resolved a configuration's genes already (space and defaults); a raw one is resolved here.
    tun = cfg.get("tuning") or json.loads(DEFAULT_CONFIG.read_text())["tuning"]
    t_cfg = tun if "defaults" in tun else live_tuning({"tuning": tun, "prismatic": cfg.get("prismatic", {})})
    t = evolve_trigger(a, ev, t_cfg)
    print(f"evolve: trigger {t['kind']} {t['source']}: {t['why']}")
    if not t["fired"]:
        print("evolve: no trigger fired; nothing run")
        return 0
    if not t["baseline"]:
        print("evolve: no baseline settings (give --baseline)", file=sys.stderr)
        return 2
    base_settings = settings_dict(t["baseline"])
    base_text = settings_str(base_settings)
    out_dir = cfg["cache"] / "evolve"
    out_dir.mkdir(parents=True, exist_ok=True)
    sig = hashlib.sha256(json.dumps([t["kind"], t["source"], t["company"], base_text], sort_keys=True).encode()).hexdigest()[:16]
    last_f = out_dir / "last.json"
    last = json.loads(last_f.read_text()) if last_f.is_file() else {}
    if not a.force and sig in last and time.time() - last[sig] < ev["cooldown_hours"] * 3600:
        print(f"evolve: the same trigger ran {(time.time() - last[sig]) / 3600:.1f} h ago "
              f"(cooldown {ev['cooldown_hours']} h); nothing run")
        return 0
    base_genes = live_genes(t_cfg, base_settings)
    guard = EvolveGuard(cfg["cache"], a.budget or ev["budget"], a.max_minutes or ev["max_minutes"],
                        cfg.get("resource_floor", {}))
    held = guard.acquire()
    if held:
        print(f"evolve: another evolve runs ({held}); nothing run", file=sys.stderr)
        return 10
    binary = Path(a.binary) if a.binary else None
    run = run or (lambda seed, vs: ablate_rows(cfg, seed, ev["years"], vs, ev["field"], None, ev["slot"],
                                               binary, guard.marker))
    try:
        rec = evolve_once(cfg, ev, t, run, guard, base_settings, base_genes, t_cfg, records, live)
    finally:
        guard.release()
    played = rec.pop("_rows")
    last[sig] = time.time()
    last_f.write_text(json.dumps(last, indent=1) + "\n")
    path = out_dir / f"{rec['id']}.json"
    path.write_text(json.dumps(rec, indent=1) + "\n")
    print(f"evolve: {rec['status']}: {rec['verdict']}")
    print(f"evolve: wrote {path}")
    if a.write:
        for seed, rows in played.items():
            ordered = [rows[n] for n in ["baseline"] + [c["name"] for c in rec["candidates"]] if n in rows]
            if not ordered or ordered[0]["variant"] != "baseline":
                continue
            for r, vs in zip(ordered, ablation_vs_first(ordered)):
                r["vs_first"] = vs
            write_ablation(ablation_record(cfg, seed, ev["years"], ev["field"], ev["slot"], ordered))
    if a.propose:
        key = f"evolve.proposal.{t['company']}" if rec["status"] == "proposed" else f"evolve.status.{t['company']}"
        http_json(f"{a.propose.rstrip('/')}/api/rhai/shared", {"key": key, "value": rec, "writer": "evolve"})
        print(f"evolve: blackboard {key} on {a.propose} (a record; it applies nothing)")
    return 12 if rec["status"] == "not_run" else 0


def evolve_once(cfg: dict, ev: dict, t: dict, run, guard: EvolveGuard, base_settings: dict,
                base_genes: dict, t_cfg: dict, records: list[dict] | None, live: list[dict] | None) -> dict:
    """One pass: baseline on the screening seeds, attribution, candidates (a configuration that lost a
    live game is vetoed, the winner of a paired live comparison joins: live_veto, live_seeds),
    screening, confirmation; the record (its `_rows` holds every row, for --write)."""
    space = t_cfg["space"]
    now = dt.datetime.now(dt.timezone.utc)
    seeds, confirm = list(ev["seeds"]), list(ev["confirm_seeds"])
    base_v = f"baseline={settings_str(base_settings)}"
    rec = {"schema": EVOLVE_SCHEMA, "id": f"evolve-{now.strftime('%Y%m%dT%H%M%S')}-c{t['company']}",
           "generated_at": now.isoformat(timespec="seconds"), "status": "incomplete", "verdict": "",
           "lifecycle": EVOLVE_LIFECYCLE, "trigger": {k: v for k, v in t.items() if k != "fired"},
           "binds": {"company": t["company"], "source": t["source"], "rev": git("rev-parse", "HEAD"),
                     "baseline": settings_str(base_settings),
                     "baseline_sha": hashlib.sha256(settings_str(base_settings).encode()).hexdigest(),
                     "field": ev["field"], "slot": ev["slot"], "years": ev["years"], "reference": ev["reference"],
                     "seeds": seeds, "confirm_seeds": confirm},
           "expires_at": (now + dt.timedelta(hours=ev["expiry_hours"])).isoformat(timespec="seconds"),
           "apply": "never by this loop: the owner (or the executor ADR 0042 names) files it as one "
                    "set_settings approval; a committed default still needs held-start pairs",
           "attribution": None, "evidence": {}, "candidates": [], "winner": None, "matches": 0, "_rows": {}}
    rows, why = evolve_batches(run, guard, seeds, [base_v])
    rec["_rows"], rec["matches"] = rows, guard.used
    if why:
        rec.update(status="not_run" if why.startswith("NOT RUN") else "incomplete", verdict=f"baseline: {why}")
        return rec
    per_seed = []
    for seed in seeds:
        b = rows[seed]["baseline"]
        me = next((s for s in b["standings"] if s["key"] == "baseline"), {})
        rv = next((s for s in b["standings"] if s["key"] == ev["reference"]), {})
        per_seed.append(mode_gaps(me, rv))
    headless = sum_gaps(per_seed)
    gaps = t.get("gaps") or headless
    modes = attributed_modes(gaps, ev["cover"])
    rec["attribution"] = {"source": "trigger" if t.get("gaps") else "baseline matches", "modes": modes,
                          "gaps": gaps, "headless": headless}
    evidence = lever_evidence(records if records is not None else
                              [json.loads(f.read_text()) for f in tracked_records(RECORD_GLOB)],
                              t_cfg, ev["reference"])
    cands = evolve_candidates(base_genes, space, evidence, modes, ev)
    live = live_entries(live_records(), t_cfg) if live is None else live
    tried = [base_genes] + [dict(base_genes, **{g: v for g, (_, v) in c["changed"].items()}) for c in cands]
    for k, (genes, source) in enumerate(live_seeds(t_cfg, live, tried)):
        changed = {g: [base_genes[g], v] for g, v in genes.items() if v != base_genes[g]}
        if changed:
            cands.append({"name": f"live{k + 1}", "changed": changed,
                          "why": f"won a paired live comparison ({source}); the arena measures it"})
    rec["vetoed"] = []
    for c in list(cands):
        veto = live_veto(t_cfg, dict(base_genes, **{g: v for g, (_, v) in c["changed"].items()}), live)
        if veto:
            cands.remove(c)
            rec["vetoed"].append({"name": c["name"], "changed": c["changed"],
                                  "by": f"{veto['record']}:{veto['company']}", "why": veto["why"]})
    # The budget holds every candidate on every screening seed and two confirmation arms per seed.
    room = (guard.budget - guard.used - 2 * len(confirm)) // max(1, len(seeds))
    cands = cands[:max(0, room)]
    rec["evidence"] = {g: dict(evidence[g], mode=gene_mode(g, ev)) for g in space
                       if gene_mode(g, ev) in set(modes) | {"capital"}}
    rec["candidates"] = [dict(c, settings=apply_changes(base_settings, c["changed"])) for c in cands]
    if not cands:
        rec.update(status="inconclusive", verdict=f"no candidate for modes {modes} within the budget")
        return rec
    rows, why = evolve_batches(run, guard, seeds, [f"{c['name']}={c['settings']}" for c in rec["candidates"]], rows)
    rec["_rows"], rec["matches"] = rows, guard.used
    for c in rec["candidates"]:
        c["pairs"] = [pair_of(s, rows[s]["baseline"], rows[s][c["name"]], ev["reference"])
                      for s in seeds if c["name"] in rows[s]]
        c["kept"], c["judged"] = judge(c["pairs"], ev)
        c["relative"] = geo([p["relative"] for p in c["pairs"]])
    if why:
        rec.update(status="incomplete", verdict=f"screening: {why}")
        return rec
    kept = sorted((c for c in rec["candidates"] if c["kept"]), key=lambda c: -c["relative"])
    if not kept:
        rec.update(status="inconclusive", verdict="no candidate favoured on every screening seed: "
                   + "; ".join(f"{c['name']} {c['judged']}" for c in rec["candidates"]))
        return rec
    best = kept[0]
    rows, why = evolve_batches(run, guard, confirm, [base_v, f"{best['name']}={best['settings']}"], rows)
    rec["_rows"], rec["matches"] = rows, guard.used
    best["confirm"] = [pair_of(s, rows[s]["baseline"], rows[s][best["name"]], ev["reference"])
                       for s in confirm if best["name"] in rows.get(s, {})]
    ok, judged = judge(best["confirm"], ev)
    if why:
        rec.update(status="incomplete", verdict=f"confirmation of {best['name']}: {why}")
    elif not ok:
        rec.update(status="inconclusive", verdict=f"{best['name']} won the screening but not the "
                                                   f"confirmation seeds: {judged}")
    else:
        rec["winner"] = {"name": best["name"], "settings": best["settings"], "changed": best["changed"],
                         "screening": best["judged"], "confirmation": judged,
                         "relative": geo([p["relative"] for p in best["pairs"] + best["confirm"]])}
        rec.update(status="proposed", verdict=f"{best['name']} ({', '.join(f'{g} {a}->{b}' for g, (a, b) in best['changed'].items())}): "
                                              f"{judged} on the confirmation seeds")
    rec["proposal_sha"] = hashlib.sha256(json.dumps({k: rec[k] for k in ("binds", "winner", "expires_at")},
                                                    sort_keys=True).encode()).hexdigest()
    return rec


def cmd_status(cfg: dict) -> None:
    orch = Orchestrator(cfg)
    print(f"edition {orch.digest} (rev {orch.provenance['rev'][:10]})")
    print(f"{'contestant':40} {'played':>6} {'wins':>5} {'pts':>5} {'avg rank':>8} "
          f"{'avg value':>14} {'sq errs':>7}")
    for r in leaderboard([c["key"] for c in orch.contestants], list(orch.matches.values())):
        print(f"{r['key']:40} {r['played']:6} {r['wins']:5} {r['points']:5} "
              f"{r['avg_rank'] if r['avg_rank'] is not None else '-':>8} "
              f"{r['avg_value']:>14,} {r['errors']:7}")
    for d in determinism(list(orch.matches.values())):
        print(f"replay {d['replay']} of {d['original']} ({' / '.join(d['hosts'])}): "
              f"{'identical' if d['identical'] else 'DIFFERENT'}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("command", choices=["discover", "fetch", "prepare", "probe", "run", "status",
                                        "report", "render", "check", "fitness", "ablate", "reap", "power",
                                        "live-export", "live-settings", "replay", "evolve"])
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--allow-dirty", action="store_true",
                    help="prepare: export uncommitted PrismaticAI/referee (records are refused)")
    ap.add_argument("--referee-only", action="store_true",
                    help="prepare: replace only the arena's referee with the repository's "
                         "(battle.json referee.rev, default HEAD); PrismaticAI and the AIs stay")
    ap.add_argument("--check", action="store_true", help="report: only check, write nothing")
    ap.add_argument("--seed", type=int, help="ablate: the map seed (default 930967890; with "
                                                   "--live-like the live game's, battle.json live_like.seed)")
    ap.add_argument("--years", type=int, help="ablate: game years (default 6); replay: game years "
                                               "after the load (default battle.json replay.years)")
    ap.add_argument("--variant", action="append", default=[],
                    help='ablate: "name=setting=v,setting=v" (the first is the baseline), repeatable')
    ap.add_argument("--field", default="AAAHogEx,AdmiralAI", help="ablate: comma-separated opponents")
    ap.add_argument("--write", action="store_true", help="ablate: record under research/results/; live-settings: write live-start.json")
    ap.add_argument("--slot", type=int, default=0,
                    help="ablate: PrismaticAI's place in the start order (its company id), as in the arena")
    ap.add_argument("--lineup", type=Path,
                    help="ablate: a match's game_start.scr; sets --field and --slot from it and adds "
                         "its PrismaticAI settings as variant 'arena'")
    ap.add_argument("--match", help="ablate: an arena match as <edition prefix>/<m-NNNN>; takes "
                                    "seed, years, field, slot and settings (variant 'arena') from "
                                    "its record")
    ap.add_argument("--log-dir", help="ablate: keep each variant's full log (script debug level 4) "
                                      "here; replay: keep its console and game log here")
    ap.add_argument("--live-like", action="store_true",
                    help="ablate: the live game's world (battle.json live_like): its match settings, "
                         "rivals, PrismaticAI's slot, seed and arms (unless --variant names others)")
    ap.add_argument("--save", help="replay: the live save to load (default battle.json replay.save)")
    ap.add_argument("--rev", help="replay: PrismaticAI at this commit instead of the working tree")
    ap.add_argument("--binary", help="replay, evolve: the OpenTTD binary (default build/openttd)")
    ap.add_argument("--edition", help="report: a past edition (its arena bundle must still be the "
                                      "prepared one, for its provenance)")
    ap.add_argument("--margin", type=float, default=1.10, help="power: the non-inferiority margin, a value ratio")
    ap.add_argument("--alpha", type=float, default=0.05, help="power: the one-sided significance level")
    ap.add_argument("--power", type=float, default=0.80, help="power: the power at no true difference")
    ap.add_argument("--game-seed", type=int, help="live-export: the map seed of the live game")
    ap.add_argument("--panel", help="live-export: the panel to read, e.g. http://127.0.0.1:8800 (read-only SQL); evolve: the panel whose blackboard\n(gap.<company>, review.<company>.month) is the trigger, read only")
    ap.add_argument("--db", type=Path, help="live-export: a copy of the panel's DuckDB store (never the live file)")
    ap.add_argument("--live", help="evolve: a live outcome record (research/results/live-*.json) as the trigger")
    ap.add_argument("--company", type=int, default=0, help="evolve: the company the trigger is about")
    ap.add_argument("--baseline", help="evolve: the baseline settings (else the trigger's)")
    ap.add_argument("--force", action="store_true", help="evolve: run without a fired trigger and past the cooldown")
    ap.add_argument("--budget", type=int, help="evolve: at most this many matches (battle.json evolve.budget)")
    ap.add_argument("--max-minutes", type=float, help="evolve: wall-clock cap (battle.json evolve.max_minutes)")
    ap.add_argument("--propose", help="evolve: a panel to write the proposal to as a blackboard key")
    a = ap.parse_args(argv)
    if a.command == "live-export":
        return cmd_live_export(a.game_seed, a.panel, a.db)
    if a.command == "live-settings":
        return cmd_live_settings(a.write)
    if a.command == "power":
        return cmd_power(a.margin, a.alpha, a.power)
    if a.command == "check":
        return cmd_check()
    if a.command == "render":
        return cmd_render()
    cfg = load_config(a.config)
    if a.command == "discover":
        cmd_discover(cfg)
    elif a.command == "fetch":
        cmd_fetch(cfg)
    elif a.command == "prepare" and a.referee_only:
        try:
            log(refresh_referee(cfg) or "arena referee is already the repository's")
        except RuntimeError as exc:
            log(f"refusing: {exc}")
            return 10
    elif a.command == "prepare":
        cmd_prepare(cfg, a.allow_dirty)
    elif a.command == "probe":
        for spec in load_hosts(cfg):
            h = Host(spec, cfg.get("remote_root", "$HOME/ttd-battle"), cfg.get("resource_floor"))
            h.probe()
            n = h.admit(0)
            print(f"{h.name:10} {h.state:10} slots={h.slots} admits={n} {h.version} {h.detail}".rstrip())
            if h.res.get("unparsed"):
                print(f"{'':10} probe figures not understood: {'; '.join(h.res['unparsed'])}")
    elif a.command == "run":
        follow_referee(cfg)
        Orchestrator(cfg).run(a.port)
    elif a.command == "status":
        cmd_status(cfg)
    elif a.command == "report":
        return cmd_report(cfg, a.check, a.edition)
    elif a.command == "fitness":
        return cmd_fitness(cfg)
    elif a.command == "replay":
        return cmd_replay(cfg, a.save, a.years, a.rev, a.binary, a.log_dir)
    elif a.command == "reap":
        Orchestrator(cfg).reap()
    elif a.command == "evolve":
        return cmd_evolve(cfg, a)
    elif a.command == "ablate":
        field, slot, variants = a.field.split(","), a.slot, list(a.variant)
        seed, years = (930967890 if a.seed is None else a.seed), (6 if a.years is None else a.years)
        if a.live_like:
            cfg, seed, field, slot, variants = live_like_ablate(cfg, a.seed, variants)
            for problem in rival_versions(cfg["cache"] / "arena" / "ai", live_like(cfg)["rivals"]):
                print(f"ablate: warning: {problem}")
        elif a.match:
            r = match_replay_inputs(cfg["results"], a.match)
            seed, years, field, slot = r["seed"], r["years"], r["field"], r["slot"]
            if not any(v.partition("=")[0] == "arena" for v in variants):
                variants.append(f"arena={r['settings']}")
            print(f"ablate: {a.match} ({r['key']}) seed {seed}, slot {slot}, field {','.join(field)}; "
                  f"played on {r['host']} with {r['host_version']}")
        elif a.lineup:
            field, slot, settings = parse_start_script(a.lineup.read_text())
            if not any(v.partition("=")[0] == "arena" for v in variants):
                variants.append(f"arena={settings}")
        return cmd_ablate(cfg, seed, years, variants, field, a.write, a.log_dir, slot)
    return 0


if __name__ == "__main__":
    sys.exit(main())
