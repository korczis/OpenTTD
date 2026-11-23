"""RESEARCH-ONLY. Failure regimes over company time series (research/failure-regimes.md).

A regime is a failure that lasts: a company that stops expanding while its rivals grow, falls
behind the strongest rival faster than the rest of the field, earns less a vehicle in a mode than
every rival that runs it, or whose planner's plan never reaches the bot. PrismaticAI ran the live
game of seed 15805 for a decade in such a regime (26 vehicles from 1951 to 1964 while AAAHogEx
went from 15 to 615) and nothing said so. Every threshold here is relative to the rivals in the
same game, or to the planner's own cycle; the few constants are named in RULES with the reason.

The detector is pure: `detect(series, who, rivals, rules)` takes {company: [sample]} (a sample is
{"t": "YYYY-MM", "value", "money", "loan", "vehicles"} and, from the headless referee, the
per-mode counts rail/road/air/water and last year's profits prail/proad/pair/pwater) and returns
a list of regime records. `unapplied_plan(rounds, rules)` judges the planner's rounds of a live
game. `failures` turns regimes into the fitness check's named reasons (`regime:<name>: ...`), and
`penalty` into one number per company for a search that wants a penalty term.

AlgoGlyph was looked at first and is not used: its catalogue has the fitting primitives
(page-hinkley change detection, least-squares), but at the pinned commit neither has a data form,
so the flow runner does not list them and runs them on their own seeded demo stream only; the
linked Rust crate carries sorting kernels only. A review also found Page-Hinkley's running-mean
statistic blind to a gap that widens steadily from the start, which is this failure.
"""
from __future__ import annotations

import json
import math
import statistics
from pathlib import Path

# The thresholds, one source for every reader (regimes.json, each with its reason under "why"):
# battle.json fitness.regimes or the caller overrides any of them.
RULES_FILE = Path(__file__).resolve().parent / "regimes.json"
RULES = json.loads(RULES_FILE.read_text())["rules"]

MODES = (("rail", "prail"), ("road", "proad"), ("air", "pair"), ("water", "pwater"))


def month_index(t: str) -> int:
    """"1960-04" or "1960-04-17" -> months since year 0, so two dates subtract."""
    return int(t[:4]) * 12 + int(t[5:7]) - 1


def theil_sen(xs: list[float], ys: list[float]) -> float:
    """The median of every pairwise slope (robust to a few wild samples); 0 with fewer than 2."""
    slopes = [(ys[j] - ys[i]) / (xs[j] - xs[i])
              for i in range(len(xs)) for j in range(i + 1, len(xs)) if xs[j] != xs[i]]
    return statistics.median(slopes) if slopes else 0.0


def growth(a: float, b: float) -> float:
    """Relative growth from a to b; a company that starts with nothing grows by its end count."""
    return (b - a) / max(a, 1)


def aligned(series: dict, keys: list[str]) -> list[str]:
    """The sample dates every company of `keys` has, in order."""
    common = None
    for k in keys:
        ts = {s["t"] for s in series.get(k, [])}
        common = ts if common is None else common & ts
    return sorted(common or [])


def by_t(samples: list[dict]) -> dict[str, dict]:
    return {s["t"]: s for s in samples}


