#!/usr/bin/env python3
"""RESEARCH-ONLY. Offline tests for `battle.py evolve`, the config-evolution loop
(research/config-evolution-loop.md), and the `ablate_rows` it plays matches through: no game, no
arena, no live panel. Matches come from a fake game binary that prints referee lines, or from a fake
`run`; the panel is a local HTTP server in the test.

Run: python3 -B tools/research/test_ai_battle_evolve.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/gate.sh change does).
"""
import argparse
import http.server
import urllib.error
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("battle_evolve_under_test", HERE / "ai-battle" / "battle.py")
battle = importlib.util.module_from_spec(spec)
sys.modules["battle_evolve_under_test"] = battle
spec.loader.exec_module(battle)

PRISMATIC = "tools/research/prismatic-ai/PrismaticAI"
GENES = ["line_budget", "own_lines", "aggressiveness", "air_planes", "max_routes"]
BASE = "aggressiveness=80,line_budget=30000,own_lines=4,air_planes=5,max_routes=30,self_steward=1,fr_from=12"


def ev_override(**kw):
    e = {"field": ["AAAHogEx"], "reference": "AAAHogEx", "slot": 0, "years": 1, "seeds": [1, 2, 3],
         "confirm_seeds": [4, 5, 6], "levers": 2, "step_frac": 0.25, "cover": 0.8, "min_rho": 0.05,
         "min_history": 5, "min_pairs": 3, "min_gain": 1.05, "budget": 40, "max_minutes": 5,
         "cooldown_hours": 24, "expiry_hours": 1}
    e.update(kw)
    return e


def config(cache: Path, **kw) -> dict:
    return {"cache": cache, "prismatic": {"path": PRISMATIC}, "resource_floor": {},
            "match": {"years": 1, "start_year": 1950, "landscape": "temperate", "map_x": 8, "map_y": 8},
            "tuning": {"genes": list(GENES), "narrow": {"line_budget": [30000, 200000, 10000]}},
            "evolve": ev_override(**kw)}


def args(**kw) -> argparse.Namespace:
    d = {"live": None, "panel": None, "company": 0, "baseline": None, "force": False, "budget": None,
         "max_minutes": None, "binary": None, "propose": None, "write": False}
    d.update(kw)
    return argparse.Namespace(**d)


def sample(value, rail=0, prail=0, air=0, pair=0, road=0, proad=0):
    return {"value": value, "vehicles": rail + air + road, "rail": rail, "road": road, "air": air, "water": 0,
            "prail": prail, "proad": proad, "pair": pair, "pwater": 0}


def row(name, value, rival=1_000_000, errors=0, exit_code=0, rail=1, prail=10_000):
    return {"variant": name, "settings": "", "exit_code": exit_code, "value": value, "errors": errors,
            "money": 0, "vehicles": rail, "rail": rail, "road": 0, "air": 0, "field_best": rival, "t": "1951-01",
            "standings": [dict(sample(value, rail=rail, prail=prail), company=0, key=name, errors=errors),
                          dict(sample(rival, rail=20, prail=400_000, air=1, pair=20_000), company=1,
                               key="AAAHogEx", errors=0)]}


def tuning(cfg=None) -> dict:
    """The GA's tuning, resolved against info.nut as cmd_evolve resolves it."""
    c = cfg or json.loads(battle.DEFAULT_CONFIG.read_text())
    return battle.live_tuning({"tuning": c["tuning"], "prismatic": {"path": PRISMATIC}})


def live_record(tmp: Path, ratio: float) -> str:
    """A live outcome record (schema ai-live-outcome/v1) whose company 0 ends 1951 at `ratio` of AAAHogEx."""
    f = tmp / f"live-{ratio}.json"
    f.write_text(json.dumps({
        "id": "live-9-1", "seed": 9, "first_month": "1950-01",
        "companies": [{"company": 0, "ai": "PrismaticAI", "settings_start": BASE, "settings_requested": []},
                      {"company": 1, "ai": "AAAHogEx", "settings_start": "", "settings_requested": []}],
        "series": {"0": [{"month": "1950-12", "value": 10}, {"month": "1951-12", "value": int(1000 * ratio)}],
                   "1": [{"month": "1950-12", "value": 500}, {"month": "1951-12", "value": 1000}]}}))
    return str(f)


def fake_run(gain_of, rival_of=lambda seed, name: 1_000_000, calls=None):
    """A `run(seed, variants)` whose value is 1000 x gain_of(seed, settings dict)."""
    def run(seed, variants):
        if calls is not None:
            calls.append((seed, [v.partition("=")[0] for v in variants]))
        out = []
        for v in variants:
            name, _, settings = v.partition("=")
            out.append(row(name, int(1000 * gain_of(seed, battle.settings_dict(settings))),
                           rival=rival_of(seed, name)))
        return out
    return run