def no_expansion(series: dict, who: str, rivals: list[str], rules: dict) -> dict | None:
    """The longest run of at least window_months in which our fleet stays flat (its maximum at most
    `slack` above its count at the run's start), the median of the rivals' own fleet growth over
    the same run exceeds `slack`, and we lose ground: our value over the strongest rival's ends the
    run lower than it started. A leader that stops adding vehicles while it pulls away (longer
    trains, bigger aircraft: a fleet count cannot see capacity) is not in this regime. `detected_at` is the first date the run had lasted
    window_months: when a watcher reading these samples would have known. `capital` says whether
    the company accumulated (idle: net money, money less loan, rose over the run) or not
    (starved): a diagnosis, never a second failure."""
    ts = aligned(series, [who] + rivals)
    if not rivals or len(ts) < 2:
        return None
    ours, theirs = by_t(series[who]), [by_t(series[r]) for r in rivals]
    lead = by_t(series[strongest(series, rivals)])

    def share(t: str) -> float:
        return ours[t]["value"] / max(1, lead[t]["value"])

    best = None
    for i, t0 in enumerate(ts):
        v0 = ours[t0]["vehicles"]
        top = v0
        j = i
        while j + 1 < len(ts) and max(top, ours[ts[j + 1]]["vehicles"]) <= v0 * (1 + rules["slack"]):
            j += 1
            top = max(top, ours[ts[j]]["vehicles"])
        t1 = ts[j]
        if month_index(t1) - month_index(t0) < rules["window_months"]:
            continue
        rival_growth = statistics.median(growth(r[t0]["vehicles"], r[t1]["vehicles"]) for r in theirs)
        if rival_growth <= rules["slack"] or share(t1) >= share(t0):
            continue
        if best is None or month_index(t1) - month_index(t0) > month_index(best["until"]) - month_index(best["since"]):
            detected = next(t for t in ts[i:j + 1] if month_index(t) - month_index(t0) >= rules["window_months"])
            net0 = ours[t0]["money"] - ours[t0].get("loan", 0)
            net1 = ours[t1]["money"] - ours[t1].get("loan", 0)
            best = {"regime": "no_expansion", "since": t0, "until": t1, "detected_at": detected,
                    "evidence": {"vehicles": [v0, ours[t1]["vehicles"]],
                                 "rival_growth_median": round(rival_growth, 3),
                                 "rivals": {k: [r[t0]["vehicles"], r[t1]["vehicles"]] for k, r in zip(rivals, theirs)},
                                 "share_of_strongest": [round(share(t0), 4), round(share(t1), 4)],
                                 "stations": [ours[t0].get("stations"), ours[t1].get("stations")],
                                 "net_money": [net0, net1],
                                 "capital": "idle" if net1 > net0 else "starved"},
                    "threshold": f"fleet within {rules['slack']:.0%} of its start for >= {rules['window_months']} "
                                 f"months while the rivals' median fleet growth exceeds {rules['slack']:.0%} "
                                 f"and our share of the strongest rival's value falls"}
    return best


def strongest(series: dict, rivals: list[str]) -> str:
    """The rival with the highest final value."""
    return max(rivals, key=lambda k: (series[k][-1]["value"] if series.get(k) else 0, k))


def log_ratio_slope(series: dict, a: str, b: str) -> tuple[float, float, int]:
    """(Theil-Sen slope a year of ln(value a / value b), the years spanned, samples used), over
    the dates both have, skipping a date where either is worth nothing (no ratio)."""
    sa, sb = by_t(series[a]), by_t(series[b])
    pts = [(month_index(t) / 12, math.log(sa[t]["value"] / sb[t]["value"]))
           for t in aligned(series, [a, b]) if sa[t]["value"] > 0 and sb[t]["value"] > 0]
    if len(pts) < 2:
        return 0.0, 0.0, len(pts)
    xs, ys = zip(*pts)
    return theil_sen(list(xs), list(ys)), xs[-1] - xs[0], len(pts)


def widening_gap(series: dict, who: str, rivals: list[str], rules: dict, leader: str | None = None) -> dict | None:
    """We fall behind the strongest rival materially faster than the median other rival does: the
    Theil-Sen slope of ln(ours / leader) is negative, and below the median of the other rivals'
    slopes of ln(theirs / leader) by at least ln(gap_factor) over the span judged, which lasts at
    least window_months. A trend, not a level: the value ratio itself is judged by the rival ratio
    check (rule project.prismatic-ai-is-judged-against-its-strongest-rival)."""
    if not rivals:
        return None
    leader = leader or strongest(series, rivals)
    ours, years, n = log_ratio_slope(series, who, leader)
    if years * 12 < rules["window_months"] or ours >= 0:
        return None
    peers = {k: log_ratio_slope(series, k, leader)[0] for k in rivals if k != leader}
    ref = statistics.median(peers.values()) if peers else 0.0
    margin = math.log(rules["gap_factor"]) / years
    if ours > ref - margin:
        return None
    return {"regime": "widening_gap", "since": series[who][0]["t"], "until": series[who][-1]["t"],
            "detected_at": series[who][-1]["t"],
            "evidence": {"leader": leader, "slope_per_year": round(ours, 3),
                         "peer_slopes_per_year": {k: round(v, 3) for k, v in peers.items()},
                         "peer_median": round(ref, 3), "years": round(years, 2), "samples": n,
                         "ratio": [round(series[who][0]["value"] / max(1, series[leader][0]["value"]), 4),
                                   round(series[who][-1]["value"] / max(1, series[leader][-1]["value"]), 4)]},
            "threshold": f"slope of ln(value / {leader}) below the other rivals' median by "
                         f"ln({rules['gap_factor']}) over {years:.1f} years"}


def year_ends(samples: list[dict]) -> list[dict]:
    """The first sample of each game year: its prail..pwater are the whole of the year before."""
    out, seen = [], set()
    for s in samples:
        if s["t"][:4] not in seen:
            seen.add(s["t"][:4])
            out.append(s)
    return out


def per_vehicle(prev: dict, now: dict, count: str, profit: str) -> float | None:
    """Last year's profit of a mode a vehicle that ran all of it: the profit over the fewer of the
    counts at the year's start and end (a vehicle bought during the year earned part of it, one
    sold took its profit away); None when the mode had no vehicle at either end."""
    n = min(prev.get(count, 0), now.get(count, 0))
    return now.get(profit, 0) / n if n > 0 else None


def mode_years(series: dict, who: str, rivals: list[str]) -> dict[str, list[tuple]]:
    """{mode: [(year end, our profit a vehicle, {rival: theirs})]} for each year end at which we ran
    the mode all year (per_vehicle); empty without the referee's per-mode profits."""
    ours = year_ends(series[who])
    if not ours or "prail" not in ours[0]:
        return {}
    theirs = {k: by_t(year_ends(series[k])) for k in rivals}
    out = {}
    for count, profit in MODES:
        for prev, now in zip(ours, ours[1:]):
            mine = per_vehicle(prev, now, count, profit)
            if mine is None:
                continue
            field = {k: per_vehicle(r[prev["t"]], r[now["t"]], count, profit)
                     for k, r in theirs.items() if prev["t"] in r and now["t"] in r}
            out.setdefault(count, []).append((now["t"], mine, {k: v for k, v in field.items() if v is not None}))
    return out


def mode_regimes(series: dict, who: str, rivals: list[str], rules: dict) -> list[dict]:
    """Two regimes per mode we run, each `pay_years` consecutive year ends long (the latest run):
    mode_never_pays, our vehicles of the mode lost money (profit a vehicle at most 0: such a
    vehicle never pays back its price), and mode_underperforms, our profit a vehicle is below every
    rival that ran the mode all year and below `pay_share` of their median (a diagnosis: the
    winner of every match shows it for its road feeders)."""
    out = []
    for mode, years in mode_years(series, who, rivals).items():
        tests = {"mode_never_pays": lambda mine, field: mine <= 0,
                 "mode_underperforms": lambda mine, field: bool(field) and mine < min(field.values())
                 and mine < rules["pay_share"] * statistics.median(field.values())}
        for name, bad in tests.items():
            run, last = [], None
            for y in years:
                run = run + [y] if bad(y[1], y[2]) else []
                if len(run) >= rules["pay_years"]:
                    last = list(run)
            if last:
                t, mine, field = last[-1]
                out.append({"regime": name, "since": last[0][0], "until": t,
                            "detected_at": last[rules["pay_years"] - 1][0],
                            "evidence": {"mode": mode, "per_vehicle": round(mine),
                                         "rivals_per_vehicle": {k: round(v) for k, v in field.items()},
                                         "years": len(last)},
                            "threshold": (f"{mode} vehicles lose money" if name == "mode_never_pays" else
                                          f"below every rival running {mode} and below {rules['pay_share']:.0%} "
                                          f"of their median") + f", {rules['pay_years']} year ends in a row"})
    return out