def history(n=12):
    """Battle records in which a higher line_budget goes with a better ratio to AAAHogEx."""
    contestants = [{"key": f"P{i}", "ai": "PrismaticAI", "settings": f"line_budget={30000 + 10000 * i}"}
                   for i in range(4)] + [{"key": "AAAHogEx", "ai": "AAAHogEx", "settings": ""}]
    matches = []
    for s in range(n):
        st = [{"key": f"P{i}", "value": 1000 * (i + 1) + s} for i in range(4)] + [{"key": "AAAHogEx", "value": 9000}]
        matches.append({"seed": s, "status": "done", "replica_of": None, "standings": st})
    matches.append({"seed": 99, "status": "done", "replica_of": "m-1", "standings": []})
    matches.append({"seed": 98, "status": "done", "replica_of": None, "standings": [{"key": "P0", "value": 5}]})
    return [{"edition": "e1", "contestants": contestants, "matches": matches},
            {"edition": "e1", "contestants": contestants, "matches": matches[:1] * 2}]


class Attribution(unittest.TestCase):
    def test_mode_gaps_split_volume_and_yield(self):
        g = battle.mode_gaps(sample(1, rail=1, prail=30_000, air=2, pair=10_000),
                             sample(1, rail=10, prail=100_000, air=0, pair=0))
        self.assertEqual(g["rail"], {"gap": 70_000, "volume": 90_000, "yield": -20_000, "ours": 1, "rival": 10})
        self.assertEqual(g["air"]["volume"], 0)       # the rival runs no aircraft: no volume term
        self.assertEqual(g["air"]["gap"], -10_000)

    def test_attributed_modes_cover_the_volume_gap_then_fall_back_to_profit(self):
        gaps = {m: {"gap": 0, "volume": 0, "yield": 0} for m in battle.MODES}
        self.assertEqual(battle.attributed_modes(gaps, 0.8), [])
        gaps["rail"]["volume"], gaps["air"]["volume"], gaps["road"]["volume"] = 70, 25, 5
        self.assertEqual(battle.attributed_modes(gaps, 0.8), ["rail", "air"])
        gaps2 = {m: {"gap": 0, "volume": -1, "yield": 0} for m in battle.MODES}
        gaps2["road"]["gap"] = 10
        self.assertEqual(battle.attributed_modes(gaps2, 0.8), ["road"])

    def test_sum_and_trigger_gaps(self):
        one = battle.mode_gaps(sample(1, rail=1, prail=1), sample(1, rail=3, prail=9))
        total = battle.sum_gaps([one, one])
        self.assertEqual(total["rail"]["volume"], 2 * one["rail"]["volume"])
        self.assertIsNone(battle.trigger_gaps(None))
        self.assertIsNone(battle.trigger_gaps({"gaps": "no"}))
        t = battle.trigger_gaps({"gaps": [{"type": "aircraft", "volume": 50, "yield": 5},
                                          {"type": "train", "volume": 10, "yield": -3}, {"type": "zeppelin"}]})
        self.assertEqual(t["air"], {"gap": 55, "volume": 50, "yield": 5, "ours": 0, "rival": 0})
        self.assertEqual(t["rail"]["gap"], 7)