def unapplied_plan(rounds: list[dict], rules: dict) -> dict | None:
    """The planner's rounds of one company (replan.<company> as the store kept each write, oldest
    first: {date, proposed: [setting names whose planned value differs from the bot's live one],
    model: {adopt}, held, pending: [approval ids filed this round]}): the longest run, from a round
    that adopted a plan, in which the bot never runs the plan (proposed stays not empty, no other
    planner holds the company) while at least `plan_rounds` rounds adopt it. A round that holds
    (model.adopt false) neither ends the run nor counts: the plan it last adopted still waits.
    `filed` counts the approvals the run filed; with none filed after its first rounds, the plan
    waits on approvals nobody decided, or that expired."""
    best, run = None, []
    for r in rounds + [None]:
        open_ = r is not None and r.get("proposed") and not r.get("held")
        adopted = open_ and (r.get("model") or {}).get("adopt")
        if adopted or (open_ and run):
            run.append(r)
            continue
        took = [x for x in run if (x.get("model") or {}).get("adopt")]
        if len(took) >= rules["plan_rounds"] and (best is None or len(took) > best["evidence"]["adopted"]):
            best = {"regime": "unapplied_plan", "since": run[0]["date"], "until": run[-1]["date"],
                    "detected_at": took[rules["plan_rounds"] - 1]["date"],
                    "evidence": {"rounds": len(run), "adopted": len(took), "settings": len(run[-1]["proposed"]),
                                 "filed": sum(len(x.get("pending") or []) for x in run),
                                 "filed_rounds": sum(1 for x in run if x.get("pending"))},
                    "threshold": f"{rules['plan_rounds']} rounds adopt a plan the bot never runs"}
        run = []
    return best


def detect(series: dict, who: str, rivals: list[str], rules: dict | None = None,
           leader: str | None = None) -> list[dict]:
    """Every regime of company `who` against `rivals` in one game, as records {regime, since,
    until, detected_at, evidence, threshold}."""
    rules = {**RULES, **(rules or {})}
    if not series.get(who):
        return []
    rivals = [k for k in rivals if series.get(k)]
    found = [no_expansion(series, who, rivals, rules), widening_gap(series, who, rivals, rules, leader)]
    found += mode_regimes(series, who, rivals, rules)
    return [f for f in found if f]


def describe(r: dict) -> str:
    """One line: the regime, its span and its evidence."""
    e = r["evidence"]
    if r["regime"] == "no_expansion":
        what = (f"fleet {e['vehicles'][0]}->{e['vehicles'][1]} while the rivals' median grew "
                f"{e['rival_growth_median']:.0%}, capital {e['capital']} (net {e['net_money'][0]:,}->{e['net_money'][1]:,})")
    elif r["regime"] == "widening_gap":
        what = (f"ln(value/{e['leader']}) {e['slope_per_year']:+.2f}/year against the other rivals' median "
                f"{e['peer_median']:+.2f}, ratio {e['ratio'][0]}->{e['ratio'][1]}")
    elif r["regime"] in ("mode_underperforms", "mode_never_pays"):
        what = f"{e['mode']} earns {e['per_vehicle']:,} a vehicle, rivals {e['rivals_per_vehicle']}"
    else:
        what = (f"{e['rounds']} rounds ({e['adopted']} adopted), {e['settings']} setting(s) differ, "
                f"{e['filed']} approval(s) filed in {e['filed_rounds']} round(s)")
    return f"{r['regime']}: {r['since']}..{r['until']} (known {r['detected_at']}): {what}"


def failures(found: list[dict], key: str, rules: dict | None = None) -> list[str]:
    """The fitness check's failures for `key`'s regimes: those named in rules' gate."""
    gate = {**RULES, **(rules or {})}["gate"]
    return [f"{key}: regime:{describe(r)}" for r in found if r["regime"] in gate]


def penalty(found: list[dict]) -> float:
    """One number for a search: the months the regimes lasted, in game years, summed (0: none).
    A genetic algorithm can subtract it from a score, so a configuration that stagnates for longer
    ranks lower however its final value came out."""
    return round(sum((month_index(r["until"]) - month_index(r["since"])) / 12 for r in found), 2)


# --------------------------------------------------------------------------- live: the panel's store
#
# `regimes.py --panel URL [--post]` judges every running company of an AI in a live game from the
# panel's store (read-only SQL through POST /api/db/query) and its blackboard, and with --post
# writes the verdict to the blackboard key regime.<company> (a rhai.shared event: on the bus and in
# the store). `--db FILE` reads a copy of a store with the duckdb CLI instead (the same SQL), and
# `--record FILE` an exported live record (schema ai-live-outcome/v1, yearly samples). It reuses
# what other services computed: the leader-gap watch's gap.<company> names the strongest rival
# (its leader) and its flags ride along, and the planner's replan.<company> writes, as the store
# kept every one, are the rounds `unapplied_plan` judges. Nothing here acts on a game.

WRITER = "regimes"
SQL = {
    "run": "SELECT max(run_ms) AS run_ms FROM company_month",
    "series": "SELECT company, name, month, value, money, loan, vehicles FROM company_month "
              "WHERE run_ms = {run} ORDER BY company, month",
    "rounds": "SELECT game_date, CAST(data AS VARCHAR) AS data FROM run_events WHERE run_ms = {run} "
              "AND kind = 'rhai.shared' AND json_extract_string(CAST(data AS VARCHAR), '$.key') = "
              "'replan.{company}' ORDER BY seq",
}


def panel_query(url: str):
    """A query runner over the panel's read-only SQL endpoint (POST /api/db/query)."""
    def run(sql: str) -> list[dict]:
        d = http_json(url, "/api/db/query", {"sql": sql})
        if d.get("error") or d.get("capped"):
            raise SystemExit(f"regimes: {d.get('error') or 'result capped by the panel'}: {sql[:80]}")
        return [dict(zip(d["columns"], row)) for row in d["rows"]]
    return run


def duckdb_query(path: str):
    """A query runner over a copy of a store through the duckdb CLI, opened read-only."""
    import subprocess

    def run(sql: str) -> list[dict]:
        p = subprocess.run(["duckdb", "-readonly", "-json", path, "-c", sql], capture_output=True, text=True)
        if p.returncode:
            raise SystemExit(f"regimes: {p.stderr.strip()}")
        return json.loads(p.stdout or "[]")
    return run


def http_json(url: str, path: str, body: dict | None = None):
    """GET (no body) or POST JSON to the panel; the answer as JSON, None on 404."""
    import urllib.error
    import urllib.request
    base = url.rstrip("/")
    req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={"content-type": "application/json", "origin": base})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def store_series(query, run: int | None = None) -> tuple[int, dict, dict]:
    """(run_ms, {company: monthly samples}, {company: name}) of one run of the store (the newest
    by default; a loaded save starts a new run, so a game continued after a load is judged from
    its load on)."""
    run = int(run if run is not None else query(SQL["run"])[0]["run_ms"])
    series, names = {}, {}
    for r in query(SQL["series"].format(run=run)):
        c = str(int(r["company"]))
        series.setdefault(c, []).append({"t": r["month"], **{k: r[k] or 0 for k in ("value", "money", "loan", "vehicles")}})
        names[c] = r["name"]
    return run, series, names


def store_rounds(query, run: int, company: int) -> list[dict]:
    """The planner's rounds of `company`, oldest first: each replan.<company> write the store kept."""
    out = []
    for r in query(SQL["rounds"].format(run=int(run), company=int(company))):
        v = json.loads(r["data"]).get("value")
        if isinstance(v, dict):
            out.append(dict(v, date=v.get("date") or r["game_date"]))
    return out


def judge_company(series: dict, company: str, rivals: list[str], rounds: list[dict], gap: dict | None,
                  rules: dict) -> dict:
    """The blackboard value of regime.<company>: every regime, the penalty, and what it read."""
    leader = None
    if isinstance(gap, dict) and isinstance(gap.get("leader"), dict):
        lead = str(int(gap["leader"].get("company")))
        leader = lead if lead in rivals else None
    found = detect(series, company, rivals, rules, leader)
    plan = unapplied_plan(rounds, {**RULES, **rules})
    if plan:
        found.append(plan)
    last = series[company][-1]["t"] if series.get(company) else None
    return {"company": int(company), "month": last, "regimes": found, "lines": [describe(r) for r in found],
            "penalty": penalty(found), "leader": leader or (strongest(series, rivals) if rivals else None),
            "leader_from": "gap" if leader else "series",
            "gap_watch": {k: gap.get(k) for k in ("flags", "streaks", "level")} if isinstance(gap, dict) else None,
            "rounds": len(rounds), "rules": {**RULES, **rules},
            "reads": ["store:company_month", f"store:replan.{company}", f"gap.{company}"]}