class Levers(unittest.TestCase):
    def test_gene_mode_tokens_and_overrides(self):
        ev = battle.evolve_cfg({})
        self.assertEqual(battle.gene_mode("air_routes_max", ev), "air")
        self.assertEqual(battle.gene_mode("max_routes", ev), "road")
        self.assertEqual(battle.gene_mode("own_lines", ev), "rail")
        self.assertEqual(battle.gene_mode("cash_reserve", ev), "capital")
        self.assertEqual(battle.gene_mode("cash_reserve", dict(ev, gene_modes={"cash_reserve": "rail"})), "rail")

    def test_spearman_with_ties_and_a_constant(self):
        self.assertAlmostEqual(battle.spearman([1, 2, 3], [10, 20, 30]), 1.0)
        self.assertAlmostEqual(battle.spearman([1, 1, 2, 2], [4, 3, 2, 1]), -0.8944, places=3)
        self.assertEqual(battle.spearman([1, 1, 1], [1, 2, 3]), 0.0)

    def test_lever_evidence_reads_the_ratio_to_the_reference_once_per_seed(self):
        t = {"space": {"line_budget": [10000, 200000, 10000], "own_lines": [0, 24, 2]},
             "defaults": {"own_lines": 18, "line_budget": 50000}, "start": {}}
        e = battle.lever_evidence(history(), t, "AAAHogEx")
        self.assertEqual(e["line_budget"]["n"], 48)   # 4 configurations x 12 seeds, the copy skipped
        self.assertGreater(e["line_budget"]["rho"], 0.9)
        self.assertEqual(e["own_lines"], {"rho": 0.0, "n": 48, "values": 1})
        self.assertEqual(battle.lever_evidence([], t, "AAAHogEx")["own_lines"]["n"], 0)

    def test_candidates_put_the_losing_mode_first_and_try_an_undirected_gene_both_ways(self):
        ev = ev_override(levers=3)
        ev["mode_tokens"] = battle.evolve_cfg({})["mode_tokens"]
        space = {"line_budget": [10000, 200000, 10000], "own_lines": [0, 24, 2], "aggressiveness": [0, 100, 10],
                 "air_planes": [1, 6, 1], "intercity": [0, 1, 1]}
        evidence = {"line_budget": {"rho": 0.3, "n": 50, "values": 5}, "aggressiveness": {"rho": -0.6, "n": 50, "values": 5},
                    "own_lines": {"rho": 0.01, "n": 50, "values": 5}, "air_planes": {"rho": 0.9, "n": 50, "values": 5},
                    "intercity": {"rho": 0.2, "n": 50, "values": 2}}
        base = {"line_budget": 30000, "own_lines": 4, "aggressiveness": 0, "air_planes": 5, "intercity": 1}
        c = battle.evolve_candidates(base, space, evidence, ["rail"], ev)
        self.assertEqual([x["changed"] for x in c], [{"line_budget": [30000, 80000]}, {"own_lines": [4, 10]},
                                                    {"own_lines": [4, 0]}])   # aggressiveness 0 cannot go down
        self.assertIn("no recorded direction", c[1]["why"])
        ev["levers"] = 5
        c = battle.evolve_candidates(dict(base, aggressiveness=50), space, evidence, ["rail"], ev)
        self.assertEqual(c[-1], {"name": "combo", "changed": {"line_budget": [30000, 80000],
                                                              "aggressiveness": [50, 30]},
                                 "why": "every directed move together"})
        self.assertEqual(battle.apply_changes({"a": 1, "b": 2}, {"a": [1, 5]}), "a=5,b=2")


class Pairs(unittest.TestCase):
    def test_a_pair_favours_only_beyond_the_reference_spread(self):
        p = battle.pair_of(1, row("baseline", 1000), row("c1", 1100, rival=1_050_000), "AAAHogEx")
        self.assertEqual(p["spread"], 0.05)
        self.assertTrue(p["favours"])
        p = battle.pair_of(1, row("baseline", 1000), row("c1", 1040, rival=1_050_000), "AAAHogEx")
        self.assertFalse(p["favours"])
        p = battle.pair_of(1, row("baseline", 1000), row("c1", 2000, errors=1), "AAAHogEx")
        self.assertFalse(p["favours"])
        self.assertEqual(battle.rival_of({"standings": []}, "AAAHogEx"), 0)

    def test_judge(self):
        ev = ev_override()
        good = [{"seed": s, "favours": True, "ours": 1.2} for s in (1, 2, 3)]
        self.assertTrue(battle.judge(good, ev)[0])
        self.assertIn("2 pair(s)", battle.judge(good[:2], ev)[1])
        self.assertIn("seed(s) 2", battle.judge([good[0], dict(good[1], favours=False), good[2]], ev)[1])
        self.assertIn("under min_gain", battle.judge([dict(p, ours=1.01) for p in good], ev)[1])
        self.assertEqual(battle.geo([]), 0.0)