def judge_store(query, ours: list[str], rules: dict, run: int | None = None,
                gaps: dict | None = None) -> dict[str, dict]:
    """regime.<company> for each company of `ours`, against every other company of the run that is
    not one of ours."""
    run, series, names = store_series(query, run)
    rivals = [c for c in series if c not in ours]
    out = {}
    for c in ours:
        if c in series:
            out[c] = dict(judge_company(series, c, rivals, store_rounds(query, run, int(c)),
                                        (gaps or {}).get(c), rules), name=names[c], run_ms=run)
    return out


def judge_record(rec: dict, rules: dict, ai: str = "PrismaticAI") -> dict[str, dict]:
    """regime.<company> for every company of `ai` in an exported live record (ai-live-outcome/v1)."""
    series = {k: [dict(p, t=p["month"]) for p in v] for k, v in rec["series"].items()}
    ours = [str(c["company"]) for c in rec["companies"] if c["ai"] == ai]
    rivals = [str(c["company"]) for c in rec["companies"] if c["ai"] != ai]
    return {c: dict(judge_company(series, c, rivals, [], None, rules), name=next(
        x["name"] for x in rec["companies"] if str(x["company"]) == c)) for c in ours if series.get(c)}


def live_regimes(rec: dict, ai: str = "PrismaticAI", rules: dict | None = None) -> dict[int, list[dict]]:
    """The one call for a search over live outcomes (battle.py live_entries, live_veto): every
    company of `ai` in an exported live record (ai-live-outcome/v1) and its regimes, as
    {company id: [regime records]}; `penalty` of a list is its weight in game years."""
    return {int(c): v["regimes"] for c, v in judge_record(rec, rules or {}, ai).items()}


def main(argv: list[str] | None = None) -> int:
    import argparse
    ap = argparse.ArgumentParser(description="Failure regimes of a live game's companies (research/failure-regimes.md)")
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--panel", help="a running panel, e.g. http://127.0.0.1:8800")
    src.add_argument("--db", help="a copy of a panel store (.duckdb), read with the duckdb CLI")
    src.add_argument("--record", help="an exported live record (research/results/live-*.json)")
    ap.add_argument("--company", action="append", default=[], help="--db: a company id to judge (repeatable)")
    ap.add_argument("--ai", default="PrismaticAI", help="--panel/--record: judge every company of this AI")
    ap.add_argument("--run-ms", type=int, help="--panel/--db: the run (default: the newest)")
    ap.add_argument("--post", action="store_true", help="--panel: write regime.<company> to the blackboard")
    ap.add_argument("--json", action="store_true", help="print the verdicts as JSON")
    a = ap.parse_args(argv)
    if a.record:
        out = judge_record(json.loads(Path(a.record).read_text()), {}, a.ai)
    elif a.db:
        if not a.company:
            ap.error("--db needs --company")
        out = judge_store(duckdb_query(a.db), [str(c) for c in a.company], {}, a.run_ms)
    else:
        bots = http_json(a.panel, "/api/bots") or {}
        ours = [str(b["id"]) for b in bots.get("running", []) if b.get("ai") == a.ai]
        gaps = {c: ((http_json(a.panel, f"/api/rhai/shared?key=gap.{c}") or {}).get("value")) for c in ours}
        out = judge_store(panel_query(a.panel), ours, {}, a.run_ms, gaps)
        if a.post:
            # The thresholds as data, for Rhai services (no file access there): regime.rules.
            http_json(a.panel, "/api/rhai/shared", {"key": "regime.rules", "value": json.loads(RULES_FILE.read_text()),
                                                    "writer": WRITER})
            for c, v in out.items():
                http_json(a.panel, "/api/rhai/shared", {"key": f"regime.{c}", "value": v, "writer": WRITER})
    if a.json:
        print(json.dumps(out, indent=1))
    else:
        for c, v in out.items():
            print(f"company {c} ({v['name']}): {len(v['regimes'])} regime(s), penalty {v['penalty']}")
            for line in v["lines"]:
                print(f"  {line}")
    return 10 if any(v["regimes"] for v in out.values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