class Guard(unittest.TestCase):
    def test_lock_owner_file_and_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            g = battle.EvolveGuard(root, 4, 1, {})
            self.assertIsNone(g.acquire())
            owner = (root / "evolve" / "lock" / "OWNER").read_text()
            self.assertIn(f"pid={os.getpid()}", owner)
            self.assertIn("how_to_stop=create", owner)
            self.assertIn("held:", battle.EvolveGuard(root, 4, 1, {}).acquire())
            g.release()
            self.assertFalse((root / "evolve" / "lock").exists())
            g.release()                                   # nothing left to release

    def test_a_dead_owner_is_taken_over_and_an_unreadable_one_is_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            lock = root / "evolve" / "lock"
            lock.mkdir(parents=True)
            self.assertIn("held:", battle.EvolveGuard(root, 4, 1, {}).acquire())
            dead = subprocess.run([sys.executable, "-c", "import os; print(os.getpid())"],
                                  capture_output=True, text=True).stdout.strip()
            (lock / "OWNER").write_text(f"host={battle.socket.gethostname()}\npid={dead}\n")
            err = io.StringIO()
            with redirect_stdout(err), redirect_stderr(err):
                self.assertIsNone(battle.EvolveGuard(root, 4, 1, {}).acquire())
            self.assertIn(f"dead pid {dead}", err.getvalue())

    def test_admit_stops_on_stop_pause_clock_budget_and_floor(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            now = [1000.0]
            g = battle.EvolveGuard(root, 3, 1, {}, clock=lambda: now[0])
            self.assertEqual(g.admit(5), (3, ""))
            (root / "PAUSED").write_text("need the cores\n")
            self.assertEqual(g.admit(1), (0, "PAUSED: need the cores"))
            (root / "PAUSED").write_text("")
            self.assertIn("PAUSED: ", g.admit(1)[1])
            (root / "PAUSED").unlink()
            (root / "STOP").write_text("owner says stop\n")
            self.assertEqual(g.admit(1), (0, "STOP: owner says stop"))
            (root / "STOP").unlink()
            g.used = 3
            self.assertEqual(g.admit(1), (0, "match budget 3 spent"))
            g.used = 0
            now[0] += 61
            self.assertEqual(g.admit(1), (0, "wall-clock cap reached"))
            floor = battle.EvolveGuard(root, 3, 1, {"min_free_disk_gib": 10 ** 9})
            n, why = floor.admit(1)
            self.assertEqual(n, 0)
            self.assertIn("floor", why)
            real = battle.os.getloadavg

            def broken():
                raise OSError("no load")
            battle.os.getloadavg = broken
            try:
                self.assertEqual(battle.EvolveGuard(root, 3, 1, {"max_load_per_core": 1.0}).admit(2), (2, ""))
            finally:
                battle.os.getloadavg = real

    def test_batches_split_to_the_admitted_size_reuse_rows_and_report_why(self):
        with tempfile.TemporaryDirectory() as tmp:
            g = battle.EvolveGuard(Path(tmp), 4, 1, {})
            calls = []
            rows, why = battle.evolve_batches(fake_run(lambda s, x: 1, calls=calls), g, [1, 2], ["a=", "b=", "c="],
                                              have={1: {"a": row("a", 1)}})
            self.assertEqual(why, "match budget 4 spent")
            self.assertEqual(calls, [(1, ["b", "c"]), (2, ["a", "b"])])
            self.assertEqual(sorted(rows[1]), ["a", "b", "c"])
            g2 = battle.EvolveGuard(Path(tmp), 5, 1, {})
            self.assertEqual(battle.evolve_batches(lambda s, v: None, g2, [1], ["a="])[1],
                             "NOT RUN: needs build/openttd and a prepared arena cache")


class Panel:
    """A local stand-in for a panel: GET /api/rhai/shared?key=, GET /api/bots, POST /api/rhai/shared."""

    def __init__(self, shared, bots=None):
        self.shared, self.bots, self.posted = shared, bots or {}, []
        panel = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def reply(self, code, body):
                data = json.dumps(body).encode()
                self.send_response(code)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                if self.path.startswith("/api/rhai/shared?key="):
                    key = battle.urllib.parse.unquote(self.path.split("=", 1)[1])
                    if key in panel.shared:
                        return self.reply(200, {"key": key, "value": panel.shared[key]})
                    return self.reply(404, {"error": "absent"})
                if self.path == "/api/bots":
                    return self.reply(200, panel.bots)
                return self.reply(500, {"error": "boom"})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["content-length"])))
                panel.posted.append((self.headers.get("origin"), body))
                self.reply(200, {"ok": True})
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class Trigger(unittest.TestCase):
    def test_a_live_record_fires_when_live_entries_judge_it_lost(self):
        ev = battle.evolve_cfg({})
        tc = tuning()
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(live_record(Path(tmp), 0.02))
            t = battle.evolve_trigger(args(live=str(f)), ev, tc)
            self.assertTrue(t["fired"])
            self.assertEqual((t["kind"], t["source"], t["baseline"]), ("live", "live-9-1", BASE))
            self.assertTrue(t["why"].startswith("lost: 0.020 of AAAHogEx at 1951-12"), t["why"])
            f = Path(live_record(Path(tmp), 0.9))
            t = battle.evolve_trigger(args(live=str(f)), ev, tc)
            self.assertFalse(t["fired"])
            self.assertTrue(t["why"].startswith("not lost:"))
            t = battle.evolve_trigger(args(live=str(f), company=3), ev, tc)
            self.assertIn("not a PrismaticAI company", t["why"])
            t = battle.evolve_trigger(args(live=str(f), force=True, baseline="a=1"), ev, tc)
            self.assertTrue(t["fired"])
            self.assertEqual(t["baseline"], "a=1")
            self.assertIn("forced", t["why"])
        t = battle.evolve_trigger(args(force=True), ev, tc)
        self.assertEqual(t["why"], "forced (--force)")

    def test_a_panel_fires_on_the_gap_level_or_a_review_alarm_and_reads_the_running_settings(self):
        ev = battle.evolve_cfg({})
        panel = Panel({"gap.0": {"level": 3, "flags": {"losing_ground": "x"}, "levers": [{"name": "max_routes"}, 7]},
                       "review.0.month": {"alarms": ["share"], "gaps": [{"type": "aircraft", "volume": 9, "yield": 1}]},
                       "gap.1": {"level": 1}},
                      bots={"running": [{"id": 0, "settings": [{"name": "own_lines", "value": 4}, {"name": "x"}]}]})
        try:
            t = battle.evolve_trigger(args(panel=panel.url), ev, None)
            self.assertTrue(t["fired"])
            self.assertEqual(t["levers"], ["max_routes"])
            self.assertEqual(t["baseline"], "own_lines=4")
            self.assertEqual(t["gaps"]["air"]["volume"], 9)
            self.assertIn("gap.0 level 3 (losing_ground)", t["why"])
            t = battle.evolve_trigger(args(panel=panel.url, company=1), ev, None)
            self.assertFalse(t["fired"])
            self.assertEqual(t["why"], "gap.1 level 1, review.1.month absent")
            t = battle.evolve_trigger(args(panel=panel.url, company=2, baseline="a=1"), ev, None)
            self.assertEqual(t["why"], "gap.2 level absent, review.2.month absent")
            panel.shared["review.2.month"] = {"alarms": []}
            self.assertIn("without alarms", battle.evolve_trigger(args(panel=panel.url, company=2), ev, None)["why"])
            with self.assertRaises(urllib.error.HTTPError):
                battle.http_json(panel.url + "/nowhere")
            self.assertEqual(battle.http_json(panel.url + "/api/rhai/shared", {"key": "k"}), {"ok": True})
            self.assertEqual(panel.posted[-1], (panel.url, {"key": "k"}))
        finally:
            panel.close()


class Evolve(unittest.TestCase):
    """cmd_evolve end to end with a fake `run`: every verdict the loop can reach."""

    def go(self, cfg, a, run, records=None, live=()):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = battle.cmd_evolve(cfg, a, run=run, records=history() if records is None else records,
                                     live=list(live))
        recs = sorted((cfg["cache"] / "evolve").glob("evolve-*.json"))
        return code, out.getvalue() + err.getvalue(), json.loads(recs[-1].read_text()) if recs else None

    def test_a_line_budget_that_wins_everywhere_is_proposed_and_written_to_the_blackboard(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            panel = Panel({})
            gain = lambda seed, s: 1.0 + (s.get("line_budget", 30000) - 30000) / 100000
            try:
                code, out, rec = self.go(cfg, args(force=True, baseline=BASE, propose=panel.url), fake_run(gain))
            finally:
                panel.close()
            self.assertEqual(code, 0, out)
            self.assertEqual(rec["status"], "proposed", out)
            self.assertEqual(rec["winner"]["changed"], {"line_budget": [30000, 70000]})
            self.assertIn("line_budget=70000", rec["winner"]["settings"])
            self.assertNotIn("fr_from", rec["binds"]["baseline"])
            self.assertEqual(rec["attribution"]["modes"], ["rail"])
            self.assertEqual(rec["matches"], 3 + 3 * len(rec["candidates"]) + 6)
            self.assertNotIn("_rows", rec)
            self.assertTrue(rec["proposal_sha"])
            key, value = panel.posted[0][1]["key"], panel.posted[0][1]["value"]
            self.assertEqual((key, panel.posted[0][1]["writer"]), ("evolve.proposal.0", "evolve"))
            self.assertEqual(value["id"], rec["id"])
            self.assertFalse((Path(tmp) / "evolve" / "lock").exists())
            # The same trigger again within the cooldown runs nothing.
            self.go(cfg, args(baseline=BASE, live=live_record(Path(tmp), 0.01)), fake_run(gain))
            code, out, _ = self.go(cfg, args(baseline=BASE, live=live_record(Path(tmp), 0.01)), fake_run(gain))
            self.assertIn("cooldown", out)

    def test_a_lost_live_configuration_is_vetoed_and_a_paired_live_winner_joins(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            tc = tuning(cfg)
            base = battle.live_genes(tc, battle.settings_dict(BASE))
            lost = {"record": "live-1-1", "company": 0, "lost": True, "why": "0.010 of AAAHogEx",
                    "genes": dict(base, line_budget=70000)}
            seed = {"record": "live-2-1", "company": 2, "lost": False, "paired": True, "ratio": 0.9,
                    "genes": dict(base, own_lines=20)}
            gain = lambda s, x: 1.2 if x.get("own_lines") == 20 else 1.0
            code, out, rec = self.go(cfg, args(force=True, baseline=BASE), fake_run(gain), live=[lost, seed])
            self.assertEqual(rec["vetoed"][0]["changed"], {"line_budget": [30000, 70000]})
            self.assertEqual(rec["vetoed"][0]["by"], "live-1-1:0")
            live1 = next(c for c in rec["candidates"] if c["name"] == "live1")
            self.assertEqual(live1["changed"], {"own_lines": [4, 20]})
            self.assertEqual(rec["status"], "proposed", rec["verdict"])
            self.assertEqual(rec["winner"]["name"], "live1")

    def test_a_trigger_that_did_not_fire_or_has_no_baseline_runs_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            code, out, rec = self.go(cfg, args(), fake_run(lambda s, x: 1))
            self.assertEqual((code, rec), (0, None))
            self.assertIn("no trigger fired", out)
            code, out, rec = self.go(cfg, args(force=True), fake_run(lambda s, x: 1))
            self.assertEqual(code, 2)
            self.assertIn("no baseline", out)

    def test_another_evolve_holding_the_lock_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            lock = Path(tmp) / "evolve" / "lock"
            lock.mkdir(parents=True)
            (lock / "OWNER").write_text(f"host={battle.socket.gethostname()}\npid={os.getpid()}\n")
            code, out, _ = self.go(cfg, args(force=True, baseline=BASE), fake_run(lambda s, x: 1))
            self.assertEqual(code, 10)
            self.assertIn("another evolve runs", out)

    def test_no_winner_is_inconclusive_and_nothing_is_proposed(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            panel = Panel({})
            try:
                code, out, rec = self.go(cfg, args(force=True, baseline=BASE, propose=panel.url),
                                         fake_run(lambda s, x: 1.0))
            finally:
                panel.close()
            self.assertEqual((code, rec["status"]), (0, "inconclusive"))
            self.assertIn("no candidate favoured on every screening seed", rec["verdict"])
            self.assertEqual(panel.posted[0][1]["key"], "evolve.status.0")

    def test_a_screening_winner_that_fails_its_confirmation_seeds_is_not_proposed(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            gain = lambda seed, s: (1.5 if s.get("line_budget", 30000) > 30000 and seed < 4 else 1.0)
            code, out, rec = self.go(cfg, args(force=True, baseline=BASE), fake_run(gain))
            self.assertEqual(rec["status"], "inconclusive")
            self.assertIn("won the screening but not the confirmation seeds", rec["verdict"])

    def test_budget_and_stop_leave_an_incomplete_record(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            gain = lambda seed, s: 1.0 + (s.get("line_budget", 30000) - 30000) / 100000
            code, out, rec = self.go(cfg, args(force=True, baseline=BASE, budget=2), fake_run(gain))
            self.assertEqual(rec["status"], "incomplete")
            self.assertEqual(rec["verdict"], "baseline: match budget 2 spent")
        for at, verdict in ((4, "screening: STOP: owner"), (7, "confirmation of c1: STOP: owner")):
            with tempfile.TemporaryDirectory() as tmp:
                cfg = config(Path(tmp))
                calls = []

                def run(seed, vs, tmp=tmp, at=at, calls=calls):
                    calls.append(seed)
                    if len(calls) == at:
                        (Path(tmp) / "STOP").write_text("owner\n")
                    return fake_run(gain)(seed, vs)
                code, out, rec = self.go(cfg, args(force=True, baseline=BASE), run)
                self.assertEqual((rec["status"], rec["verdict"]), ("incomplete", verdict))
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            code, out, rec = self.go(cfg, args(force=True, baseline=BASE, budget=3), fake_run(gain))
            self.assertEqual((rec["status"], rec["candidates"]), ("inconclusive", []))
            self.assertIn("no candidate for modes", rec["verdict"])

    def test_write_records_every_played_seed_and_skips_one_never_played(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            results = Path(tmp) / "results"
            results.mkdir()
            old, real_git = battle.RESULTS_DIR, battle.git
            battle.RESULTS_DIR = results
            battle.git = lambda *a, **k: "0" * 40 if a[0] == "rev-parse" else ""
            try:
                code, out, rec = self.go(cfg, args(force=True, baseline=BASE, budget=2, write=True),
                                         fake_run(lambda s, x: 1.0))
            finally:
                battle.RESULTS_DIR, battle.git = old, real_git
            self.assertEqual(rec["status"], "incomplete")
            self.assertEqual(sorted(p.name.split("-seed")[1] for p in results.glob("ablation-*.json")),
                             ["1.json", "2.json"])          # seed 3 was never played

    def test_an_invalid_ablation_record_is_written_with_a_warning(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = battle.RESULTS_DIR
            battle.RESULTS_DIR = Path(tmp)
            out = io.StringIO()
            try:
                rows = [dict(row("baseline", 1000), vs_first=0.0)]
                rec = {"schema": battle.ABLATION_SCHEMA, "id": "ablation-x-seed1", "generated_at": "now", "seed": 1,
                       "years": 1, "field": ["AAAHogEx"], "slot": 0, "match_cfg": config(Path(tmp))["match"], "rev": "not-a-hash",
                       "tree_dirty": False, "rows": rows}
                with redirect_stdout(out), redirect_stderr(out):
                    battle.write_ablation(rec)
            finally:
                battle.RESULTS_DIR = old
            self.assertIn("warning: ablation-x-seed1.json: rev is not a full commit hash", out.getvalue())
            self.assertTrue((Path(tmp) / "ablation-x-seed1.md").is_file())

    def test_no_binary_is_not_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            code, out, rec = self.go(cfg, args(force=True, baseline=BASE, binary=str(Path(tmp) / "none")), None)
            self.assertEqual((code, rec["status"]), (12, "not_run"))

    def test_trigger_gaps_override_the_headless_attribution_and_history_is_read_from_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = config(Path(tmp))
            results = Path(tmp) / "results"
            results.mkdir()
            (results / "battle-x.json").write_text(json.dumps(history()[0]))
            panel = Panel({"review.0.month": {"alarms": ["share"], "gaps": [{"type": "aircraft", "volume": 9}]}},
                          bots={"running": []})
            old = battle.RESULTS_DIR
            battle.RESULTS_DIR = results
            try:
                out = io.StringIO()
                with redirect_stdout(out), redirect_stderr(out):
                    battle.cmd_evolve(cfg, args(panel=panel.url, baseline=BASE), run=fake_run(lambda s, x: 1.0))
            finally:
                battle.RESULTS_DIR = old
                panel.close()
            rec = json.loads(sorted((Path(tmp) / "evolve").glob("evolve-*.json"))[-1].read_text())
            self.assertEqual(rec["attribution"]["source"], "trigger")
            self.assertEqual(rec["attribution"]["modes"], ["air"])
            self.assertTrue(all(c["changed"].keys() <= {"air_planes", "aggressiveness"} for c in rec["candidates"]))


def fake_game(repo: Path, body: str) -> Path:
    fake = repo / "build" / "openttd"
    fake.parent.mkdir(parents=True, exist_ok=True)
    fake.write_text("#!/bin/sh\n" + body)
    fake.chmod(0o755)
    return fake


def arena(cache: Path) -> None:
    a = cache / "arena"
    (a / "baseset").mkdir(parents=True)
    (a / "game").mkdir(parents=True)
    (a / "ai" / "library").mkdir(parents=True)
    (a / "ai" / "AAAHogEx.tar").write_bytes(b"")


# The fake game reads its own start script: PrismaticAI's value is line_budget / 10, AAAHogEx's fixed.
GAME = r"""S="$HOME/Documents/OpenTTD/scripts/game_start.scr"
B=$(grep -o 'line_budget=[0-9]*' "$S" | head -1 | cut -d= -f2)
V=$(( ${B:-10000} / 10 ))
echo "[x] dbg: [script:1] [18] [E] REF Q date=1950-1 cid=0 value=$V money=1 vehicles=1 stations=1 rail=1 road=0 air=0 water=0 prail=500 proad=0 pair=0 pwater=0 name=P"
echo "[x] dbg: [script:1] [18] [E] REF Q date=1950-1 cid=1 value=100000 money=1 vehicles=9 stations=1 rail=9 road=0 air=2 water=0 prail=90000 proad=0 pair=8000 pwater=0 name=H"
echo "[x] dbg: [script:1] [0] [S] squirrel error"
echo "pid $TTD_BATTLE_ORCHESTRATOR" > "$HOME/owner.txt"
"""


class AblateRows(unittest.TestCase):
    """ablate_rows against a fake game binary: standings per company, errors of PrismaticAI's own slot,
    the owner marker in the environment, a killed match on timeout, and cmd_ablate's NOT RUN."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        self.repo, self.cache = t / "repo", t / "cache"
        (self.repo / PRISMATIC).mkdir(parents=True)
        arena(self.cache)
        self.cfg = config(self.cache)
        self.old = battle.REPO, battle.RESULTS_DIR
        battle.REPO, battle.RESULTS_DIR = self.repo, t / "results"
        battle.RESULTS_DIR.mkdir()

    def tearDown(self):
        battle.REPO, battle.RESULTS_DIR = self.old
        self.tmp.cleanup()

    def test_rows_carry_standings_and_the_slot_errors(self):
        fake_game(self.repo, GAME)
        rows = battle.ablate_rows(self.cfg, 1, 1, ["a=line_budget=30000", "b=line_budget=90000"], ["AAAHogEx"],
                                  log_dir=str(Path(self.tmp.name) / "logs"), owner="evolve:h:1")
        self.assertEqual([r["value"] for r in rows], [3000, 9000])
        self.assertEqual(rows[0]["errors"], 1)            # the Squirrel error is company 0's
        s = rows[1]["standings"]
        self.assertEqual([(x["company"], x["key"]) for x in s], [(0, "b"), (1, "AAAHogEx")])
        self.assertEqual((s[1]["rail"], s[1]["prail"], s[1]["pair"]), (9, 90000, 8000))
        self.assertTrue((Path(self.tmp.name) / "logs" / "a.log").is_file())
        rows = battle.ablate_rows(self.cfg, 1, 1, ["a=line_budget=30000"], ["AAAHogEx"], slot=1)
        self.assertEqual(rows[0]["value"], 100000)        # slot 1: PrismaticAI is company 1
        self.assertEqual(rows[0]["errors"], 0)
        self.assertEqual(rows[0]["standings"][0]["errors"], 1)

    def test_a_match_past_its_timeout_is_killed(self):
        fake_game(self.repo, "sleep 30\n")
        t0 = time.time()
        with self.assertRaises(subprocess.TimeoutExpired):
            battle.ablate_rows(self.cfg, 1, 1, ["a=", "b="], ["AAAHogEx"], timeout=1)
        self.assertLess(time.time() - t0, 20)

    def test_cmd_ablate_prints_writes_and_says_not_run(self):
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(out):
            self.assertEqual(battle.cmd_ablate(self.cfg, 1, 1, ["a="], ["AAAHogEx"], False), 12)
        self.assertIn("NOT RUN", out.getvalue())
        fake_game(self.repo, GAME)
        real_git = battle.git
        battle.git = lambda *a, **k: "0" * 40 if a[0] == "rev-parse" else ""
        try:
            with redirect_stdout(out), redirect_stderr(out):
                self.assertEqual(battle.cmd_ablate(self.cfg, 1, 1, ["a=line_budget=30000", "b=line_budget=60000"],
                                                   ["AAAHogEx"], True), 0)
        finally:
            battle.git = real_git
        self.assertIn("+100.0%", out.getvalue())
        rec = json.loads(next(battle.RESULTS_DIR.glob("ablation-*.json")).read_text())
        self.assertEqual(battle.validate_ablation(rec, next(battle.RESULTS_DIR.glob("ablation-*.json")).name), [])
        self.assertEqual(rec["rows"][1]["standings"][1]["key"], "AAAHogEx")

    def test_evolve_end_to_end_through_the_fake_game_writes_ablation_records(self):
        fake_game(self.repo, GAME)
        real_git = battle.git
        battle.git = lambda *a, **k: "0" * 40 if a[0] == "rev-parse" else ""
        out = io.StringIO()
        try:
            with redirect_stdout(out), redirect_stderr(out):
                code = battle.cmd_evolve(self.cfg, args(force=True, baseline=BASE, write=True), records=history())
        finally:
            battle.git = real_git
        self.assertEqual(code, 0, out.getvalue())
        rec = json.loads(next((self.cache / "evolve").glob("evolve-*.json")).read_text())
        self.assertEqual(rec["status"], "proposed", out.getvalue())
        self.assertEqual(rec["winner"]["changed"], {"line_budget": [30000, 70000]})
        self.assertEqual(len(list(battle.RESULTS_DIR.glob("ablation-*.json"))), 6)
        self.assertIn("wrote", out.getvalue())

    def test_main_dispatches_evolve(self):
        cfgf = Path(self.tmp.name) / "c.json"
        raw = json.loads(battle.DEFAULT_CONFIG.read_text())
        raw.update(cache=str(self.cache), results=str(self.cache), hosts_file=str(self.cache / "h.json"),
                   baseset=str(self.cache))
        raw["tuning"]["enabled"] = False
        cfgf.write_text(json.dumps(raw))
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(out):
            self.assertEqual(battle.main(["evolve", "--config", str(cfgf)]), 0)
        self.assertIn("no trigger fired", out.getvalue())


if __name__ == "__main__":
    unittest.main()
