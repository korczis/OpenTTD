#!/usr/bin/env python3
"""RESEARCH-ONLY. Offline tests for tools/research/ai-battle/battle.py: no game, no SSH, no arena.

Run: python3 -B tools/research/test_ai_battle.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/gate.sh change does).

Covered: the referee line parser and company-id mapping, the match status contract (exit 0 and the
final-year report), scoring and its exclusions, the replay comparison, the console start script
(double quotes, settings), tick arithmetic, the record validator, the derived Markdown and README
region, the ablation validator and its derived page with charts, the tar export that strips a
directory's parents, the run guards (probe parsing on macOS, Linux and garbage, the PAUSED marker,
the STOP file, the resource floor, the ORCHESTRATOR file and the match's owner marker) and the
records a commit can hold (git index in a hook).
"""
import importlib.util
import io
import json
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("battle", HERE / "ai-battle" / "battle.py")
battle = importlib.util.module_from_spec(spec)
sys.modules["battle"] = battle
spec.loader.exec_module(battle)

KEYS = ["A", "B C", "D"]
MC = {"years": 2, "start_year": 1960, "landscape": "temperate", "map_x": 8, "map_y": 8}


def ref(y, mo, cid, value, veh=1, name="X"):
    return (f"[2026-11-19 05:00:00] dbg: [script:1] [18] [E] REF Q date={y}-{mo} cid={cid} "
            f"value={value} money=5 income=7 perf=9 vehicles={veh} stations=2 name={name}")


def match(mid, host, seed, values, status="done", replica_of=None):
    lineup = [{"key": k, "ai": k} for k in KEYS]
    series = {k: [{"t": "1962-01", "value": v, "vehicles": 1, "stations": 1, "income": 0}]
              for k, v in zip(KEYS, values)}
    m = {"id": mid, "host": host, "host_version": "v1", "seed": seed, "years": 2,
         "ticks": battle.match_ticks(2), "lineup": lineup, "series": series, "errors": {},
         "status": status, "replica_of": replica_of, "match_cfg": MC}
    m["standings"] = battle.standings_of(m)
    return m


class ParseLog(unittest.TestCase):
    def test_reads_ref_lines_errors_and_keeps_last_duplicate(self):
        text = "\n".join([
            ref(1960, 1, 0, 0), ref(1960, 1, 1, 0, name="Two Words Co"),
            ref(1960, 4, 0, 10), ref(1960, 4, 0, 11),   # a repeated quarter: the last one wins
            "dbg: [script:1] [1] [S] Your script made an error: the index 'x' does not exist",
            "dbg: [script:1] [1] [I] chatter that is not a referee line",
        ])
        series, errors = battle.parse_log(text)
        self.assertEqual([s["t"] for s in series[0]], ["1960-01", "1960-04"])
        self.assertEqual(series[0][-1]["value"], 11)
        self.assertEqual(series[1][0]["name"], "Two Words Co")
        self.assertEqual(errors, {1: 1})


def shared(y, mo, ind, owners, name="Coal Mine"):
    return (f"[2026-11-22 20:00:00] dbg: [script:1] [18] [E] REF S date={y}-{mo} ind={ind} "
            f"owners={owners} name={name}")


class SharedProducers(unittest.TestCase):
    """The referee's REF S lines and the fitness failure of two PrismaticAI companies at one
    producer (research/incident-2026-11-22-sibling-industries.md)."""

    def test_parse_unions_owners_and_keeps_the_first_quarter(self):
        text = "\n".join([ref(1960, 4, 0, 1), shared(1960, 4, 7, "0,2", "Gundhead Coal Mine"),
                          shared(1960, 7, 7, "0,1,2", "Gundhead Coal Mine"), shared(1961, 1, 9, "1,3"),
                          "dbg: REF S date=1961-1 ind=x owners= name=broken"])
        got = battle.parse_shared(text)
        self.assertEqual(sorted(got), [7, 9])
        self.assertEqual(got[7], {"name": "Gundhead Coal Mine", "owners": {0, 1, 2}, "first": "1960-04",
                                  "cargoes": {None: {"owners": {0, 1, 2}, "first": "1960-04"}}})
        self.assertEqual(got[9]["owners"], {1, 3})
        self.assertEqual(battle.parse_shared("no referee here"), {})

    def test_parse_keeps_each_cargo_apart(self):
        # Since 2026-11-23 the referee names the cargo: one line per producer and cargo.
        text = "\n".join([shared(1965, 4, 7, "0,2", "Punthill Farm").replace(" owners=", " cargo=6 owners="),
                          shared(1965, 7, 7, "1,2", "Punthill Farm").replace(" owners=", " cargo=4 owners=")])
        got = battle.parse_shared(text)[7]
        self.assertEqual(got["owners"], {0, 1, 2})
        self.assertEqual(got["cargoes"], {6: {"owners": {0, 2}, "first": "1965-04"},
                                          4: {"owners": {1, 2}, "first": "1965-07"}})

    def lineup(self):
        return [{"ai": "PrismaticAI", "key": "P a"}, {"ai": "PrismaticAI", "key": "P b"},
                {"ai": "AAAHogEx", "key": "AAAHogEx"}]

    def test_two_prismatic_companies_at_one_producer_fail_a_rival_does_not(self):
        m = {"lineup": self.lineup(), "shared": {7: {"name": "Coal Mine", "owners": {0, 1, 2}, "first": "1960-04"},
                                                9: {"name": "Forest", "owners": {0, 2}, "first": "1961-01"}}}
        self.assertEqual(battle.sibling_producers(m),
                         ["shared_producer:Coal Mine (industry 7, from 1960-04): P a + P b"])
        self.assertEqual(battle.sibling_producers({"lineup": self.lineup()}), [])
        fit = {"rules": {}, "must_be_healthy": []}
        m.update(status="done", errors={}, series={"P a": [], "P b": [], "AAAHogEx": []})
        self.assertIn("shared_producer:Coal Mine (industry 7, from 1960-04): P a + P b",
                      battle.fitness_verdict(m, fit))

    def test_two_prismatic_companies_on_different_cargoes_of_one_farm_do_not_fail(self):
        # Livestock for one, grain for the other: nothing of the farm's output is split.
        farm = {"name": "Farm", "owners": {0, 1}, "first": "1965-04",
                "cargoes": {4: {"owners": {0, 2}, "first": "1965-04"}, 6: {"owners": {1, 2}, "first": "1965-04"}}}
        self.assertEqual(battle.sibling_producers({"lineup": self.lineup(), "shared": {7: farm}}), [])
        farm["cargoes"][6]["owners"] = {0, 1}
        self.assertEqual(battle.sibling_producers({"lineup": self.lineup(), "shared": {7: farm}}),
                         ["shared_producer:Farm (industry 7, cargo 6, from 1965-04): P a + P b"])

    def test_the_fitness_config_plays_the_claim_race(self):
        # Two PrismaticAI companies, the lower one held back by test_claim_race, so the higher one
        # builds and checks its source station first and only the second claim check (rail.nut
        # ClaimBlocks) settles the race; shared_producer judges it like every lineup.
        cfg = json.loads((Path(battle.__file__).parent / "battle.json").read_text())
        race = [lu for lu in cfg["fitness"]["lineups"]
                if any(c.get("label") == "PrismaticAI claim race A" for c in lu)]
        self.assertEqual(len(race), 1)
        ours = [c for c in race[0] if c["ai"] == "PrismaticAI"]
        self.assertEqual([c["label"] for c in ours], ["PrismaticAI claim race A", "PrismaticAI claim race B"])
        self.assertIn("test_claim_race=", ours[0]["settings"])
        self.assertNotIn("test_claim_race=", ours[1]["settings"])
        # Otherwise the same bot, so both choose the same producer first.
        self.assertEqual(ours[0]["settings"].replace(",test_claim_race=20", ""), ours[1]["settings"])


def trainless(y, mo, cid, rail, ids):
    return (f"dbg: [script] REF T date={y}-{mo} cid={cid} rail_stations={rail} trainless={len(ids)} "
            f"ids={','.join(map(str, ids))}")


class TrainlessStations(unittest.TestCase):
    """The referee's REF T lines and the fitness failure of a PrismaticAI company that keeps the same
    rail stations without a train of its own for a year (research/incident-2026-11-23-trainless-lines.md)."""

    def test_parse_keeps_report_order_and_station_ids_per_company(self):
        text = "\n".join([trainless(1960, 4, 0, 4, [3, 7]), ref(1960, 4, 0, 1), trainless(1960, 4, 1, 9, []),
                          trainless(1960, 7, 0, 6, [3, 7, 9]),
                          "dbg: REF T date=1960-7 cid=x rail_stations=1 trainless=1 ids=1"])
        got = battle.parse_trainless(text)
        self.assertEqual(sorted(got), [0, 1])
        self.assertEqual(got[0], [{"date": "1960-04", "rail": 4, "trainless": 2, "ids": {3, 7}},
                                  {"date": "1960-07", "rail": 6, "trainless": 3, "ids": {3, 7, 9}}])
        self.assertEqual(got[1][0]["ids"], set())
        self.assertEqual(battle.parse_trainless("no referee here"), {})

    def lineup(self):
        return [{"ai": "PrismaticAI", "key": "P"}, {"ai": "AAAHogEx", "key": "AAAHogEx"},
                {"ai": "PrismaticAI", "key": "P2"}]

    def reports(self, *ids):
        return [{"date": f"{1960 + i // 4}-{3 * (i % 4) + 1:02d}", "rail": 12, "trainless": len(s), "ids": set(s)}
                for i, s in enumerate(ids)]

    def test_the_same_station_trainless_for_a_year_fails_a_line_being_laid_does_not(self):
        kept = self.reports([1], [5, 6], [5, 6, 8], [5, 6], [5, 6, 9])
        # The live game's pattern: two termini of a finished line keep no train. P2's stations are
        # those of lines being laid, a different pair every few quarters, as on the fitness seed.
        laying = self.reports([170, 171], [170, 171], [170, 171], [192, 193], [192, 193])
        m = {"lineup": self.lineup(), "trainless": {0: kept, 1: self.reports([4], [4], [4], [4]), 2: laying}}
        want = "P: trainless_stations:2 rail station(s) without a train from 1960-04 to 1961-01 (stations 5, 6; max 0)"
        self.assertEqual(battle.trainless_stations(m, {}), [want])
        self.assertEqual(battle.trainless_stations(m, {"trainless_max": 2}), [])
        # The test hook's own lineup keeps stations trainless on purpose and is not judged here.
        self.assertEqual(battle.trainless_stations(m, {"trainless_skip": ["P"]}), [])
        self.assertEqual(battle.trainless_stations({"lineup": self.lineup(), "trainless": {0: kept[:3]}}, {}), [])
        # Fewer reports than a year: not judged; a shorter window from the rules: judged on it.
        self.assertEqual(battle.trainless_stations({"lineup": self.lineup(), "trainless": {0: kept[1:4]}},
                                                   {"trainless_reports": 3})[0].split(":")[1], " trainless_stations")
        self.assertEqual(battle.trainless_stations({"lineup": self.lineup()}, {}), [])
        fit = {"rules": {}, "must_be_healthy": []}
        m.update(status="done", errors={}, series={"P": [], "AAAHogEx": [], "P2": []})
        self.assertIn(want, battle.fitness_verdict(m, fit))

    def test_the_fitness_config_declares_the_rule(self):
        cfg = json.loads((Path(battle.__file__).parent / "battle.json").read_text())
        self.assertEqual((cfg["fitness"]["rules"]["trainless_max"], cfg["fitness"]["rules"]["trainless_reports"]), (0, 4))
        # The recovery's own lineup is held to run trains, and its settings switch the test hook on.
        self.assertEqual(cfg["fitness"]["must_run_trains"], ["PrismaticAI trainless test"])
        self.assertEqual(cfg["fitness"]["rules"]["trainless_skip"], ["PrismaticAI trainless test"])
        test = [c for lu in cfg["fitness"]["lineups"] for c in lu if c.get("label") == "PrismaticAI trainless test"]
        self.assertEqual(len(test), 1)
        self.assertIn("test_trainless=2", test[0]["settings"])

    def test_a_contestant_held_to_run_trains_fails_without_one(self):
        lineup = [{"ai": "PrismaticAI", "key": "T"}, {"ai": "PrismaticAI", "key": "P"}]
        fit = {"rules": {}, "must_be_healthy": [], "must_run_trains": ["T"]}
        m = {"lineup": lineup, "status": "done", "errors": {},
             "series": {"T": [{"t": "1960-01", "value": 5, "vehicles": 3, "money": 0, "stations": 1, "rail": 3}, {"t": "1961-01", "value": 6, "vehicles": 3, "money": 0, "stations": 1, "rail": 0}],
                        "P": [{"t": "1961-01", "value": 7, "vehicles": 1, "money": 0, "stations": 1, "rail": 0}]}}
        self.assertIn("T: no_trains: no train at the end of the match", battle.fitness_verdict(m, fit))
        self.assertFalse(any(f.startswith("P: no_trains") for f in battle.fitness_verdict(m, fit)))
        m["series"]["T"][-1]["rail"] = 2
        self.assertFalse(any("no_trains" in f for f in battle.fitness_verdict(m, fit)))
        m["series"]["T"] = []
        self.assertIn("T: no_trains: no train at the end of the match", battle.fitness_verdict(m, fit))


def capital(year, q, cid, frozen, days, **more):
    """One PrismaticAI capital report line as capital.nut logs it (a warning, script level 3)."""
    extra = "".join(f" {k}={v}" for k, v in more.items())
    return (f"dbg: [script:3] [{cid}] [W] PrismaticAI: capital report {year}Q{q}: frozen={frozen} days={days}"
            f" calls=4 bought=1 spent=30000 holds=0 idle=0 money=12000 loan=0 keep=15000 level=3{extra}")


class FrozenConstruction(unittest.TestCase):
    """capital.nut's quarterly report and the fitness failure of a healthy PrismaticAI company whose
    construction stood frozen on too many days of its first years (the live game of seed 12609:
    525 of 1,096 days in 1950-1952)."""

    def test_parse_keeps_report_order_and_fields_per_company(self):
        text = "\n".join([capital(1960, 1, 0, 10, 91), ref(1960, 4, 0, 1), capital(1960, 1, 2, 0, 50),
                          capital(1960, 2, 0, 0, 91, extra=5),
                          "dbg: [script:3] [0] [I] PrismaticAI: capital report 1960Q3: frozen=91 days=91",
                          "dbg: [script:3] [0] [W] PrismaticAI: capital: no vehicle more on route 1,2,"])
        got = battle.parse_capital(text)
        self.assertEqual(sorted(got), [0, 2])
        self.assertEqual([(r["year"], r["quarter"], r["frozen"], r["days"]) for r in got[0]],
                         [(1960, 1, 10, 91), (1960, 2, 0, 91)])
        self.assertEqual((got[0][1]["extra"], got[0][0]["level"], got[0][0]["spent"]), (5, 3, 30000))
        self.assertEqual(battle.parse_capital("no reports"), {})

    def match(self, reports):
        return {"lineup": [{"ai": "PrismaticAI", "key": "P"}, {"ai": "AAAHogEx", "key": "AAAHogEx"},
                           {"ai": "PrismaticAI", "key": "P2"}],
                "match_cfg": {"start_year": 1950}, "capital": reports}

    def reports(self, frozen):
        return [{"year": 1950 + i // 4, "quarter": i % 4 + 1, "frozen": f, "days": 91} for i, f in enumerate(frozen)]

    def test_more_than_the_limit_of_the_first_years_frozen_fails(self):
        fit = {"rules": {}, "must_be_healthy": ["P", "P2"]}
        # The live game's shape: 163 days in 1950, half of 1951, 170 days in 1952; then nothing.
        live = self.reports([0, 72, 91, 0, 91, 75, 0, 15, 91, 79, 0, 8] + [91] * 8)
        got = battle.frozen_construction(self.match({0: live, 2: self.reports([0] * 12)}), fit)
        self.assertEqual(got, ["P: frozen:522 of 1092 days (47.8%) in 1950-1952 (max 15%)"])
        # Only the first frozen_years count: the frozen 1953-1954 quarters above did not.
        late = self.reports([0] * 12 + [91] * 8)
        self.assertEqual(battle.frozen_construction(self.match({0: late}), fit), [])
        # At the limit passes; the limit and the window come from the rules.
        self.assertEqual(battle.frozen_construction(self.match({0: self.reports([0, 0, 0, 54])}), fit), [])
        self.assertEqual(len(battle.frozen_construction(self.match({0: self.reports([0, 0, 0, 55])}), fit)), 1)
        loose = {"rules": {"frozen_max": 0.5, "frozen_years": 1}, "must_be_healthy": ["P"]}
        self.assertEqual(battle.frozen_construction(self.match({0: live}), loose), [])
        self.assertEqual(battle.frozen_construction(self.match({0: self.reports([91] * 4)}), loose),
                         ["P: frozen:364 of 364 days (100.0%) in 1950-1950 (max 50%)"])

    def test_only_healthy_contestants_with_reports_are_judged(self):
        frozen = self.reports([91] * 4)
        self.assertEqual(battle.frozen_construction(self.match({0: frozen}), {"rules": {}, "must_be_healthy": []}), [])
        self.assertEqual(battle.frozen_construction(self.match({}), {"rules": {}, "must_be_healthy": ["P"]}), [])
        self.assertEqual(battle.frozen_construction(self.match({1: frozen}), {"rules": {}, "must_be_healthy": ["AAAHogEx"]}), [])
        m = self.match({0: frozen})
        m.update(status="done", errors={}, series={"P": [], "AAAHogEx": [], "P2": []})
        self.assertIn("P: frozen:364 of 364 days (100.0%) in 1950-1952 (max 15%)",
                      battle.fitness_verdict(m, {"rules": {}, "must_be_healthy": ["P"]}))


class FitnessRun(unittest.TestCase):
    """cmd_fitness end to end against a fake game binary that prints referee lines: the shared
    producer is printed and fails the check."""

    def test_fake_match_with_two_prismatic_companies_on_one_mine_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "repo"
            (repo / "build").mkdir(parents=True)
            (repo / "ai" / "PrismaticAI").mkdir(parents=True)
            (repo / "referee" / "BattleReferee").mkdir(parents=True)
            lines = [ref(1960, 1, c, 10, veh=1) for c in range(3)] + \
                    [ref(1961, 1, c, 20 + c, veh=2) for c in range(3)] + [shared(1960, 4, 7, "0,1")] + \
                    [trainless(1960, 1, 1, 4, [5, 6]), trainless(1960, 4, 1, 4, [5, 6]), trainless(1960, 7, 1, 4, [5, 6]),
                     trainless(1961, 1, 1, 4, [5, 6, 7]), trainless(1961, 1, 2, 8, [1, 2, 3]),
                     capital(1960, 1, 0, 30, 91), capital(1960, 2, 0, 0, 91)]
            fake = repo / "build" / "openttd"
            fake.write_text("#!/bin/sh\ncat <<'EOF'\n" + "\n".join(lines) + "\nEOF\n")
            fake.chmod(0o755)
            arena = tmp / "cache" / "arena"
            (arena / "baseset").mkdir(parents=True)
            (arena / "ai" / "library").mkdir(parents=True)
            (arena / "ai" / "AAAHogEx.tar").write_bytes(b"")
            cfg = {"cache": tmp / "cache", "prismatic": {"path": "ai/PrismaticAI"},
                   "referee": {"path": "referee/BattleReferee"},
                   "match": {"years": 1, "start_year": 1960, "landscape": "temperate", "map_x": 8, "map_y": 8},
                   "fitness": {"years": 1, "seed": 1, "rules": {}, "must_be_healthy": [],
                               "lineups": [[{"ai": "PrismaticAI", "label": "P a"},
                                            {"ai": "PrismaticAI", "label": "P b"}, {"ai": "AAAHogEx"}]]}}
            old = battle.REPO
            battle.REPO = repo
            out, err = io.StringIO(), io.StringIO()
            try:
                with redirect_stdout(out), redirect_stderr(err):
                    code = battle.cmd_fitness(cfg)
            finally:
                battle.REPO = old
        self.assertEqual(code, 10, out.getvalue() + err.getvalue())
        self.assertIn("shared producer Coal Mine (industry 7) from 1960-04: P a, P b", out.getvalue())
        self.assertIn("shared_producer:Coal Mine (industry 7, from 1960-04): P a + P b", err.getvalue())
        self.assertIn("P b                        rail stations   4, without a train   3 (1961-01)", out.getvalue())
        self.assertIn("P b: trainless_stations:2 rail station(s) without a train from 1960-01 to 1961-01 "
                      "(stations 5, 6; max 0)", err.getvalue())
        self.assertNotIn("AAAHogEx: trainless", err.getvalue())
        self.assertIn("P a                        construction frozen 30 of 182 days in its first 3 years", out.getvalue())


def funnel(year, cid, mode, fields, level="E"):
    return f"[2026-11-24 10:00:00] dbg: [script:1] [{cid}] [{level}] FUNNEL year={year} mode={mode} {fields}"


RAIL_ROW = ("hb=40 short_hb=3 idle_hb=0 searches=12 found=2 producers=40 ours=3 claimed=20 small=6 "
            "served=5 no_acceptor=8 pairs_tried=4 tried=4 stations=2 path_fail=1 laid=1 built=1 "
            "single_track=1 trainless=0 loco_refused_cash=0 trained=1 lines=3 paying=2 trains=5 trains_added=2")


class GrowthFunnel(unittest.TestCase):
    """PrismaticAI's yearly FUNNEL lines (PrismaticAI/funnel.nut): parsed per company, the stage that
    lost most, the fitness failure of a missing year, the post-mortem, the fitness print and the
    orchestrator's poll (research/growth-funnel.md)."""

    def test_parse_keeps_the_last_line_of_a_year_and_mode_and_only_error_level(self):
        text = "\n".join([funnel(1951, 0, "rail", "tried=1"), funnel(1951, 0, "rail", RAIL_ROW),
                          funnel(1951, 0, "road", "tried=3 stop_fail=2 built=1 vehicles=2 paying=1"),
                          funnel(1950, 2, "air", "short=4 tried=1 built=1"),
                          funnel(1952, 0, "rail", "tried=9", level="I"),
                          "dbg: [script:1] [0] [E] FUNNEL year=x mode=rail tried=1"])
        got = battle.parse_funnel(text)
        self.assertEqual(sorted(got), [0, 2])
        self.assertEqual([(r["year"], r["mode"]) for r in got[0]], [(1951, "rail"), (1951, "road")])
        self.assertEqual((got[0][0]["claimed"], got[0][0]["trains_added"]), (20, 2))
        self.assertEqual(got[2], [{"year": 1950, "mode": "air", "short": 4, "tried": 1, "built": 1}])

    def test_most_lost_is_the_largest_share_and_none_without_a_loss(self):
        row = {"year": 1951, "mode": "rail"}
        row.update({k: int(v) for k, v in battle.REF_FIELD.findall(RAIL_ROW)})
        # Per population: producers, tries, lines. Stations lost 2 of 4 tries ties path 1 of 2
        # stations at a half; the tie goes to the larger loss.
        self.assertEqual(battle.funnel_most_lost(row), [("discovery", "claimed", 20, 40),
                                                        ("build", "stations", 2, 4), ("run", "paying", 1, 3)])
        row.update(claimed=0, ours=0, served=0, no_acceptor=0, lines=0, paying=0)
        self.assertEqual(battle.funnel_most_lost(row), [("build", "stations", 2, 4)])
        self.assertEqual(battle.funnel_most_lost({"year": 1951, "mode": "air", "tried": 0}), [])
        self.assertEqual(battle.funnel_most_lost({"year": 1951, "mode": "unknown", "tried": 5}), [])
        text = battle.funnel_text({"year": 1951, "mode": "road", "tried": 4, "stop_fail": 3})
        self.assertEqual(text, "road 1951: tried=4 stop_fail=3; most lost: build stop 3/4")
        self.assertEqual(battle.funnel_text({"year": 1951, "mode": "ship", "vehicles": 0}),
                         "ship 1951: vehicles=0; most lost: none")

    def test_a_held_contestant_owes_a_rail_line_for_every_whole_year(self):
        m = {"lineup": [{"key": "P", "ai": "PrismaticAI"}, {"key": "Q", "ai": "PrismaticAI"}],
             "years": 3, "match_cfg": {"start_year": 1950}, "status": "done", "errors": {},
             "series": {"P": [], "Q": []},
             "funnel": {"P": [{"year": 1950, "mode": "rail"}, {"year": 1951, "mode": "road"}]}}
        self.assertEqual(battle.funnel_gaps(m, ["P"]), ["P: no_funnel: no FUNNEL rail line for 1951"])
        self.assertEqual(battle.funnel_gaps(m, []), [])
        fit = {"rules": {}, "must_be_healthy": [], "must_funnel": ["P", "Q"]}
        fails = battle.fitness_verdict(m, fit)
        self.assertIn("P: no_funnel: no FUNNEL rail line for 1951", fails)
        self.assertIn("Q: no_funnel: no FUNNEL rail line for 1950, 1951", fails)

    def test_postmortem_carries_the_last_funnel_year_of_each_mode(self):
        lineup = [{"key": "W", "ai": "W"}, {"key": "P", "ai": "PrismaticAI"}]
        m = {"lineup": lineup, "errors": {}, "series": {
            "W": [{"t": "1952-01", "value": 1000, "vehicles": 10, "income": 10}],
            "P": [{"t": "1952-01", "value": 100, "vehicles": 10, "income": 10}]},
            "funnel": {"P": [{"year": 1950, "mode": "rail", "tried": 9, "stations": 0},
                             {"year": 1951, "mode": "rail", "tried": 2, "stations": 1, "laid": 1, "built": 1}]}}
        m["standings"] = battle.standings_of(m)
        pm = battle.postmortem(m)
        self.assertIn("funnel rail 1951: tried=2 stations=1 laid=1 built=1; most lost: build stations 1/2", pm[0]["findings"])
        self.assertFalse(any("1950" in f for f in pm[0]["findings"]))

    def test_fitness_prints_each_funnel_line_and_fails_a_missing_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "repo"
            (repo / "build").mkdir(parents=True)
            (repo / "ai" / "PrismaticAI").mkdir(parents=True)
            (repo / "referee" / "BattleReferee").mkdir(parents=True)
            lines = [ref(1960, 1, c, 10, veh=1) for c in range(3)] + \
                    [ref(1962, 1, c, 20 + c, veh=2) for c in range(3)] + \
                    [funnel(1960, 0, "rail", "tried=2 stations=1 laid=1 built=1"), funnel(1960, 0, "road", "tried=1 built=1"),
                     funnel(1961, 0, "rail", "tried=0"), funnel(1960, 1, "rail", "tried=1"),
                     funnel(1960, 9, "rail", "tried=1")]
            fake = repo / "build" / "openttd"
            fake.write_text("#!/bin/sh\ncat <<'EOF'\n" + "\n".join(lines) + "\nEOF\n")
            fake.chmod(0o755)
            arena = tmp / "cache" / "arena"
            (arena / "baseset").mkdir(parents=True)
            (arena / "ai" / "library").mkdir(parents=True)
            (arena / "ai" / "AAAHogEx.tar").write_bytes(b"")
            cfg = {"cache": tmp / "cache", "prismatic": {"path": "ai/PrismaticAI"},
                   "referee": {"path": "referee/BattleReferee"},
                   "match": {"years": 2, "start_year": 1960, "landscape": "temperate", "map_x": 8, "map_y": 8},
                   "fitness": {"years": 2, "seed": 1, "rules": {}, "must_be_healthy": [],
                               "must_funnel": ["P a", "P b"],
                               "lineups": [[{"ai": "PrismaticAI", "label": "P a"},
                                            {"ai": "PrismaticAI", "label": "P b"}, {"ai": "AAAHogEx"}]]}}
            old = battle.REPO
            battle.REPO = repo
            out, err = io.StringIO(), io.StringIO()
            try:
                with redirect_stdout(out), redirect_stderr(err):
                    code = battle.cmd_fitness(cfg)
            finally:
                battle.REPO = old
        self.assertEqual(code, 0, out.getvalue() + err.getvalue())
        self.assertIn("P a                        funnel rail 1960: tried=2 stations=1 laid=1 built=1; most lost: build stations 1/2",
                      out.getvalue())
        self.assertIn("P a                        funnel road 1960: tried=1 built=1; most lost: none", out.getvalue())
        self.assertIn("P b                        funnel rail 1960: tried=1; most lost: build stations 1/1", out.getvalue())
        # Company 9 is not in the lineup; 1961, the last year, is not owed by a match of two years.
        self.assertNotIn("tried=1; most lost: build stations 1/1\nfitness[0]: P b", out.getvalue())
        self.assertIn("fitness: PASS", out.getvalue())

    def test_the_orchestrator_poll_reads_funnel_lines_into_the_match(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            orch, host = Guards.orchestrator(tmp)
            host.state = "online"
            m = {"id": "m-0001", "host": "h", "edition": "e1", "status": "running", "started_at": 0.0, "seed": 1,
                 "ticks": 1,
                 "lineup": [{"key": "A", "ai": "PrismaticAI"}, {"key": "B", "ai": "B"}], "years": 2,
                 "match_cfg": MC, "series": {}, "errors": {}}
            orch.matches = {"m-0001": m}
            body = "\n".join([ref(1960, 1, 0, 5), ref(1960, 1, 1, 6), funnel(1960, 0, "rail", "tried=3 stations=1")])
            seen = []

            def sh(script, timeout=60):
                seen.append(script)
                return subprocess.CompletedProcess(script, 0, f"@@@ m-0001\nRUNNING\n{body}\n", "")
            host.sh = sh
            with mock.patch.object(battle, "log"):
                orch.poll_host(host)
        self.assertIn("\\] \\[E\\] FUNNEL ", seen[0])
        self.assertEqual(m["funnel"], {"A": [{"year": 1960, "mode": "rail", "tried": 3, "stations": 1}]})


    def test_a_healthy_prismatic_that_stops_expanding_fails_with_a_named_regime(self):
        """research/failure-regimes.md: PrismaticAI keeps 10 vehicles for three years while both
        rivals grow and pull away; the check names the regime, prints it, and fails."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "repo"
            (repo / "build").mkdir(parents=True)
            (repo / "ai" / "PrismaticAI").mkdir(parents=True)
            (repo / "referee" / "BattleReferee").mkdir(parents=True)
            lines = []
            for q in range(13):
                y, mo = 1960 + q // 4, 3 * (q % 4) + 1
                lines += [ref(y, mo, 0, 1000 + q, veh=10), ref(y, mo, 1, 2000 * (q + 1), veh=5 + 2 * q),
                          ref(y, mo, 2, 1500 + 100 * q, veh=5 + q)]
            fake = repo / "build" / "openttd"
            fake.write_text("#!/bin/sh\ncat <<'EOF'\n" + "\n".join(lines) + "\nEOF\n")
            fake.chmod(0o755)
            arena = tmp / "cache" / "arena"
            (arena / "baseset").mkdir(parents=True)
            (arena / "ai" / "library").mkdir(parents=True)
            (arena / "ai" / "AAAHogEx.tar").write_bytes(b"")
            cfg = {"cache": tmp / "cache", "prismatic": {"path": "ai/PrismaticAI"},
                   "referee": {"path": "referee/BattleReferee"},
                   "match": {"years": 3, "start_year": 1960, "landscape": "temperate", "map_x": 8, "map_y": 8},
                   "fitness": {"years": 3, "seed": 1, "rules": {}, "must_be_healthy": ["PrismaticAI"],
                               "lineups": [[{"ai": "PrismaticAI"}, {"ai": "AAAHogEx"}, {"ai": "AdmiralAI"}]]}}
            old = battle.REPO
            battle.REPO = repo
            out, err = io.StringIO(), io.StringIO()
            try:
                with redirect_stdout(out), redirect_stderr(err):
                    code = battle.cmd_fitness(cfg)
            finally:
                battle.REPO = old
        self.assertEqual(code, 10, out.getvalue() + err.getvalue())
        self.assertIn(f"fitness[0]: {'PrismaticAI':26} regime no_expansion: 1960-01..1963-01 "
                      "(known 1962-01): fleet 10->10", out.getvalue())
        self.assertIn("FAIL [0] PrismaticAI: regime:no_expansion: 1960-01..1963-01", err.getvalue())
        self.assertNotIn("AAAHogEx: regime", err.getvalue())


class MatchContract(unittest.TestCase):
    def test_done_needs_exit_zero_and_final_year(self):
        m = match("m-0001", "h", 1, [3, 2, 1])
        self.assertEqual(battle.match_status(m, 0), "done")
        self.assertEqual(battle.match_status(m, 137), "incomplete")
        m["series"] = {k: [dict(s[0], t="1961-10")] for k, s in m["series"].items()}
        self.assertEqual(battle.match_status(m, 0), "incomplete")

    def test_ticks_cover_the_final_january_report(self):
        self.assertEqual(battle.match_ticks(20), int((20 * 365.25 + 6) * 74))
        self.assertGreater(battle.match_ticks(1), 365 * 74 + 2 * 74)

    def test_start_script_quotes_names_and_passes_settings(self):
        text = battle.start_script_text([{"ai": "LuDiAI AfterFix"},
                                         {"ai": "PrismaticAI", "settings": "a=1,b=2"}])
        self.assertEqual(text, 'start_ai "LuDiAI AfterFix"\nstart_ai "PrismaticAI" a=1,b=2\n')

    def test_a_start_script_reads_back_into_the_lineup_an_ablate_replays(self):
        lineup = [{"ai": "AdmiralAI"}, {"ai": "PrismaticAI", "settings": "a=1,b=2"},
                  {"ai": "LuDiAI AfterFix"}]
        field, slot, settings = battle.parse_start_script(battle.start_script_text(lineup))
        self.assertEqual((field, slot, settings), (["AdmiralAI", "LuDiAI AfterFix"], 1, "a=1,b=2"))

    def test_a_match_record_gives_the_inputs_to_replay_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp) / "abcdef0123" / "matches"
            d.mkdir(parents=True)
            (d / "m-0009.json").write_text(json.dumps({
                "seed": 7, "years": 20, "host": "h", "host_version": "v",
                "lineup": [{"ai": "CivilAI", "key": "CivilAI"},
                           {"ai": "PrismaticAI", "key": "PrismaticAI GA g1.0", "settings": "a=1"},
                           {"ai": "AAAHogEx", "key": "AAAHogEx"}]}))
            r = battle.match_replay_inputs(Path(tmp), "abcd/m-0009")
            self.assertEqual((r["seed"], r["field"], r["slot"], r["settings"], r["key"]),
                             (7, ["CivilAI", "AAAHogEx"], 1, "a=1", "PrismaticAI GA g1.0"))
            with self.assertRaises(SystemExit):
                battle.match_replay_inputs(Path(tmp), "ffff/m-0009")

    def test_launch_uses_an_edition_directory_and_refuses_a_leftover(self):
        host = battle.Host({"name": "h", "ssh": None}, "$HOME/ttd-battle")
        m = {"id": "m-0001", "edition": "abc", "lineup": [{"ai": "A"}], "ticks": 7, "seed": 3}
        text = battle.launch_script(host, m, MC)
        self.assertIn("test ! -e $HOME/ttd-battle/matches/abc/m-0001\n", text)
        self.assertIn("null:ticks=7", text)
        self.assertIn("-G 3", text)

    def test_grouped_contestants_take_turns_one_per_match(self):
        cs = [{"key": "P1", "group": "P"}, {"key": "P2", "group": "P"}, {"key": "A"}, {"key": "B"}]
        seen = set()
        for n in range(4):
            lu = [c["key"] for c in battle.lineup_for(cs, n)]
            self.assertEqual(len(lu), 3)
            self.assertEqual(sum(k.startswith("P") for k in lu), 1)
            self.assertTrue({"A", "B"} <= set(lu))
            seen |= {k for k in lu if k.startswith("P")}
        self.assertEqual(seen, {"P1", "P2"})

    def test_every_member_of_a_group_starts_from_the_same_slot_on_one_seed(self):
        cs = [{"key": "P1", "group": "P"}, {"key": "P2", "group": "P"}, {"key": "P3", "group": "P"},
              {"key": "A"}, {"key": "B"}]
        slots = {}
        for n in range(12):
            lu = [c["key"] for c in battle.lineup_for(cs, n, 3)]
            p = next(i for i, k in enumerate(lu) if k.startswith("P"))
            slots.setdefault(n // 3, set()).add(p)
        self.assertTrue(all(len(v) == 1 for v in slots.values()))
        self.assertGreater(len({next(iter(v)) for v in slots.values()}), 1)  # still rotates across seeds

    def test_reap_keeps_the_current_edition_and_finished_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            for ed, mid, finished in (("cur", "m-0001", False), ("old", "m-0001", False), ("old", "m-0002", True)):
                d = Path(tmp) / "matches" / ed / mid
                d.mkdir(parents=True)
                proc = subprocess.Popen(["sleep", "30"])
                (d / "pid").write_text(str(proc.pid))
                if finished:
                    (d / "exit_code").write_text("0")
                procs = getattr(self, "_procs", [])
                procs.append((ed, mid, proc))
                self._procs = procs
            out = subprocess.run(["/bin/sh", "-c", battle.reap_script(tmp, "cur")], capture_output=True,
                                 text=True).stdout.strip()
            self.assertEqual(out, "reaped 1")
            # A signal lands asynchronously: give a killed process a moment to be reaped before polling
            # (under full CPU load an immediate poll() still saw it alive).
            for ed, mid, p in self._procs:
                try:
                    p.wait(timeout=5 if (ed, mid) == ("old", "m-0001") else 0.1)
                except subprocess.TimeoutExpired:
                    pass
            alive = {(ed, mid): p.poll() is None for ed, mid, p in self._procs}
            self.assertEqual(alive, {("cur", "m-0001"): True, ("old", "m-0001"): False, ("old", "m-0002"): True})
            for _, _, p in self._procs:
                p.kill()

    def test_contestant_key_prefers_label(self):
        self.assertEqual(battle.contestant_key({"ai": "P", "settings": "x=1"}), "P[x=1]")
        self.assertEqual(battle.contestant_key({"ai": "P", "label": "P max"}), "P max")


class HostShell(unittest.TestCase):
    def test_an_unanswering_host_times_out_into_an_exit_code_not_an_exception(self):
        host = battle.Host({"name": "h", "ssh": None}, "$HOME/ttd-battle")
        r = host.try_sh("sleep 5", timeout=1)
        self.assertEqual(r.returncode, 124)
        self.assertIn("timed out", r.stderr)
        self.assertEqual(host.try_sh("echo ok").stdout.strip(), "ok")


MAC_PROBE = """VERSION OpenTTD 20261119-master-g9750b3f946
NCPU 18
LOAD { 62.23 50.52 45.42 }
SWAP total = 37888.00M  used = 35840.50M  free = 2047.50M  (encrypted)
DISK /dev/disk3s5   971298980 874618684  29954288    97%    /System/Volumes/Data
"""

LINUX_PROBE = """VERSION OpenTTD 15.0
NCPU 8
LOAD 0.52 1.58 0.59 1/389 12345
MEM MemTotal:       16314248 kB
MEM MemAvailable:   12000000 kB
MEM SwapTotal:       2097148 kB
MEM SwapFree:        2097148 kB
DISK /dev/nvme0n1p2  490617784 104857600 360710144  23% /home
"""


class Guards(unittest.TestCase):
    """The run guards: pause marker, STOP file, resource floor and traceable ownership."""
    LIMITS = {"max_load_per_core": 1.0, "min_free_disk_gib": 35, "max_swap_used_gib": 16}

    def test_probe_parses_a_macos_sample(self):
        res = battle.parse_probe(MAC_PROBE)
        self.assertEqual(res["version"], "20261119-master-g9750b3f946")
        self.assertEqual((res["ncpu"], res["load"]), (18, [62.23, 50.52, 45.42]))
        self.assertEqual((res["disk_free_kib"], res["disk_mount"]), (29954288, "/System/Volumes/Data"))
        self.assertAlmostEqual(res["swap_used_mib"], 35840.5)
        self.assertEqual((res["paused"], res["unparsed"]), (None, []))
        self.assertEqual(battle.probe_state(res, 0, "", "b")[0], "online")

    def test_probe_parses_a_linux_sample_and_uptime_fallbacks(self):
        res = battle.parse_probe(LINUX_PROBE)
        self.assertEqual((res["ncpu"], res["load"][:2]), (8, [0.52, 1.58]))
        self.assertEqual((res["disk_free_kib"], res["disk_mount"]), (360710144, "/home"))
        self.assertEqual((res["mem_avail_kib"], res["swap_used_mib"]), (12000000, 0))
        self.assertEqual(res["unparsed"], [])
        for line, want in (("LOAD 10:01  up 3 days,  2 users,  load average: 0.52, 0.58, 0.59", 0.52),
                           ("LOAD  6:09  up 13 days, 21 users, load averages: 62.23 50.52 45.42", 62.23),
                           ("LOAD 10:01 up 3 days, 2 users, load average: 0,52, 0,58, 0,59", 0.52)):
            self.assertEqual(battle.parse_probe(line)["load"][0], want, line)

    def test_garbage_probe_output_blocks_nothing_but_is_reported(self):
        res = battle.parse_probe("NCPU lots\nLOAD n/a\nDISK df: no such file\nwhat is this\n")
        self.assertEqual((res["ncpu"], res["load"], res["disk_free_kib"]), (None, None, None))
        self.assertEqual(len(res["unparsed"]), 4)
        self.assertEqual(battle.floor_decision(None, res["ncpu"], None, 3, self.LIMITS), (3, ""))
        state, detail = battle.probe_state(res, 0, "", "b")   # no build line: not handed matches
        self.assertEqual(state, "error")
        self.assertIn("not understood", detail)
        self.assertIn("no NCPU line", battle.parse_probe("VERSION OpenTTD x\n")["unparsed"])

    def test_a_pause_marker_pauses_the_host_with_its_first_line(self):
        res = battle.parse_probe(MAC_PROBE + "PAUSED /h/ttd-battle/PAUSED\nPAUSEDMSG need the cores\n")
        self.assertEqual(battle.probe_state(res, 0, "", "b"), ("paused", "need the cores"))
        res = battle.parse_probe("NO_BINARY\nPAUSED /h/ttd-battle/PAUSED.txt\nPAUSEDMSG \n")
        self.assertEqual(battle.probe_state(res, 0, "", "b"), ("paused", "/h/ttd-battle/PAUSED.txt present"))
        host = battle.Host({"name": "h", "ssh": None}, "$HOME/ttd-battle")
        host.state, host.res = "paused", res
        self.assertEqual(host.admit(0), 0)

    def test_the_probe_script_finds_a_pause_file_in_the_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            host = battle.Host({"name": "h", "ssh": None, "binary": f"{tmp}/none"}, tmp)
            Path(tmp, "PAUSED-by-alice").write_text("Stop ttd-battle\nsecond line\n")
            out = subprocess.run(["/bin/sh", "-c", battle.probe_script(host.binary, tmp)],
                                 capture_output=True, text=True)
            self.assertEqual(out.returncode, 0)
            res = battle.parse_probe(out.stdout)
            self.assertEqual(res["paused"], "Stop ttd-battle")
            self.assertTrue(res["no_binary"])
            self.assertIsNotNone(res["disk_free_kib"])   # df of the root works on this machine

    def test_floor_decision(self):
        gib = battle.GIB_KIB
        fd = battle.floor_decision
        self.assertEqual(fd(2.0, 18, 100 * gib, 4, self.LIMITS), (4, ""))
        n, why = fd(86.0, 18, 100 * gib, 4, self.LIMITS)
        self.assertEqual(n, 0)
        self.assertIn("load 86.0 on 18 cores = 4.78/core > ceiling 1.0", why)
        n, why = fd(15.5, 18, 100 * gib, 4, self.LIMITS)       # room for 2 more under 18.0
        self.assertEqual(n, 2)
        self.assertIn("room for 2", why)
        self.assertEqual(fd(17.5, 18, 100 * gib, 4, self.LIMITS)[0], 0)
        n, why = fd(1.0, 18, 30 * gib, 4, self.LIMITS)
        self.assertEqual((n, why), (0, "free disk 30.0 GiB < floor 35 GiB"))
        n, why = fd(1.0, 18, 100 * gib, 4, self.LIMITS, swap_used_mib=35 * 1024)
        self.assertEqual((n, why), (0, "swap 35.0 GiB used > ceiling 16 GiB"))
        self.assertEqual(fd(None, None, None, 2, self.LIMITS), (2, ""))   # unknown: never blocks
        self.assertEqual(fd(2.0, 18, 100 * gib, 0, self.LIMITS), (0, ""))  # full, not busy
        self.assertEqual(fd(3.0, 4, 100 * gib, 2, dict(self.LIMITS, max_load_per_core=2.0)), (2, ""))

    def test_host_floor_overrides_and_busy_state(self):
        host = battle.Host({"name": "h", "ssh": None, "slots": 32,
                            "resource_floor": {"max_load_per_core": 0.5}},
                           "$HOME/ttd-battle", {"min_free_disk_gib": 50})
        self.assertEqual(host.limits, {"max_load_per_core": 0.5, "min_free_disk_gib": 50,
                                       "max_swap_used_gib": 16})
        host.state, host.res = "online", battle.parse_probe(MAC_PROBE)
        self.assertEqual(host.admit(0), 0)
        self.assertEqual(host.state, "busy")
        self.assertIn("free disk 28.6 GiB < floor 50 GiB", host.detail)

    def test_orchestrator_file_names_owner_and_how_to_stop(self):
        text = battle.orchestrator_text("mac one.local", 4242, "2026-11-22T05:00:00+00:00", "c" * 40,
                                        "a" * 40, "ed1", "$HOME/ttd-battle", "/Users/x/ttd-battle")
        lines = dict(l.split("=", 1) for l in text.splitlines() if "=" in l and not l.startswith("#"))
        self.assertEqual((lines["host"], lines["pid"], lines["commit"]), ("mac one.local", "4242", "c" * 40))
        self.assertEqual(lines["started_at"], "2026-11-22T05:00:00+00:00")
        self.assertEqual(lines["marker"], "TTD_BATTLE_ORCHESTRATOR=mac_one.local:4242")
        self.assertIn("$HOME/ttd-battle/PAUSED", lines["how_to_stop"])
        self.assertIn("SIGTERM pid 4242 on host mac one.local", lines["how_to_stop"])
        self.assertIn("/Users/x/ttd-battle/STOP", lines["how_to_stop"])
        self.assertNotIn("\nEOS\n", text)   # it goes through a quoted here-document

    def test_claim_writes_the_orchestrator_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            host = battle.Host({"name": "h", "ssh": None}, f"{tmp}/root")
            text = battle.orchestrator_text("h", 1, "t", "c", "a", "e", host.root, tmp)
            self.assertTrue(host.claim(text))
            self.assertEqual(Path(tmp, "root", "ORCHESTRATOR").read_text(), text)

    def test_match_launch_carries_the_orchestrator_marker(self):
        host = battle.Host({"name": "h", "ssh": None}, "$HOME/ttd-battle")
        m = {"id": "m-0001", "edition": "abc", "lineup": [{"ai": "A"}], "ticks": 7, "seed": 3}
        text = battle.launch_script(host, m, MC, battle.owner_marker("mac.local", 99))
        self.assertIn("export TTD_BATTLE_ORCHESTRATOR=mac.local:99\n", text)
        self.assertIn('echo "$TTD_BATTLE_ORCHESTRATOR" > orchestrator', text)
        self.assertIn("'ttd-battle[mac.local:99]'", text)
        self.assertEqual(battle.owner_marker("a b;rm", 1), "a_b_rm:1")

    def test_stop_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(battle.stop_requested(Path(tmp)))
            Path(tmp, "STOP").write_text("")
            self.assertEqual(battle.stop_requested(Path(tmp)), f"{Path(tmp) / 'STOP'} present")
            Path(tmp, "STOP").write_text("owner: stop ttd-battle\n")
            self.assertEqual(battle.stop_requested(Path(tmp)), "owner: stop ttd-battle")

    @staticmethod
    def orchestrator(tmp):
        cache = Path(tmp)
        (cache / "arena").mkdir()
        (cache / "arena" / "PROVENANCE.json").write_text(json.dumps({"rev": "a" * 40}))
        cfg = {"cache": cache, "results": cache / "results", "hosts_file": cache / "hosts.json",
               "remote_root": f"{tmp}/remote", "seed": "s", "match": MC,
               "contestants": [{"ai": "A", "label": "A"}, {"ai": "B", "label": "B"}]}
        (cache / "hosts.json").write_text(json.dumps({"hosts": [{"name": "h", "ssh": None, "slots": 3}]}))
        orch = battle.Orchestrator(cfg, edition="e1")
        host = orch.hosts[0]
        host.synced_digest = orch.digest            # nothing to rsync
        host.launched = []

        def probe():
            host.state, host.detail, host.res = "online", "", battle.parse_probe(LINUX_PROBE)

        def try_sh(script, timeout=60):
            host.launched.append(script)
            return subprocess.CompletedProcess(script, 0, "launched\n", "")
        host.probe, host.try_sh = probe, try_sh
        return orch, host

    def test_a_stop_file_stops_scheduling_everywhere(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            orch, host = self.orchestrator(tmp)
            Path(tmp, "STOP").write_text("owner says stop\n")
            with mock.patch.object(battle, "log"):
                orch.schedule_pass()
            self.assertEqual((orch.matches, host.launched), ({}, []))
            self.assertTrue(any("STOP" in e["msg"] and "owner says stop" in e["msg"] for e in orch.events))
            Path(tmp, "STOP").unlink()
            with mock.patch.object(battle, "log"):
                orch.schedule_pass()
            self.assertEqual(len(orch.matches), 3)       # 3 slots, load 1.58 of 8 cores
            self.assertTrue(host.launched[0].startswith("mkdir -p"))   # ORCHESTRATOR first
            self.assertIn(f"TTD_BATTLE_ORCHESTRATOR={orch.owner}", host.launched[1])
            self.assertTrue(any("removed: scheduling resumes" in e["msg"] for e in orch.events))

    def test_a_loaded_host_is_busy_and_gets_no_match(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            orch, host = self.orchestrator(tmp)
            loaded = MAC_PROBE.replace("29954288", "99954288").replace("used = 35840.50M", "used = 0.00M")
            host.probe = lambda: setattr(host, "res", battle.parse_probe(loaded)) or setattr(
                host, "state", "online")
            with mock.patch.object(battle, "log"):
                orch.schedule_pass()
            self.assertEqual((host.state, orch.matches, host.launched), ("busy", {}, []))
            self.assertIn("62.2 on 18 cores", host.detail)


def init_repo(path: Path) -> None:
    for args in (["init", "-q"], ["config", "user.email", "t@example.invalid"],
                 ["config", "user.name", "t"]):
        subprocess.run(["git", "-C", str(path), *args], check=True, capture_output=True)


class TrackedRecords(unittest.TestCase):
    def test_a_hook_counts_only_staged_records(self):
        import os
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            init_repo(root)
            results = root / "research" / "results"
            results.mkdir(parents=True)
            staged, untracked = results / "battle-1-a.json", results / "battle-2-b.json"
            staged.write_text("{}")
            untracked.write_text("{}")
            (results / "other.json").write_text("{}")
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            with mock.patch.dict(os.environ, env, clear=True):
                subprocess.run(["git", "-C", str(root), "add", str(staged)], check=True)
                self.assertEqual(battle.tracked_records("battle-*.json", results, root), [staged, untracked])
                # git may export a relative index path to a hook: it is made absolute first.
                with mock.patch.dict(os.environ, {"GIT_INDEX_FILE": os.path.relpath(
                        root / ".git" / "index")}):
                    self.assertEqual(battle.tracked_records("battle-*.json", results, root), [staged])
                # Not a checkout of the repository (another toplevel): every record on disk.
                self.assertEqual(battle.tracked_records("battle-*.json", results, root / "research"),
                                 [staged, untracked])

    def test_outside_git_every_record_on_disk_counts(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp).resolve()
            (d / "battle-1.json").write_text("{}")
            self.assertEqual(battle.tracked_records("battle-*.json", d, d), [d / "battle-1.json"])
            self.assertEqual(battle.tracked_records("battle-*.json", d / "missing", d), [])


class Scoring(unittest.TestCase):
    def test_points_wins_and_exclusions(self):
        ms = [match("m-0001", "h1", 1, [30, 20, 10]),
              match("m-0002", "h2", 2, [5, 50, 1]),
              match("m-0003", "h2", 1, [30, 20, 10], replica_of="m-0001"),  # replay: not scored
              match("m-0004", "h1", 3, [99, 0, 0], status="incomplete")]    # unfinished: not scored
        board = {r["key"]: r for r in battle.leaderboard(KEYS, ms)}
        self.assertEqual(board["A"]["points"], 2 + 1)
        self.assertEqual(board["B C"]["points"], 1 + 2)
        self.assertEqual(board["B C"]["wins"], 1)
        self.assertEqual(board["A"]["played"], 2)
        self.assertEqual(board["D"]["avg_rank"], 3.0)
        self.assertEqual(battle.leaderboard(KEYS, ms)[0]["key"], "B C")   # tie broken by value

    def test_replay_comparison(self):
        ms = [match("m-0001", "h1", 1, [30, 20, 10]),
              match("m-0003", "h2", 1, [30, 20, 10], replica_of="m-0001"),
              match("m-0002", "h1", 2, [3, 2, 1]),
              match("m-0004", "h2", 2, [3, 2, 9], replica_of="m-0002")]
        det = {d["replay"]: d["identical"] for d in battle.determinism(ms)}
        self.assertEqual(det, {"m-0003": True, "m-0004": False})


def record(**over):
    ms = [match("m-0001", "h1", 1, [30, 20, 10])]
    rec = {"schema": battle.RECORD_SCHEMA, "scoring": battle.SCORING, "id": "battle-20261119T000000-abcdef12",
           "generated_at": "2026-11-19T00:00:00+00:00", "edition": "abcdef1234567890",
           "provenance": {"prismatic_path": "p", "referee_path": "r", "rev": "a" * 40,
                          "dirty": False, "baseset": "opengfx.tar", "ais": []},
           "match_cfg": MC, "contestants": [{"key": k, "ai": k, "settings": ""} for k in KEYS],
           "hosts": [{"name": "h1", "versions": ["v1"]}],
           "matches": [{"id": m["id"], "host": m["host"], "host_version": "v1", "seed": 1,
                        "years": 2, "ticks": m["ticks"], "lineup": KEYS, "replica_of": None,
                        "status": "done", "exit_code": 0,
                        "standings": [{k: f[k] for k in ("key", "value", "vehicles", "stations",
                                                         "errors")} for f in m["standings"]]}
                       for m in ms],
           "determinism": []}
    rec["leaderboard"] = battle.leaderboard(KEYS, rec["matches"])
    rec.update(over)
    return rec


class Health(unittest.TestCase):
    RULES = {"fleet_min_peak": 10, "fleet_keep": 0.7}

    @staticmethod
    def fleet(*counts):
        return [{"t": f"{1960 + i // 4}-{1 + 3 * (i % 4):02d}", "value": 1, "vehicles": v}
                for i, v in enumerate(counts)]

    def test_fleet_drop_and_errors_are_anomalies(self):
        self.assertEqual(battle.anomalies(self.fleet(5, 22, 20, 11), 0, self.RULES), ["fleet_drop:22->11"])
        self.assertEqual(battle.anomalies(self.fleet(5, 22, 20, 16), 0, self.RULES), [])
        self.assertEqual(battle.anomalies(self.fleet(2, 9, 3), 0, self.RULES), [])  # too small to judge
        self.assertEqual(battle.anomalies(self.fleet(1, 2), 3, self.RULES), ["squirrel_errors:3"])

    def test_fitness_fails_on_collapse_of_a_healthy_bot_and_on_any_bot_crash(self):
        fit = {"rules": self.RULES, "must_be_healthy": ["P steward"]}
        lineup = [{"ai": "PrismaticAI", "key": "P steward"}, {"ai": "PrismaticAI", "key": "P max"},
                  {"ai": "AAAHogEx", "key": "AAAHogEx"}]
        m = {"status": "done", "lineup": lineup, "errors": {},
             "series": {"P steward": self.fleet(5, 22, 21), "P max": self.fleet(5, 24, 9),
                        "AAAHogEx": self.fleet(1, 50, 2)}}
        self.assertEqual(battle.fitness_verdict(m, fit), [])          # only the healthy bot is judged
        m["series"]["P steward"] = self.fleet(5, 22, 8)
        self.assertEqual(battle.fitness_verdict(m, fit), ["P steward: fleet_drop:22->8"])
        m["series"]["P steward"] = self.fleet(5, 22, 21)
        m["errors"] = {"P max": 2}
        self.assertEqual(battle.fitness_verdict(m, fit), ["P max: squirrel_errors:2"])
        m["status"] = "incomplete"
        self.assertTrue(battle.fitness_verdict(m, fit)[0].startswith("match incomplete"))

    @staticmethod
    def books(vehicles, money):
        """Quarterly samples with a fleet and a bank balance, as the referee's REF Q lines carry."""
        return [{"t": f"{1955 + i // 4}-{1 + 3 * (i % 4):02d}", "value": 1, "vehicles": v, "money": m}
                for i, (v, m) in enumerate(zip(vehicles, money))]

    INCIDENT_MONEY = [600_000 + i * 160_000 for i in range(20)]   # 0.6M -> 3.64M over 20 quarters

    def test_a_flat_fleet_on_a_growing_bank_balance_hoards(self):
        # The incident (owner's live co-play game 2026-11-22, company 0): 35-37 vehicles from 1955
        # to 1960 while the bank balance climbed from 0.6M to 3.77M.
        money = self.INCIDENT_MONEY
        flat = self.books([37] * 20, money)
        self.assertEqual(battle.anomalies(flat, 0, self.RULES),
                         ["hoards:1955-01..1956-10:600000->1720000@37v"])
        wobbly = self.books([35, 36, 37, 36] * 5, money)                # the incident's 35-37
        self.assertEqual([a.split(":")[0] for a in battle.anomalies(wobbly, 0, self.RULES)], ["hoards"])
        # A fleet that grows with its money spends it.
        growing = self.books([10 + 3 * i for i in range(20)], money)
        self.assertEqual(battle.anomalies(growing, 0, self.RULES), [])
        # A poor company with a flat fleet has nothing to spend: below the floor.
        poor = self.books([12] * 20, [50_000 + i * 20_000 for i in range(20)])
        self.assertEqual(battle.anomalies(poor, 0, self.RULES), [])
        # Fewer samples than the window (default 8 quarters): too early to judge.
        self.assertEqual(battle.anomalies(self.books([37] * 7, money[:7]), 0, self.RULES), [])
        # The window and thresholds come from the rules.
        rules = dict(self.RULES, hoard_window=4, hoard_factor=1.1, hoard_floor=100_000)
        self.assertEqual(battle.anomalies(poor[:6], 0, rules),
                         ["hoards:1955-01..1955-10:50000->110000@12v"])

    def test_money_borrowed_to_build_is_not_hoarded(self):
        # The fitness seed 930967890, PrismaticAI self-steward (fix/prismatic-steward-capital,
        # 2026-11-23): 41-42 vehicles from 1964-04; the 1966-01 sample came while a rail line was
        # being built on the whole 300k loan, so the bank balance read 321k -> 795k.
        vehicles = [41, 42, 37, 36, 37, 36, 37, 37]
        money = [320_770, 327_540, 90_175, 178_265, 194_779, 283_682, 348_182, 794_771]
        loan = [0, 0, 0, 0, 0, 0, 0, 300_000]
        built = [dict(s, loan=l) for s, l in zip(self.books(vehicles, money), loan)]
        self.assertEqual(battle.anomalies(built, 0, self.RULES), [])
        # The same books with the loan not logged (a record from before loan=) read it as a hoard.
        self.assertEqual(battle.anomalies(self.books(vehicles, money), 0, self.RULES),
                         ["hoards:1955-01..1956-10:320770->794771@41v"])
        # A real hoard with no loan stays one: the owner's game, 0.6M -> 3.77M on 35-37 vehicles.
        incident = [dict(s, loan=0) for s in self.books([35, 36, 37, 36] * 5, self.INCIDENT_MONEY)]
        self.assertEqual(battle.anomalies(incident, 0, self.RULES),
                         ["hoards:1955-01..1956-10:600000->1720000@35v"])
        # And one that holds a loan throughout is judged on what it owns.
        owed = [dict(s, loan=100_000) for s in self.books([37] * 20, self.INCIDENT_MONEY)]
        self.assertEqual(battle.anomalies(owed, 0, self.RULES),
                         ["hoards:1955-01..1956-10:500000->1620000@37v"])

    def test_a_window_that_starts_in_debt_is_not_judged(self):
        # Review of #244: 100k in the bank on a 300k loan, then 520k with no loan, is a company
        # paying back its loan; "-200000->520000" passed the factor test for any rise.
        vehicles = [20] * 8
        money = [100_000, 150_000, 200_000, 250_000, 320_000, 400_000, 460_000, 520_000]
        loan = [300_000, 300_000, 250_000, 200_000, 100_000, 50_000, 0, 0]
        repaying = [dict(s, loan=l) for s, l in zip(self.books(vehicles, money), loan)]
        self.assertEqual(battle.anomalies(repaying, 0, self.RULES), [])
        # The same company holding on afterwards is caught by a window that starts above 0.
        later = self.books([20] * 16, money + [600_000 + i * 150_000 for i in range(8)])
        later = [dict(s, loan=l) for s, l in zip(later, loan + [0] * 8)]
        self.assertEqual(battle.anomalies(later, 0, self.RULES),
                         ["hoards:1955-10..1957-07:50000->900000@20v"])

    def test_the_referee_line_carries_the_loan(self):
        line = ("[2026-11-23 00:00:00] dbg: [script:1] [18] [E] REF Q date=1966-1 cid=0 value=1340672 "
                "money=794771 loan=300000 income=189062 perf=303 vehicles=37 stations=40 name=A B")
        series, _ = battle.parse_log(line + "\n" + ref(1966, 4, 1, 5))
        self.assertEqual((series[0][0]["money"], series[0][0]["loan"]), (794771, 300000))
        self.assertEqual(series[1][0]["loan"], 0)          # a line without loan= reads 0
        src = (battle.REPO / "tools/research/ai-battle/referee/BattleReferee/main.nut").read_text()
        self.assertIn('" loan=" + this.LoanOf(c)', src)

    def test_a_company_that_builds_stations_is_not_hoarding(self):
        # The planned fitness contestant (seed 930967890, 2026-11-22) kept 9 vehicles while its
        # money rose 283k -> 697k: it was laying one rail line per heartbeat, months each, so its
        # stations grew while its fleet waited. The incident's company built nothing at all.
        money = self.INCIDENT_MONEY
        building = [dict(s, stations=10 + 2 * i) for i, s in enumerate(self.books([37] * 20, money))]
        self.assertEqual(battle.anomalies(building, 0, self.RULES), [])
        idle = [dict(s, stations=40) for s in self.books([37] * 20, money)]
        self.assertEqual([a.split(":")[0] for a in battle.anomalies(idle, 0, self.RULES)], ["hoards"])

    def test_fitness_fails_on_hoarding_of_a_healthy_bot_only(self):
        fit = {"rules": self.RULES, "must_be_healthy": ["P planned"]}
        lineup = [{"ai": "PrismaticAI", "key": "P planned"}, {"ai": "PrismaticAI", "key": "P max"}]
        money = self.INCIDENT_MONEY
        m = {"status": "done", "lineup": lineup, "errors": {},
             "series": {"P planned": self.books([10 + 3 * i for i in range(20)], money),
                        "P max": self.books([37] * 20, money)}}
        self.assertEqual(battle.fitness_verdict(m, fit), [])          # only the healthy bot is judged
        m["series"]["P planned"] = self.books([37] * 20, money)
        self.assertEqual(battle.fitness_verdict(m, fit),
                         ["P planned: hoards:1955-01..1956-10:600000->1720000@37v"])

    def test_hoarding_fails_only_the_contestants_held_to_spend(self):
        # must_not_hoard narrows the hoards check within must_be_healthy.
        fit = {"rules": self.RULES, "must_be_healthy": ["P a", "P b"], "must_not_hoard": ["P a"]}
        lineup = [{"ai": "PrismaticAI", "key": "P a"}, {"ai": "PrismaticAI", "key": "P b"}]
        flat = self.books([37] * 20, self.INCIDENT_MONEY)
        m = {"status": "done", "lineup": lineup, "errors": {}, "series": {"P a": flat, "P b": flat}}
        self.assertEqual(battle.fitness_verdict(m, fit), ["P a: hoards:1955-01..1956-10:600000->1720000@37v"])
        cfg = json.loads(battle.DEFAULT_CONFIG.read_text())["fitness"]
        self.assertIn("PrismaticAI planned", cfg["must_not_hoard"])
        self.assertIn("PrismaticAI self-steward", cfg["must_not_hoard"])
        self.assertTrue(set(cfg["must_not_hoard"]) <= set(cfg["must_be_healthy"]))

    def test_the_merged_self_steward_bot_spends_on_the_fitness_seed(self):
        # PrismaticAI self-steward on seed 930967890 at 2301e263c1, 1963-07..1966-01 (battle.py
        # ablate, script=4): money 0.54M -> 1.03M, but every two-year window adds vehicles (+17% or
        # more) and stations, so it is not a hoard and the lineup is held to must_not_hoard. The
        # pre-merge tree of the rich growth held 27 vehicles and built nothing on 485k -> 1.1M.
        vehicles = [24, 24, 24, 24, 28, 28, 28, 28, 33, 33, 33]
        stations = [29, 31, 31, 31, 33, 35, 35, 35, 38, 38, 37]
        money = [535702, 608888, 687405, 780684, 721156, 817834, 901771, 960151, 883135, 990155,
                 1033888]
        merged = [dict(s, stations=st) for s, st in zip(self.books(vehicles, money), stations)]
        self.assertEqual(battle.anomalies(merged, 0, self.RULES), [])
        idle = self.books([27] * 9, [485_000 + i * 80_000 for i in range(9)])
        before = [dict(s, stations=30) for s in idle]
        kinds = [a.split(":")[0] for a in battle.anomalies(before, 0, self.RULES)]
        self.assertEqual(kinds, ["hoards"])

    def test_fitness_holds_a_planned_prismatic_ai_healthy(self):
        # The default, self-steward and solo-max lineups are all unplanned; the live game's bot was
        # planned (b.planned in main.nut: rail_a/rail_b set) and hoarded unseen.
        fit = json.loads(battle.DEFAULT_CONFIG.read_text())["fitness"]
        keys = {battle.contestant_key(c) for lu in fit["lineups"] for c in lu}
        self.assertTrue({"PrismaticAI", "PrismaticAI self-steward", "PrismaticAI solo-max",
                         "PrismaticAI planned"} <= keys)
        planned = next(c for lu in fit["lineups"] for c in lu if c.get("label") == "PrismaticAI planned")
        self.assertEqual(planned["settings"], "rail_a=0,rail_b=1")
        self.assertTrue({"PrismaticAI", "PrismaticAI self-steward", "PrismaticAI planned"}
                        <= set(fit["must_be_healthy"]))
        self.assertTrue({"PrismaticAI", "PrismaticAI self-steward"} <= set(fit["never_last"]))
        for key in ("hoard_window", "hoard_factor", "hoard_floor"):
            self.assertIn(key, fit["rules"])


class Telemetry(unittest.TestCase):
    def test_v3_fields_parse_and_v2_lines_still_do(self):
        v3 = ("dbg: [script:1] [18] [E] REF Q date=1961-4 cid=0 value=9 money=1 income=2 perf=3 "
              "vehicles=4 stations=5 rail=1 road=3 air=0 water=0 prail=-7 proad=12 pair=0 pwater=0 "
              "cargo=44 expenses=-6 name=Big Co Transport")
        series, _ = battle.parse_log(v3 + "\n" + ref(1961, 7, 0, 10))
        a, b = series[0]
        self.assertEqual((a["rail"], a["proad"], a["cargo"], a["name"]), (1, 12, 44, "Big Co Transport"))
        self.assertEqual((b["value"], b["vehicles"]), (10, 1))
        self.assertNotIn("rail", b)

    def test_postmortem_names_where_a_loser_falls_behind(self):
        lineup = [{"key": "W", "ai": "W"}, {"key": "P", "ai": "PrismaticAI"}]
        def s(t, value, veh, income, prail, proad):
            return {"t": t, "value": value, "vehicles": veh, "income": income, "rail": veh // 2,
                    "road": veh - veh // 2, "air": 0, "water": 0, "prail": prail, "proad": proad,
                    "pair": 0, "pwater": 0, "cargo": veh * 10}
        m = {"lineup": lineup, "errors": {}, "series": {
            "W": [s("1961-01", 100, 10, 50, 0, 0), s("1962-01", 1000, 100, 4000, 9000, 900)],
            "P": [s("1961-01", 80, 8, 40, 0, 0), s("1962-01", 200, 10, 100, 100, 50)]}}
        m["standings"] = battle.standings_of(m)
        pm = battle.postmortem(m)
        self.assertEqual(len(pm), 1)
        self.assertEqual((pm[0]["contestant"], pm[0]["winner"], pm[0]["gap"]), ("P", "W", 5.0))
        self.assertEqual(pm[0]["behind_since"], "1962-01")
        text = " | ".join(pm[0]["findings"])
        self.assertIn("fleet 10 vs 100", text)
        self.assertIn("rail profit/yr", text)

    def test_store_is_read_only_through_the_api(self):
        with tempfile.TemporaryDirectory() as tmp:
            st = battle.Store(Path(tmp) / "b.sqlite")
            m = match("m-0001", "h1", 1, [3, 2, 1])
            m.update(edition="e1", started_at=0.0, ticks=1, lineup=[{"key": k, "ai": k} for k in KEYS])
            st.match(m)
            r = st.query("SELECT contestant, rank FROM standings ORDER BY rank")
            self.assertEqual([row[0] for row in r["rows"]], KEYS)
            self.assertIn("error", st.query("DELETE FROM standings"))
            self.assertIn("error", st.query("SELECT 1; DROP TABLE matches"))
            lineup = json.loads(st.query("SELECT lineup FROM matches")["rows"][0][0])
            self.assertEqual(lineup[0], {"key": "A", "ai": "A", "settings": ""})

    def test_store_keeps_each_samples_loan_in_a_new_and_in_an_old_store(self):
        # The hoard rules judge money net of the loan; a stored sample must keep the loan for any
        # analysis that judges it later (research/design-net-investment-hoard-rule.md).
        with tempfile.TemporaryDirectory() as tmp:
            old = Path(tmp) / "old.sqlite"
            db = sqlite3.connect(old)  # a store from before the loan column
            db.executescript(battle.SCHEMA.replace(" loan INTEGER,", ""))
            db.execute("INSERT INTO samples (edition, match_id, contestant, t, money) "
                       "VALUES ('e0', 'm-0000', 'A', '1960-01', 7)")
            db.commit()
            db.close()
            for path in (Path(tmp) / "new.sqlite", old):
                st = battle.Store(path)
                m = match("m-0001", "h1", 1, [3, 2, 1])
                m.update(edition="e1", started_at=0.0, ticks=1,
                         lineup=[{"key": k, "ai": k} for k in KEYS])
                for series in m["series"].values():
                    for s in series:
                        s.update(money=900_000, loan=300_000)
                st.match(m)
                rows = st.query("SELECT money, loan FROM samples WHERE edition = 'e1'")["rows"]
                self.assertTrue(rows, path.name)
                self.assertEqual({tuple(r) for r in rows}, {(900_000, 300_000)}, path.name)
            older = battle.Store(old).query("SELECT loan FROM samples WHERE edition = 'e0'")["rows"]
            self.assertEqual(older, [(None,)], "a sample from before the column reads unrecorded")

    def test_fitness_fails_when_a_never_last_bot_is_last(self):
        lineup = [{"ai": "PrismaticAI", "key": "PrismaticAI"}, {"ai": "X", "key": "X"}]
        fl = Health.fleet
        m = {"status": "done", "lineup": lineup, "errors": {},
             "series": {"PrismaticAI": [dict(p, value=5) for p in fl(1, 2)],
                        "X": [dict(p, value=9) for p in fl(1, 2)]}}
        fit = {"rules": Health.RULES, "never_last": ["PrismaticAI"]}
        self.assertEqual(battle.fitness_verdict(m, fit), ["PrismaticAI: last of 2 (value 5)"])
        m["series"]["X"][-1]["value"] = 1
        self.assertEqual(battle.fitness_verdict(m, fit), [])


class Tuning(unittest.TestCase):
    T = {"seed": "t", "group": "P", "label_prefix": "P GA", "population": 4, "mutation_rate": 0.5,
         "fixed": {"self_steward": 1}, "start": {"a": 50, "b": 4},
         "space": {"a": [0, 100, 10], "b": [2, 10, 2]}}

    def test_generations_are_deterministic_and_within_bounds(self):
        g0 = battle.first_generation(self.T)
        self.assertEqual(g0, battle.first_generation(self.T))
        self.assertEqual(len(g0), 4)
        self.assertEqual(g0[0]["genes"], {"a": 50, "b": 4})
        for v in g0:
            self.assertTrue(0 <= v["genes"]["a"] <= 100 and v["genes"]["a"] % 10 == 0)
            self.assertIn("self_steward=1", v["settings"])
            self.assertEqual(v["group"], "P")
        scored = [(v, {"score": i}) for i, v in enumerate(g0)]
        g1 = battle.next_generation(self.T, {"generation": 0}, scored)
        self.assertEqual(g1[0]["genes"], g0[3]["genes"])          # elitism: the best survives
        self.assertTrue(g1[0]["label"].endswith("g1.0"))
        self.assertEqual(g1, battle.next_generation(self.T, {"generation": 0}, scored))

    def test_a_narrowed_range_reaches_the_elite_and_its_children(self):
        g0 = battle.first_generation(self.T)
        g0[3]["genes"]["b"] = 0                                    # bred under an older, wider range
        g1 = battle.next_generation(self.T, {"generation": 0}, [(v, {"score": i}) for i, v in enumerate(g0)])
        self.assertTrue(all(2 <= v["genes"]["b"] <= 10 for v in g1))
        self.assertIn("b=2", g1[0]["settings"])

    def test_brain_proposals_join_as_clamped_offspring(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "brain.json"
            f.write_text(json.dumps({"schema": "battle-brain/v1", "proposals": [
                {"settings": "a=37,b=99,zzz=1", "kind": "safe"}, {"settings": "a=200"}]}))
            t = dict(self.T, brain_offspring=2, brain_file=str(f))
            self.assertEqual(battle.brain_proposals(t), [({"a": 40, "b": 10}, "Safe"), ({"a": 100, "b": 4}, "")])
            g0 = battle.first_generation(t)
            g1 = battle.next_generation(t, {"generation": 0}, [(v, {"score": i}) for i, v in enumerate(g0)])
            self.assertEqual([v["label"].rsplit(" ", 1)[1] for v in g1[1:3]], ["g1.brainSafe", "g1.brain1"])
            f.write_text("{}")
            self.assertEqual(battle.brain_proposals(t), [])

    def test_fitness_normalises_points_and_gap(self):
        ms = [{"status": "done", "standings": [{"key": "W", "value": 100}, {"key": "X", "value": 10}]},
              {"status": "done", "standings": [{"key": "X", "value": 50}, {"key": "W", "value": 5}]},
              {"status": "done", "replica_of": "m", "standings": [{"key": "X", "value": 1}]}]
        f = battle.fitness_of("X", ms)
        self.assertEqual((f["matches"], f["points"], f["log_gap"], f["last_share"]), (2, 0.5, -0.5, 0.5))
        self.assertEqual(battle.fitness_of("X", ms, 0.5)["score"], round(0.5 - 0.05 - 0.25, 4))


INFO_FIXTURE = r'''
class X extends AIInfo {
	function GetSettings() {
		AddSetting({ name = "one", description = "a, b = c", min_value = 0, max_value = 100,
			easy_value = 30, custom_value = 30, flags = CONFIG_INGAME, });
		AddSetting({
			name = "two",
			description = "Quoted \"<name> (AI)\" // not a comment",
			min_value = -1,
			max_value = 50000,
			custom_value = 25000,
			step_size = 5000,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		/* AddSetting({ name = "commented", min_value = 0, max_value = 1 }); */
		// AddSetting({ name = "line_commented", min_value = 0, max_value = 1 });
		for (local i = 2; i <= 3; i++) {
			AddSetting({
				name = "fr" + i + "_from",
				min_value = -1, max_value = 65535,
				easy_value = -1, custom_value = -1,
				flags = 0,
			});
		}
	}
}
'''
INFO_NUT = HERE / "prismatic-ai" / "PrismaticAI" / "info.nut"
BATTLE_JSON = HERE / "ai-battle" / "battle.json"
SETTINGS = {"a": {"min": 0, "max": 100, "step": 1, "default": 50, "boolean": False, "ingame": True},
            "b": {"min": 0, "max": 20, "step": 2, "default": 4, "boolean": False, "ingame": True},
            "s": {"min": 0, "max": 1, "step": 1, "default": 1, "boolean": True, "ingame": True}}


class GeneSpace(unittest.TestCase):
    def test_info_settings_parses_one_line_multi_line_and_loop_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "info.nut"
            f.write_text(INFO_FIXTURE)
            s = battle.info_settings(f)
        self.assertEqual(sorted(s), ["fr2_from", "fr3_from", "one", "two"])
        self.assertEqual(s["one"], {"min": 0, "max": 100, "step": 1, "default": 30,
                                    "boolean": False, "ingame": True})
        self.assertEqual(s["two"], {"min": -1, "max": 50000, "step": 5000, "default": 25000,
                                    "boolean": True, "ingame": True})
        self.assertEqual(s["fr3_from"]["ingame"], False)

    def test_info_settings_reads_the_real_bot(self):
        s = battle.info_settings(INFO_NUT)
        self.assertEqual(s["cash_reserve"]["step"], 5000)
        self.assertTrue(s["intercity"]["boolean"])
        self.assertIn("fr16_cargo", s)
        self.assertIn("line24_wagons", s)

    def test_effective_space_is_info_bounds_overridden_by_narrow(self):
        t = {"genes": ["a", "b"], "narrow": {"a": [20, 80, 10]}}
        self.assertEqual(battle.gene_space(t, SETTINGS), {"a": [20, 80, 10], "b": [0, 20, 2]})
        self.assertEqual(list(battle.gene_space({"genes": ["b", "a"]}, SETTINGS)), ["b", "a"])
        legacy = {"space": {"a": [0, 10, 5]}}                      # an older configuration's form
        self.assertEqual(battle.gene_space(legacy, SETTINGS), {"a": [0, 10, 5]})

    def test_errors_name_an_unknown_gene_a_wide_narrow_and_a_pinned_gene(self):
        self.assertEqual(battle.gene_space_errors({"genes": ["a", "b"], "fixed": {"s": 1}}, SETTINGS), [])
        errs = battle.gene_space_errors({"genes": ["a", "zz", "s"], "fixed": {"s": 1, "yy": 1},
                                         "narrow": {"a": [-5, 80, 10], "b": [0, 20, 2]}}, SETTINGS)
        self.assertEqual(len(errs), 5, errs)
        joined = "\n".join(errs)
        for needle in ("zz", "yy", "a: narrow [-5, 80, 10]", "b: narrow", "s: pinned"):
            self.assertIn(needle, joined)
        self.assertTrue(battle.gene_space_errors({"genes": ["a"], "narrow": {"a": [0, 200, 1]}}, SETTINGS))
        self.assertTrue(battle.gene_space_errors({"genes": ["a"], "narrow": {"a": [50, 10, 1]}}, SETTINGS))
        self.assertTrue(battle.gene_space_errors({"space": {"q": [0, 1, 1]}}, SETTINGS))

    def test_the_real_configuration_names_only_settings_of_the_bot(self):
        t = json.loads(BATTLE_JSON.read_text())["tuning"]
        s = battle.info_settings(INFO_NUT)
        self.assertNotIn("space", t)                               # derived, not hand-listed
        self.assertEqual(battle.gene_space_errors(t, s), [])
        for k, b in t.get("narrow", {}).items():                   # a narrow differs, or it is noise
            self.assertNotEqual(b, [s[k]["min"], s[k]["max"], s[k]["step"]], k)
        for k in ("rich_growth", "modernize", "compete", "upgrade_track", "air_keep_last"):
            self.assertNotIn(k, t["genes"])                        # a bug fix is not a gene
            if k in s:
                self.assertEqual(t["fixed"].get(k), 1, k)

    def test_check_refuses_a_gene_the_bot_does_not_declare(self):
        cfg = json.loads(BATTLE_JSON.read_text())
        self.assertEqual(battle.config_errors(cfg), [])
        cfg["tuning"]["genes"] = cfg["tuning"]["genes"] + ["no_such_setting"]
        self.assertTrue(any("no_such_setting" in e for e in battle.config_errors(cfg)))
        cfg["tuning"]["genes"].pop()
        cfg["tuning"]["narrow"]["aggressiveness"] = [0, 1000, 10]  # wider than info.nut allows
        self.assertTrue(any("aggressiveness" in e for e in battle.config_errors(cfg)))

    def test_resolving_sets_the_space_and_starts_a_new_gene_at_the_bots_default(self):
        cfg = json.loads(BATTLE_JSON.read_text())
        t = cfg["tuning"]
        t["start"].pop("aggressiveness")
        battle.resolve_genes(cfg)
        s = battle.info_settings(INFO_NUT)
        self.assertEqual(list(t["space"]), t["genes"])
        self.assertEqual(t["start"]["aggressiveness"], s["aggressiveness"]["default"])
        self.assertEqual(t["space"]["aggressiveness"], t["narrow"]["aggressiveness"])
        cfg["tuning"]["genes"] = ["no_such_setting"]
        with self.assertRaises(SystemExit):
            battle.resolve_genes(cfg)


class Promotion(unittest.TestCase):
    @staticmethod
    def m(seed, vals, **kw):
        return dict({"status": "done", "seed": seed, "edition": "e",
                     "standings": [{"key": k, "value": v} for k, v in vals],
                     "lineup": [{"key": k, "ai": k, "settings": ""} for k, _ in vals]}, **kw)

    def test_pairs_on_common_seeds_and_promotes_only_clear_winners(self):
        ms = [self.m(s, [("PrismaticAI", 100), ("PrismaticAI X", 130), ("AAAHogEx", 900)]) for s in range(18)]
        ms += [self.m(s, [("PrismaticAI", 100), ("PrismaticAI Y", 104)]) for s in range(18)]
        ms.append(self.m(99, [("PrismaticAI X", 1)]))                      # no default on that seed
        ms.append(self.m(1, [("PrismaticAI", 1), ("PrismaticAI X", 999)], replica_of="m-1"))  # replay
        pv = {r["contestant"]: r for r in battle.paired_vs(ms, "PrismaticAI")}
        self.assertEqual((pv["PrismaticAI X"]["pairs"], pv["PrismaticAI X"]["wins"], pv["PrismaticAI X"]["ratio"]), (18, 18, 1.3))
        promo = [p["contestant"] for p in battle.promotions(ms, {"min_pairs": 16, "min_win_share": 0.8, "min_ratio": 1.05})]
        self.assertEqual(promo, ["PrismaticAI X"])                         # Y: +4% is below min_ratio
        self.assertEqual(battle.promotions(ms, {"min_pairs": 20}), [])

    def test_a_steady_gain_is_promoted_by_its_interval_not_its_win_share(self):
        # +15% on 2 of 3 seeds, -5% on the third: a 67% win share, below 0.8, but a steady gain.
        vals = [115, 115, 95]
        ms = [self.m(s, [("PrismaticAI", 100), ("PrismaticAI Z", vals[s % 3])]) for s in range(36)]
        z = [r for r in battle.paired_vs(ms, "PrismaticAI") if r["contestant"] == "PrismaticAI Z"][0]
        self.assertEqual((z["pairs"], z["wins"]), (36, 24))
        self.assertGreater(z["ci_low"], 1.0)
        rules = {"min_pairs": 16, "min_win_share": 0.8, "min_ratio": 1.05}
        self.assertEqual(battle.promotions(ms, rules), [])
        promo = battle.promotions(ms, dict(rules, min_pairs_ci=30, min_ci_low=1.0))
        self.assertEqual([(p["contestant"], p["why"]) for p in promo], [("PrismaticAI Z", "interval")])
        self.assertEqual(battle.promotions(ms[:24], dict(rules, min_pairs_ci=30)), [])   # too few pairs

    def test_a_configuration_is_pooled_across_its_labels(self):
        def g(seed, label, value, settings):
            return {"status": "done", "seed": seed, "started_at": seed, "edition": f"e{seed // 6}",
                    "lineup": [{"key": "PrismaticAI", "ai": "PrismaticAI", "settings": ""},
                               {"key": label, "ai": "PrismaticAI", "settings": settings}],
                    "standings": [{"key": label, "value": value}, {"key": "PrismaticAI", "value": 100}]}
        ms = [g(s, f"PrismaticAI GA g{s // 6}.0", 150, "a=1") for s in range(18)]
        pv = battle.paired_vs(ms, "PrismaticAI")
        self.assertEqual(len(pv), 1)
        self.assertEqual((pv[0]["pairs"], pv[0]["contestant"], pv[0]["settings"]), (18, "PrismaticAI GA g2.0", "a=1"))


class Records(unittest.TestCase):
    NAME = "battle-20261119T000000-abcdef12.json"

    def test_valid_record_passes(self):
        self.assertEqual(battle.validate_record(record(), self.NAME), [])

    def test_dirty_edition_unnamed_build_and_forged_board_are_refused(self):
        rec = record()
        rec["provenance"]["dirty"] = True
        rec["matches"][0]["host_version"] = ""
        rec["leaderboard"][0]["points"] += 1
        errs = "\n".join(battle.validate_record(rec, self.NAME))
        self.assertIn("uncommitted", errs)
        self.assertIn("arena build", errs)
        self.assertIn("not derived", errs)

    def test_missing_field_and_wrong_name(self):
        rec = record()
        del rec["matches"]
        self.assertTrue(battle.validate_record(rec, self.NAME))
        self.assertTrue(battle.validate_record(record(), "battle-other.json"))

    def test_region_is_derived_and_replaceable(self):
        rec = record()
        region = battle.render_region(rec)
        self.assertTrue(region.startswith(battle.BEGIN) and region.endswith(battle.END))
        self.assertIn("| 1 | A |", region)
        self.assertNotIn("| Match |", region)        # the per-match table stays in the record
        text = f"intro\n{battle.BEGIN}\nold\n{battle.END}\noutro\n"
        out = battle.replace_region(text, region)
        self.assertTrue(out.startswith("intro\n") and out.endswith("\noutro\n"))
        self.assertEqual(battle.replace_region(out, region), out)
        self.assertIn("| m-0001 | h1 | 1 | A |", battle.render_md(rec))

    def test_the_page_charts_its_record_and_the_readme_keeps_the_tables(self):
        rec = record()
        rec["provenance"]["prismatic_path"] = "tools/research/prismatic-ai/B C"  # the home AI: "B C"
        md = battle.render_md(rec)
        a, b = md.index(battle.CHARTS_BEGIN), md.index(battle.CHARTS_END)
        charts = md[a:b]
        self.assertLess(md.index("| # | Contestant |"), a)
        self.assertLess(b, md.index("| Match |"))
        self.assertEqual(charts.count("```mermaid"), 5)
        page = f"{rec['id']}.md"
        # Every bar and slice opens the row it summarises: a contestant, a match, a winner, an arena.
        self.assertIn(f"%% links: {page}?col=Contestant&filter=A | {page}?col=Contestant&filter=B+C", charts)
        self.assertIn(f"%% links: {page}?col=Match&filter=m-0001", charts)
        self.assertIn(f"%% links: {page}?col=Winner&filter=A", charts)
        self.assertIn(f"%% links: {page}?col=Arena&filter=h1", charts)
        self.assertIn("%% series: Winner | Best B C", charts)
        self.assertIn("    bar [30]\n    line [20]", charts)
        self.assertNotIn("e+", charts)                # Mermaid parses plain digits only
        self.assertNotIn(battle.CHARTS_BEGIN, battle.render_region(rec))
        region = md[a:b + len(battle.CHARTS_END)]
        self.assertEqual(battle.render_md(rec, charts=False), md.replace(region + "\n\n", ""))

    def test_render_rewrites_every_stale_page_from_its_json(self):
        import os
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp).resolve()
            rec = record()
            (d / f"{rec['id']}.json").write_text(json.dumps(rec))
            (d / f"{rec['id']}.md").write_text("hand-edited\n")
            readme = d / "README.md"
            readme.write_text(f"x\n{battle.BEGIN}\nold\n{battle.END}\n")
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            with mock.patch.dict(os.environ, env, clear=True), \
                    mock.patch.object(battle, "RESULTS_DIR", d), mock.patch.object(battle, "README", readme), \
                    mock.patch.object(battle, "REPO", d), redirect():
                self.assertEqual(battle.cmd_render(), 0)
                self.assertEqual((d / f"{rec['id']}.md").read_text(), battle.render_md(rec))
                self.assertIn(battle.render_region(rec), readme.read_text())
                before = (d / f"{rec['id']}.md").stat().st_mtime_ns
                self.assertEqual(battle.cmd_render(), 0)   # current: nothing is written again
                self.assertEqual((d / f"{rec['id']}.md").stat().st_mtime_ns, before)


def redirect():
    import contextlib
    return contextlib.redirect_stdout(io.StringIO())


def ablation(**over):
    rows = [{"variant": v, "settings": s, "exit_code": 0, "value": value, "money": 1, "vehicles": rail + road + air,
             "rail": rail, "road": road, "air": air, "errors": 0, "field_best": best, "t": "1970-01"}
            for v, s, value, best, rail, road, air in (("off", "x=0", 2000000, 8000000, 4, 10, 6),
                                                       ("on+x", "x=1", 3000000, 6000000, 9, 2, 5),
                                                       ("worse", "x=2", 1500000, 0, 1, 1, 1))]
    for r, vs in zip(rows, battle.ablation_vs_first(rows)):
        r["vs_first"] = vs
    rec = {"schema": battle.ABLATION_SCHEMA, "id": "ablation-20261119T000000-seed7",
           "generated_at": "2026-11-19T00:00:00+00:00", "seed": 7, "years": 10, "field": ["F1", "F2"],
           "match_cfg": dict(MC, years=10), "rev": "b" * 40, "tree_dirty": False, "rows": rows}
    rec.update(over)
    return rec


class Ablations(unittest.TestCase):
    NAME = "ablation-20261119T000000-seed7.json"

    def test_valid_ablation_passes_and_a_forged_ratio_or_twin_variant_is_refused(self):
        self.assertEqual(battle.validate_ablation(ablation(), self.NAME), [])
        rec = ablation()
        rec["rows"][1]["vs_first"] += 0.01
        rec["rows"][2]["variant"] = "off"
        errs = "\n".join(battle.validate_ablation(rec, self.NAME))
        self.assertIn("vs_first is not derived", errs)
        self.assertIn("named twice", errs)
        self.assertTrue(battle.validate_ablation(ablation(rows=[]), self.NAME))
        self.assertTrue(battle.validate_ablation(ablation(), "ablation-other.json"))
        self.assertTrue(battle.validate_ablation(ablation(schema="x"), self.NAME))

    def test_the_page_charts_every_variant_after_its_table(self):
        rec = ablation()
        md = battle.render_ablation_md(rec)
        a, b = md.index(battle.ABLATION_CHARTS_BEGIN), md.index(battle.ABLATION_CHARTS_END)
        charts = md[a:b]
        self.assertLess(md.index("| Variant | Value |"), a)
        self.assertEqual(charts.count("```mermaid"), 4)
        page = f"{rec['id']}.md"
        # Each bar opens its variant's row; a "+" in a name is quoted in the query.
        self.assertIn(f"%% links: {page}?col=Variant&filter=off | {page}?col=Variant&filter=on%2Bx | "
                      f"{page}?col=Variant&filter=worse", charts)
        self.assertIn("%% series: Final value | Best opponent", charts)
        self.assertIn("    bar [2000000, 3000000, 1500000]\n    line [8000000, 6000000, 0]", charts)
        self.assertIn("    bar [0, 50, -25]", charts)                 # change against "off", in %
        self.assertIn("    bar [25, 50, 0]", charts)                  # share of the best opponent
        self.assertIn("%% series: Rail | Road | Air\n", charts)
        self.assertIn("Spread between them: 100.0% of the lowest", charts)
        self.assertIn("lowest: worse (-25.0%)", charts)
        self.assertNotIn("e+", charts)                                 # plain digits only
        region = md[a:b + len(battle.ABLATION_CHARTS_END)]
        self.assertEqual(battle.render_ablation_md(rec, charts=False), md.replace("\n" + region + "\n", ""))

    def test_render_rewrites_a_stale_ablation_page_and_check_fails_until_it_does(self):
        import contextlib
        import os
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp).resolve()
            rec = ablation()
            (d / f"{rec['id']}.json").write_text(json.dumps(rec))
            page = d / f"{rec['id']}.md"
            page.write_text("hand-edited\n")
            readme = d / "README.md"
            readme.write_text(f"x\n{battle.render_region(None)}\n")
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            err = io.StringIO()
            with mock.patch.dict(os.environ, env, clear=True), \
                    mock.patch.object(battle, "RESULTS_DIR", d), mock.patch.object(battle, "README", readme), \
                    mock.patch.object(battle, "REPO", d), redirect(), contextlib.redirect_stderr(err):
                self.assertEqual(battle.cmd_check(), 11)
                self.assertIn(f"{page.name}: not derived from {rec['id']}.json", err.getvalue())
                self.assertEqual(battle.cmd_render(), 0)
                self.assertEqual(page.read_text(), battle.render_ablation_md(rec))
                self.assertEqual(battle.cmd_check(), 0)
                before = page.stat().st_mtime_ns
                self.assertEqual(battle.cmd_render(), 0)   # current: nothing is written again
                self.assertEqual(page.stat().st_mtime_ns, before)
                rec["rows"][0]["vs_first"] = 0.5            # a forged record is a contract failure
                (d / f"{rec['id']}.json").write_text(json.dumps(rec))
                self.assertEqual(battle.cmd_check(), 10)


class Export(unittest.TestCase):
    def test_export_strips_parent_directories(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "ref.tar"
            path = "tools/research/ai-battle/referee/BattleReferee"
            battle.export_tree(path, "HEAD", dest) if self._tracked(path) else self.skipTest(
                "referee not committed yet")
            with tarfile.open(dest) as t:
                names = t.getnames()
            self.assertIn("BattleReferee/main.nut", names)
            self.assertFalse(any(n.startswith("tools/") for n in names))

    @staticmethod
    def _tracked(path):
        r = subprocess.run(["git", "-C", str(battle.REPO), "ls-tree", "-d", "HEAD", path],
                           capture_output=True, text=True)
        return bool(r.stdout.strip())


# ------------------------------------------------------------------ live outcome learning (fixtures)
LIVE_T = {"seed": "t", "group": "P", "label_prefix": "P GA", "population": 4, "mutation_rate": 0.5,
          "fixed": {}, "start": {"a": 50, "b": 4}, "defaults": {"a": 30, "b": 4},
          "space": {"a": [0, 100, 10], "b": [2, 10, 2]}, "live": {"horizon_years": 4}}


def live_store():
    """Canned answers of a panel store for every LIVE_SQL query: a game (seed 7) of two runs, the
    second a --load under run seed 99, with two PrismaticAI configurations, AAAHogEx and an unknown
    company; an older run of seed 7 and an unrelated game (seed 5)."""
    def ev(run, seq, kind, date, text=None, data=None):
        return {"run_ms": run, "seq": seq, "game_date": date, "kind": kind, "text": text,
                "data": json.dumps(data if data is not None else {"text": text})}
    months = {100: ["1950-06", "1950-12", "1951-12", "1952-12"], 200: ["1952-12", "1953-06"]}
    vals = {0: [10, 20, 50, 100], 1: [10, 30, 100, 200], 2: [10, 20, 100, 400], 3: [1, 1, 1, 1]}
    series = []
    for run, ms in months.items():
        for c, vs in vals.items():
            for i, m in enumerate(ms):
                v = vs[i] if run == 100 else vs[-1] + 10 * (i + 1)
                series.append({"run_ms": run, "company": c, "name": f"Co {c}", "month": m, "value": v,
                               "money": 1, "loan": 0, "vehicles": 2})
    answers = {
        "runs": [{"instance": "panel-1", "seed": 7, "run_ms": 50, "first_month": "1950-01", "last_month": "1950-02"},
                 {"instance": "panel-1", "seed": 7, "run_ms": 100, "first_month": "1950-06", "last_month": "1952-12"},
                 {"instance": "panel-1", "seed": 99, "run_ms": 200, "first_month": "1952-12", "last_month": "1953-06"},
                 {"instance": "panel-1", "seed": 5, "run_ms": 300, "first_month": "1953-07", "last_month": "1953-09"}],
        "edges": [{"run_ms": 100, "month": "1952-12", "name": "Co 0"}, {"run_ms": 200, "month": "1952-12", "name": "Co 0"},
                  {"run_ms": 300, "month": "1953-07", "name": "Someone else"}],
        "series": series,
        "events": [ev(100, 1, "company.founded", "1950-06-01", data={"id": 0}),
                   ev(100, 2, "panel.ai", "1950-06-01", "start_ai PrismaticAI a=40,aggressiveness=40,fr_from=3,b=x sent: the company"),
                   ev(100, 3, "company.founded", "1950-06-02", data={"id": 1}),
                   ev(100, 4, "panel.console", "1950-06-02", 'start_ai "AAAHogEx"'),
                   ev(100, 5, "company.founded", "1950-06-03", data={"id": 2}),
                   ev(100, 6, "panel.console", "1950-06-03", 'start_ai "PrismaticAI" a=70,b=8,aggressiveness=70'),
                   ev(100, 7, "company.founded", "1950-06-04", data={"id": 2}),
                   ev(100, 8, "company.founded", "1950-06-05", data={"id": 3}),
                   ev(100, 9, "panel.mcp", "1951-02-01",
                      'openttd.ai.set_setting({"company": 0, "name": "a", "value": 40}) -> pending approval #1'),
                   # Never enacted (no decision): it changes nothing and does not end company 0's window.
                   ev(100, 10, "panel.mcp", "1951-03-01",
                      'openttd.ai.set_setting({"company": 0, "name": "a", "value": 90}) -> pending approval #4'),
                   ev(200, 11, "panel.mcp", "1953-02-01",
                      'openttd.ai.set_setting({"company": 2, "name": "b", "value": 10}) -> pending approval #2'),
                   ev(200, 12, "panel.mcp", "1953-02-01",
                      'openttd.ai.set_setting({"company": 2, "name": "fr_to", "value": 3}) -> pending approval #3'),
                   ev(200, 13, "decision.recorded", "1953-02-02",
                      data={"action_type": "ai.set_setting", "approval_id": 2, "command_result": "succeeded"}),
                   ev(200, 14, "decision.recorded", "1953-02-03",
                      data={"action_type": "ai.start", "approval_id": 3, "command_result": "succeeded"}),
                   ev(100, 15, "panel.mcp", "1951-04-01",
                      'openttd.ai.set_setting({"company": 0, "name": "b", "value": 6})')],
        "watchdog": [{"key": "watchdog.0", "value": json.dumps({"signature": "train 2 road 3 ship 0 aircraft 1 stations 4"})},
                     {"key": "watchdog.tuning", "value": "7"}],
        "revision": [{"version": json.dumps({"revision": "abc123"})}],
    }

    def query(name, sql):
        if "{runs}" in battle.LIVE_SQL[name]:
            assert "100,200" in sql, sql
        return answers[name]
    return query


def live_rec():
    return battle.live_record(live_store(), 7, "test")


class Live(unittest.TestCase):
    def test_record_follows_the_load_maps_ais_and_keeps_only_strategy_settings(self):
        rec = live_rec()
        self.assertEqual(rec["id"], "live-7-100")
        self.assertEqual([r["run_ms"] for r in rec["runs"]], [100, 200])
        self.assertEqual((rec["first_month"], rec["final_month"], rec["revision"]), ("1950-06", "1953-06", "abc123"))
        comp = {c["company"]: c for c in rec["companies"]}
        self.assertEqual([comp[c]["ai"] for c in range(4)], ["PrismaticAI", "AAAHogEx", "PrismaticAI", None])
        self.assertEqual(comp[0]["settings_start"], "a=40,aggressiveness=40")   # no fr_from, no non-number
        # a=40 again is no change; a=90 and b=6 were requested and never enacted.
        self.assertEqual(comp[0]["settings_requested"], [
            {"date": "1951-03-01", "name": "a", "value": 90, "applied": None},
            {"date": "1951-04-01", "name": "b", "value": 6, "applied": None}])
        self.assertEqual(comp[2]["settings_requested"],
                         [{"date": "1953-02-01", "name": "b", "value": 10, "applied": "1953-02-02"}])
        self.assertEqual([p["month"] for p in rec["series"]["0"]], ["1950-12", "1951-12", "1952-12", "1953-06"])
        self.assertEqual(rec["series"]["0"][2]["value"], 110)          # the loaded run's month replaces it
        self.assertEqual(rec["mode_mix"], {"0": {"train": 2, "road": 3, "ship": 0, "aircraft": 1, "stations": 4}})
        self.assertEqual(rec["outcome"]["leader"], 2)
        self.assertEqual(rec["outcome"]["prismatic"]["0"]["rank"], 3)
        self.assertEqual(battle.validate_live(rec, "live-7-100.json"), [])
        with self.assertRaises(SystemExit):
            battle.live_record(live_store(), 12345, "test")

    def test_validation_refuses_a_forged_or_incomplete_record(self):
        rec = live_rec()
        self.assertIn("missing outcome", battle.validate_live({k: v for k, v in rec.items() if k != "outcome"}, "x")[0])
        bad = json.loads(json.dumps(rec))
        bad["schema"], bad["outcome"]["leader_value"] = "other", 1
        errs = "\n".join(battle.validate_live(bad, "live-8-1.json"))
        self.assertIn("schema other", errs)
        self.assertIn("does not name the file", errs)
        bad = json.loads(json.dumps(rec))
        bad["outcome"]["leader_value"] = 1
        self.assertIn("not derived", battle.validate_live(bad, "live-7-100.json")[0])
        del bad["series"]["3"]
        self.assertIn("company 3 has no series", battle.validate_live(bad, "live-7-100.json")[0])
        no_ref = json.loads(json.dumps(rec))
        no_ref["companies"][1]["ai"] = "Other"
        self.assertIsNone(battle.live_outcome(no_ref)["prismatic"]["0"]["ratio_to_reference"])

    def test_helpers(self):
        self.assertEqual(battle.next_month("1950-12"), "1951-01")
        self.assertEqual(battle.next_month("1950-01"), "1950-02")
        self.assertEqual(battle.settings_dict("a=1, rail_a=3,line2_trains=1,x=y,b=-2,"), {"a": 1, "b": -2})
        self.assertEqual(battle.game_runs([], [], 7), [])
        self.assertEqual(battle.live_genes(LIVE_T, {"a": 37}), {"a": 40, "b": 4})
        self.assertEqual(battle.live_genes({k: v for k, v in LIVE_T.items() if k != "defaults"}, {}), {"a": 50, "b": 4})
        self.assertEqual(battle.live_cfg({})["reference"], "AAAHogEx")

    def test_judging_pairs_a_game_and_stops_at_the_first_setting_change(self):
        live = {(e["company"]): e for e in battle.live_entries([live_rec()], LIVE_T)}
        # Company 2's change in 1953 ends its window at 1952-12; the game is judged there.
        self.assertEqual((live[0]["at"], live[2]["at"]), ("1952-12", "1952-12"))
        self.assertEqual((live[0]["ratio"], live[2]["ratio"]), (0.5238, 1.9524))
        self.assertTrue(live[0]["lost"] and live[0]["paired"])
        self.assertFalse(live[2]["lost"])
        self.assertIn("the game's best", live[0]["why"])
        # Alone and short: no evidence; alone and long enough: judged against loss_ratio.
        rec = live_rec()
        rec["companies"][2]["ai"] = "Other"
        rec["companies"][0]["settings_requested"] = [{"date": "1951-03-01", "name": "a", "value": 90,
                                                      "applied": "1951-03-05"}]
        (e,) = battle.live_entries([rec], LIVE_T)
        self.assertFalse(e["evidence"])
        self.assertIn("first setting change 1951", e["why"])
        rec["companies"][0]["settings_requested"][0]["applied"] = None   # requested, never enacted
        (e,) = battle.live_entries([rec], LIVE_T)
        self.assertEqual((e["ratio"], e["lost"], e["paired"]), (0.5238, False, False))
        self.assertIn("a loss is under 0.5", e["why"])
        (e,) = battle.live_entries([rec], dict(LIVE_T, live={"loss_ratio": 0.6}))
        self.assertEqual((e["lost"], e["radius"]), (True, 3))          # lost to the rival: a wider veto
        self.assertEqual(battle.live_veto(LIVE_T, {"a": 100, "b": 10}, [e])["company"], 0)
        (e,) = battle.live_entries([rec], dict(LIVE_T, live={"min_years": 5}))
        self.assertIn("first setting change none", e["why"])

    def test_the_ga_never_keeps_a_configuration_that_lost_live_and_plays_the_live_winner(self):
        live = battle.live_entries([live_rec()], LIVE_T)       # lost: a=40,b=4; won: a=70,b=8
        self.assertEqual(battle.live_veto(LIVE_T, {"a": 40, "b": 4}, live)["company"], 0)
        self.assertIsNone(battle.live_veto(LIVE_T, {"a": 40, "b": 6}, live))
        wide = battle.live_entries([live_rec()], dict(LIVE_T, live={"veto_genes": 1}))
        self.assertIsNotNone(battle.live_veto(LIVE_T, {"a": 40, "b": 6}, wide))
        g0 = [battle.variant(LIVE_T, 0, i, g, []) for i, g in enumerate(
            [{"a": 10, "b": 2}, {"a": 20, "b": 2}, {"a": 30, "b": 2}, {"a": 40, "b": 4}])]
        scored = [(v, {"score": i}) for i, v in enumerate(g0)]   # the lost configuration scores best
        g1 = battle.next_generation(LIVE_T, {"generation": 0, "history": []}, scored, live)
        self.assertEqual(g1[0]["genes"], {"a": 30, "b": 2})       # the elite is the best not vetoed
        self.assertTrue(g1[1]["label"].endswith("g1.live"))
        self.assertEqual((g1[1]["genes"], g1[1]["parents"]), ({"a": 70, "b": 8}, ["live-7-100:2"]))
        self.assertTrue(all(v["genes"] != {"a": 40, "b": 4} for v in g1))
        # Played already: no seed again. Every variant vetoed: no elite, children from all of them.
        hist = {"generation": 0, "history": [{"variants": [{"genes": {"a": 70, "b": 8}}]}]}
        self.assertEqual(battle.live_seeds(LIVE_T, live, [{"a": 70, "b": 8}]), [])
        lost = [(battle.variant(LIVE_T, 0, i, {"a": 40, "b": 4}, []), {"score": i}) for i in range(2)]
        g1 = battle.next_generation(LIVE_T, hist, lost, live)
        self.assertEqual(len(g1), 4)
        self.assertFalse(any(v["label"].endswith("g1.0") and v["parents"] == [lost[1][0]["label"]] for v in g1))
        pop1 = dict(LIVE_T, population=1)
        self.assertEqual(len(battle.next_generation(pop1, {"generation": 0}, scored, live)), 1)

    def test_live_settings_take_only_a_configuration_with_live_evidence(self):
        brec = {"id": "battle-x", "contestants": [
            {"key": "P lost", "ai": "PrismaticAI", "settings": "a=40,b=4"},
            {"key": "P won", "ai": "PrismaticAI", "settings": "a=70,b=8,rail_a=2"},
            {"key": "P arena", "ai": "PrismaticAI", "settings": "a=60,b=4"},
            {"key": "P unplayed", "ai": "PrismaticAI", "settings": "a=90"},
            {"key": "AAAHogEx", "ai": "AAAHogEx", "settings": ""}],
            "leaderboard": [{"key": "AAAHogEx", "played": 4, "avg_value": 1000},
                            {"key": "P lost", "played": 4, "avg_value": 300},
                            {"key": "P arena", "played": 4, "avg_value": 250},
                            {"key": "P won", "played": 4, "avg_value": 200},
                            {"key": "P unplayed", "played": 0, "avg_value": 0}]}
        d = battle.derive_live_settings(LIVE_T, brec, [live_rec()])
        # The arena's best did not lose live but has no live evidence either: not chosen.
        self.assertEqual((d["chosen"], d["settings"]), ("live-7-100:2", "a=70,aggressiveness=70,b=8"))
        self.assertEqual(d["candidates"][0]["arena"], "P won")
        self.assertIn("found not lost", d["reason"])
        self.assertEqual([(a["candidate"], a["vetoed_by"]) for a in d["arena"]],
                         [("P lost", "live-7-100:0"), ("P arena", None), ("P won", None)])
        self.assertEqual(d["seeds"], [])                                  # the arena has played it
        # Its only winner lost elsewhere (a loss to the rival, within 3 genes): nothing is derived.
        loser = live_rec()
        loser["id"], loser["companies"][2]["ai"] = "live-8-1", "Other"
        loser["companies"][0]["settings_start"] = "a=70,b=8"
        none = battle.derive_live_settings(dict(LIVE_T, live={"loss_ratio": 0.6}), None, [live_rec(), loser])
        self.assertEqual((none["chosen"], none["settings"], none["arena"]), (None, None, []))
        self.assertEqual(none["candidates"][0]["vetoed_by"], "live-8-1:0")
        self.assertIn("no configuration has live evidence", none["reason"])

    def test_runners_read_the_panel_and_a_store_copy(self):
        import http.server
        import os
        import threading
        from unittest import mock

        class H(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["content-length"])))
                out = ({"error": "no"} if "bad" in body["sql"] else
                       {"columns": ["x", "o"], "rows": [[1, self.headers["origin"]]], "capped": False})
                data = json.dumps(out).encode()
                self.send_response(200)
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass
        srv = http.server.HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        url = f"http://127.0.0.1:{srv.server_port}/"
        try:
            q = battle.panel_query(url)
            self.assertEqual(q("n", "SELECT 1"), [{"x": 1, "o": url.rstrip("/")}])
            with self.assertRaises(SystemExit):
                q("n", "bad")
        finally:
            srv.shutdown()
            srv.server_close()
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "duckdb"
            fake.write_text("#!/bin/sh\ncase \"$5\" in *bad*) echo broken >&2; exit 1;; esac\n"
                            "echo '[{\"x\": 2}]'\n")
            fake.chmod(0o755)
            with mock.patch.dict(os.environ, {"PATH": f"{tmp}:{os.environ['PATH']}"}):
                q = battle.duckdb_query(Path(tmp) / "copy.duckdb")
                self.assertEqual(q("n", "SELECT 2"), [{"x": 2}])
                with self.assertRaises(SystemExit):
                    q("n", "bad")

    def test_export_settings_render_and_check(self):
        import contextlib
        import os
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp).resolve()
            readme = d / "README.md"
            readme.write_text(f"x\n{battle.render_region(None)}\n")
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            err = io.StringIO()
            with mock.patch.dict(os.environ, env, clear=True), \
                    mock.patch.object(battle, "RESULTS_DIR", d), mock.patch.object(battle, "README", readme), \
                    mock.patch.object(battle, "REPO", d), redirect(), contextlib.redirect_stderr(err), \
                    mock.patch.object(battle, "panel_query", lambda url: live_store()), \
                    mock.patch.object(battle, "duckdb_query", lambda path: live_store()):
                self.assertEqual(battle.main(["live-export", "--game-seed", "7"]), 2)
                self.assertEqual(battle.main(["live-export", "--game-seed", "7", "--panel", "http://x",
                                              "--db", "c.duckdb"]), 2)
                self.assertEqual(battle.main(["live-export", "--game-seed", "7", "--db", str(d / "c.duckdb")]), 0)
                with mock.patch.object(battle, "validate_live", lambda rec, name: ["odd"]):
                    self.assertEqual(battle.main(["live-export", "--game-seed", "7", "--panel", "http://x"]), 0)
                self.assertIn("odd", err.getvalue())
                rec = json.loads((d / "live-7-100.json").read_text())
                self.assertEqual(rec["source"], "panel:http://x")
                self.assertEqual(battle.cmd_check(), 11)                 # live-start.json missing
                self.assertIn("live-start.json: not derived", err.getvalue())
                self.assertEqual(battle.cmd_render(), 0)
                self.assertEqual(battle.cmd_check(), 0)
                self.assertEqual(battle.cmd_render(), 0)                 # current: nothing rewritten
                self.assertEqual(battle.main(["live-settings", "--write"]), 0)
                self.assertEqual(json.loads((d / "live-start.json").read_text())["chosen"], "live-7-100:2")
                with mock.patch.object(battle, "derive_live_settings", lambda *a: {"chosen": None, "settings": None,
                                                                                   "reason": "none"}):
                    self.assertEqual(battle.main(["live-settings"]), 10)   # nothing evidenced: no start
                rec["outcome"]["leader_value"] = 1                        # a forged record
                (d / "live-7-100.json").write_text(json.dumps(rec))
                self.assertEqual(battle.cmd_check(), 10)

    def test_the_orchestrator_vetoes_a_variant_that_lost_live(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            orch, _host = Guards.orchestrator(tmp)
            t = dict(LIVE_T, enabled=True, matches_per_variant=1)
            orch.cfg["tuning"] = t
            variants = [battle.variant(t, 0, i, g, []) for i, g in enumerate(
                [{"a": 40, "b": 4}, {"a": 20, "b": 2}])]
            path = battle.tuning_path(orch.cfg)
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({"generation": 0, "variants": variants, "history": []}))
            orch.matches = {"m": {"status": "done", "standings": [
                {"key": variants[0]["label"], "value": 9}, {"key": variants[1]["label"], "value": 5}]}}
            with mock.patch.object(battle, "cmd_report"), mock.patch.object(battle, "live_records",
                                                                            return_value=[live_rec()]):
                self.assertTrue(orch.tuning_step())
            self.assertTrue(any("vetoed" in e["msg"] and variants[0]["label"] in e["msg"] for e in orch.events))
            state = json.loads(path.read_text())
            self.assertEqual(state["variants"][0]["genes"], {"a": 20, "b": 2})

    def test_game_15805_is_a_loss_and_vetoes_the_ga_pick_it_ran(self):
        """The committed record of live game 15805 (PrismaticAI 1 train, 36x behind AAAHogEx by 1964):
        replan's request of rail_after=7 (1952-05-08) was never enacted, so it does not end the window;
        judged at 1953-12 the company lost to AAAHogEx, and GA g347.3 (which it ran, with three owner
        overrides) is vetoed."""
        results = HERE.parents[1] / "research" / "results"
        t = battle.live_tuning(json.loads(BATTLE_JSON.read_text()))
        rec = json.loads(next(results.glob("live-15805-*.json")).read_text())
        self.assertTrue(any(r["name"] == "rail_after" and not r["applied"]
                            for r in rec["companies"][0]["settings_requested"]))
        (e,) = [x for x in battle.live_entries([rec], t) if x["company"] == 0]
        self.assertEqual((e["at"], e["lost"]), ("1953-12", True))
        self.assertLess(e["ratio"], 0.5)
        board = json.loads((results / "battle-20261122T045411-a80cc94a.json").read_text())
        g347 = next(c for c in board["contestants"] if c["key"] == "PrismaticAI GA g347.3")
        self.assertIs(battle.live_veto(t, battle.live_genes(t, battle.settings_dict(g347["settings"])), [e]), e)

    def test_battle_json_declares_the_live_rule_and_it_resolves(self):
        cfg_raw = json.loads(BATTLE_JSON.read_text())
        t = battle.live_tuning(cfg_raw)
        self.assertEqual(set(battle.live_cfg(t)), set(battle.LIVE_DEFAULTS))
        self.assertTrue(set(t["defaults"]) <= set(t["space"]))


class RefereeFollow(unittest.TestCase):
    """The arena follows the repository's referee (owner, 2026-11-25: new matches log loan=)."""
    PATH = "tools/research/ai-battle/referee/BattleReferee"

    def setUp(self):
        if not Export._tracked(self.PATH):
            self.skipTest("referee not committed yet")
        self.head = battle.git("rev-parse", "HEAD")

    def cfg(self, tmp, **referee):
        cache = Path(tmp)
        return {"cache": cache, "results": cache / "results", "hosts_file": cache / "hosts.json",
                "baseset": cache / "opengfx-8.0.tar", "match": MC, "contestants": [],
                "prismatic": {"path": "tools/research/prismatic-ai/PrismaticAI"},
                "referee": {"path": self.PATH, **referee}}

    def old_arena(self, cfg):
        game = cfg["cache"] / "arena" / "game"
        game.mkdir(parents=True)
        with tarfile.open(game / battle.REFEREE_TAR, "w") as t:   # a pre-loan referee
            data = b"function Start() {}\n"
            info = tarfile.TarInfo("BattleReferee/main.nut")
            info.size = len(data)
            t.addfile(info, io.BytesIO(data))
        prov = {"rev": "b" * 40, "dirty": False, "ais": ["PrismaticAI.tar"]}
        (cfg["cache"] / "arena" / "PROVENANCE.json").write_text(json.dumps(prov))
        return game / battle.REFEREE_TAR

    def test_refresh_replaces_only_the_referee_and_records_its_commit(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            cfg = self.cfg(tmp)
            tar = self.old_arena(cfg)
            msg = battle.refresh_referee(cfg)
            self.assertIn(f"arena referee bbbbbbbbbb -> {self.head[:10]}", msg)
            self.assertIn("loan=", battle.tar_files(tar)["BattleReferee/main.nut"].decode())
            self.assertFalse(tar.with_name(battle.REFEREE_TAR + ".new").exists())
            prov = json.loads((cfg["cache"] / "arena" / "PROVENANCE.json").read_text())
            self.assertEqual((prov["rev"], prov["referee_rev"], prov["ais"]),
                             ("b" * 40, self.head, ["PrismaticAI.tar"]))
            self.assertIn("referee_prepared_at", prov)
            # Same files again: nothing changes, so neither does the edition.
            self.assertIsNone(battle.refresh_referee(cfg))
            self.assertFalse(tar.with_name(battle.REFEREE_TAR + ".new").exists())
            with mock.patch.object(battle, "log") as log:
                battle.follow_referee(cfg)
            log.assert_called_once_with("arena referee is the repository's")

    def test_refresh_refuses_a_missing_or_dirty_arena(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            cfg = self.cfg(tmp)
            with self.assertRaisesRegex(RuntimeError, "no prepared arena"):
                battle.refresh_referee(cfg)
            with mock.patch.object(battle, "log") as log:
                battle.follow_referee(cfg)          # run goes on, and says why it did not follow
            self.assertIn("referee not followed: no prepared arena", log.call_args[0][0])
            self.old_arena(cfg)
            prov = cfg["cache"] / "arena" / "PROVENANCE.json"
            prov.write_text(json.dumps({"rev": "b" * 40, "dirty": True}))
            with self.assertRaisesRegex(RuntimeError, "uncommitted"):
                battle.refresh_referee(cfg)

    def test_follow_can_be_turned_off(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            cfg = self.cfg(tmp, follow=False)
            tar = self.old_arena(cfg)
            with mock.patch.object(battle, "log") as log:
                battle.follow_referee(cfg)
            log.assert_not_called()
            self.assertNotIn(b"loan=", battle.tar_files(tar)["BattleReferee/main.nut"])
            with mock.patch.object(battle, "log") as log:
                battle.follow_referee(self.cfg(tmp))
            self.assertIn("arena referee bbbbbbbbbb ->", log.call_args[0][0])

    def test_prepare_exports_the_referee_at_its_own_rev(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            cfg = self.cfg(tmp, rev="HEAD")
            if len(battle.git("rev-list", "--max-count=2", "HEAD").split()) < 2:
                self.skipTest("a shallow clone has no older commit to pin PrismaticAI to")
            cfg["prismatic"]["rev"] = "HEAD~1"       # a pinned bot is refereed by today's referee
            cfg["baseset"].write_bytes(b"")
            with mock.patch.object(battle, "log"):
                battle.cmd_prepare(cfg, False)
            prov = json.loads((cfg["cache"] / "arena" / "PROVENANCE.json").read_text())
            self.assertEqual((prov["rev"], prov["referee_rev"]),
                             (battle.git("rev-parse", "HEAD~1"), self.head))
            self.assertIsNone(battle.refresh_referee(cfg))

    def test_cli_referee_only_and_run_follow(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            cfg = self.cfg(tmp)
            conf = Path(tmp, "battle.json")
            conf.write_text(json.dumps({**{k: str(v) for k, v in cfg.items()
                                           if isinstance(v, Path)},
                                        **{k: v for k, v in cfg.items()
                                           if not isinstance(v, Path)}}))
            with mock.patch.object(battle, "log") as log:
                self.assertEqual(battle.main(["prepare", "--referee-only", "--config", str(conf)]), 10)
            self.assertIn("refusing: no prepared arena", log.call_args[0][0])
            self.old_arena(cfg)
            with mock.patch.object(battle, "log") as log:
                self.assertEqual(battle.main(["prepare", "--referee-only", "--config", str(conf)]), 0)
            self.assertIn("arena referee bbbbbbbbbb ->", log.call_args[0][0])
            with mock.patch.object(battle, "log") as log:
                battle.main(["prepare", "--referee-only", "--config", str(conf)])
            log.assert_called_once_with("arena referee is already the repository's")
            with mock.patch.object(battle, "follow_referee") as follow, \
                    mock.patch.object(battle, "Orchestrator") as orch:
                battle.main(["run", "--config", str(conf), "--port", "1"])
            follow.assert_called_once()
            orch.return_value.run.assert_called_once_with(1)

    def test_records_name_the_referee_commit(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            orch, _ = Guards.orchestrator(tmp)
            orch.provenance = {"prismatic_path": "p", "referee_path": "r", "rev": "a" * 40,
                               "referee_rev": "c" * 40, "dirty": False, "baseset": "b", "ais": []}
            with mock.patch.object(battle, "log"):
                rec = battle.build_record(orch.cfg, orch)
            self.assertEqual(rec["provenance"]["referee_rev"], "c" * 40)
            del orch.provenance["referee_rev"]      # an arena prepared before 2026-11-25
            self.assertNotIn("referee_rev", battle.build_record(orch.cfg, orch)["provenance"])
        p = record()["provenance"]
        self.assertEqual(battle.code_at(p), f"PrismaticAI and referee at `{'a' * 40}`")
        self.assertEqual(battle.code_at(dict(p, referee_rev="a" * 40)),
                         f"PrismaticAI and referee at `{'a' * 40}`")
        self.assertEqual(battle.code_at(dict(p, referee_rev="c" * 40)),
                         f"PrismaticAI at `{'a' * 40}`, referee at `{'c' * 40}`")
        rec = record()
        rec["provenance"]["referee_rev"] = "c" * 40
        self.assertEqual(battle.validate_record(rec, Records.NAME), [])
        self.assertIn("referee at `" + "c" * 40, battle.render_md(rec))
        rec["provenance"]["referee_rev"] = "HEAD"
        self.assertIn("provenance.referee_rev is not a full commit hash",
                      "\n".join(battle.validate_record(rec, Records.NAME)))


# ------------------------------------------------------------------ live-like arena and replay

LIVE = {"seed": 15805, "slot": 0,
        "match": {"start_year": 1950, "map_x": 8, "map_y": 8, "script_opcodes": 250000},
        "rivals": [{"ai": "AAAHogEx", "version": "115"}, {"ai": "NoCAB", "version": "2.1.4"},
                   {"ai": "SimpleAI"}],
        "arms": {"unplanned": "own_lines=4", "planned": "own_lines=4,fr_from=0,fr_to=1,fr_cargo=1"}}


def company(n, name, money, loan, value, t, r=0, p=0, s=0, ai=True):
    return (f"[2026-11-24 17:00:00] #:{n}(Red) Company Name: '{name}'  Year Founded: 1950  Money: {money}  "
            f"Loan: {loan}  Value: {value}  (T:{t}, R:{r}, P:{p}, S:{s}) {'AI' if ai else ''}")


def console(months, mine_trains, mine_value=100, lead_value=1000, start=(1952, 11)):
    """A replay console log: `months` monthly reports of company 0 (the bot) and company 1."""
    lines, (y, m) = ["Console log output started to 'x'."], start
    for i in range(months):
        lines += [f"[2026-11-24 17:00:00] Date: {y:04d}-{m:02d}-01",
                  company(1, "Bot Transport", 5, 0, mine_value, mine_trains(i), r=3, p=2),
                  company(2, "AAAHogEx", 9, 300000, lead_value, 20),
                  company(9, "Unnamed", 100000, 100000, 1, 0, ai=False),
                  "Script file 'scripts/monthly.scr' scheduled to execute at the start of next calendar month."]
        y, m = (y + 1, 1) if m == 12 else (y, m + 1)
    return "\n".join(lines) + "\n"


class LiveLike(unittest.TestCase):
    def test_fetch_wanted_adds_the_live_rivals_once(self):
        cfg = {"contestants": [{"ai": "PrismaticAI", "source": "repository"},
                               {"ai": "AAAHogEx", "source": "bananas"}], "live_like": LIVE}
        self.assertEqual(battle.fetch_wanted(cfg), ["AAAHogEx", "NoCAB", "SimpleAI"])
        self.assertEqual(battle.fetch_wanted({"contestants": []}), [])
        self.assertEqual(battle.live_like({"live_like": None}), {})

    def test_rival_versions_names_a_missing_tar_and_another_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            for name in ("484f4745-AAAHogEx-115.tar", "4e434142-NoCAB-2.0.tar", "534d504c-SimpleAI-14.tar",
                         "4c444146-LuDiAI_AfterFix-v27.tar", "PrismaticAI.tar"):
                (d / name).write_bytes(b"")
            self.assertEqual(battle.tar_versions(d, "LuDiAI AfterFix"), ["v27"])
            self.assertEqual(battle.tar_versions(d / "missing", "NoCAB"), [])
            got = battle.rival_versions(d, LIVE["rivals"] + [{"ai": "DictatorAI", "version": "183"}])
            self.assertEqual(len(got), 2, got)
            self.assertIn("NoCAB: version 2.0 in", got[0])
            self.assertIn("the live game ran 2.1.4", got[0])
            self.assertIn("DictatorAI: no tar in", got[1])

    def test_battle_cfg_sets_the_script_budget_only_when_given(self):
        mc = dict(MC, breakdowns=1)
        self.assertNotIn("[script]", battle.battle_cfg_text(mc))
        text = battle.battle_cfg_text(dict(mc, script_opcodes=250000))
        self.assertTrue(text.endswith("[script]\nscript_max_opcode_till_suspend = 250000\n"), text)

    def test_live_like_ablate_takes_the_live_world_and_arms(self):
        cfg = {"match": dict(MC), "live_like": LIVE}
        out, seed, field, slot, variants = battle.live_like_ablate(cfg, None, [])
        self.assertEqual((seed, field, slot), (15805, ["AAAHogEx", "NoCAB", "SimpleAI"], 0))
        self.assertEqual(out["match"]["start_year"], 1950)
        self.assertEqual(out["match"]["landscape"], "temperate")   # kept from the arena's match
        self.assertEqual(cfg["match"]["start_year"], 1960)         # the caller's config is untouched
        self.assertEqual(variants, ["unplanned=own_lines=4", "planned=own_lines=4,fr_from=0,fr_to=1,fr_cargo=1"])
        self.assertEqual(battle.live_like_ablate(cfg, 7, ["a=x=1"])[1:5:3], (7, ["a=x=1"]))
        with self.assertRaises(SystemExit):
            battle.live_like_ablate({"match": MC}, None, [])

    def test_ablate_live_like_runs_the_live_field_from_the_command_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "repo"
            (repo / "build").mkdir(parents=True)
            (repo / "ai" / "PrismaticAI").mkdir(parents=True)
            fake = repo / "build" / "openttd"
            # Every company of the lineup reports once; the script proves the live cfg reached it.
            fake.write_text("#!/bin/sh\ngrep -q 'script_max_opcode_till_suspend = 250000' battle.cfg || exit 3\n"
                            "grep -c start_ai \"$HOME/Documents/OpenTTD/scripts/game_start.scr\" >&2\ncat <<'EOF'\n"
                            + "\n".join(ref(1956, 1, c, 10 + c) for c in range(4)) + "\nEOF\n")
            fake.chmod(0o755)
            arena = tmp / "cache" / "arena"
            for sub in ("baseset", "ai/library", "game"):
                (arena / sub).mkdir(parents=True)
            (arena / "ai" / "484f4745-AAAHogEx-115.tar").write_bytes(b"")
            conf = tmp / "battle.json"
            conf.write_text(json.dumps({
                "cache": str(tmp / "cache"), "results": str(tmp / "results"), "hosts_file": str(tmp / "h.json"),
                "baseset": str(tmp / "b.tar"), "prismatic": {"path": "ai/PrismaticAI"},
                "match": dict(MC, breakdowns=1), "live_like": LIVE}))
            old = battle.REPO
            battle.REPO = repo
            out = io.StringIO()
            try:
                with redirect_stdout(out):
                    code = battle.main(["ablate", "--live-like", "--config", str(conf)])
            finally:
                battle.REPO = old
        self.assertEqual(code, 0, out.getvalue())
        self.assertIn("ablate: warning: NoCAB: no tar in", out.getvalue())
        self.assertIn("ablate: unplanned", out.getvalue())
        self.assertIn("ablate: planned", out.getvalue())
        self.assertIn("value           10", out.getvalue())   # PrismaticAI is company 0, the live slot

    def test_plain_ablate_keeps_its_old_defaults(self):
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, redirect_stdout(out):
            conf = Replay._conf(Path(tmp))
            old = battle.REPO
            battle.REPO = Path(tmp)
            try:
                code = battle.main(["ablate", "--config", conf])
            finally:
                battle.REPO = old
        self.assertEqual(code, 12)   # no build: not run, after the --live-like / --match / --lineup choice
        self.assertIn("ablate: NOT RUN", out.getvalue())

    def test_fetch_downloads_the_live_rivals_and_reports_their_versions(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            fake = tmp / "openttd"
            # A stand-in content client: answers `content state` and writes the downloaded tars.
            fake.write_text("#!/bin/sh\nwhile read -r line; do case \"$line\" in\n"
                            "  'content state '*) echo \"7, AI, Selected, ${line#content state }, 1\";;\n"
                            "  'content download') mkdir -p content_download/ai\n"
                            "    : > content_download/ai/484f4745-AAAHogEx-115.tar\n"
                            "    : > content_download/ai/4e434142-NoCAB-2.0.tar\n"
                            "    echo 'Completed download of NoCAB';;\n"
                            "  quit) exit 0;;\nesac; done\n")
            fake.chmod(0o755)
            (tmp / "b.tar").write_bytes(b"")
            (tmp / "hosts.json").write_text(json.dumps({"hosts": [{"name": "local", "binary": str(fake)}]}))
            cfg = {"cache": tmp / "cache", "baseset": tmp / "b.tar", "hosts_file": tmp / "hosts.json",
                   "contestants": [{"ai": "AAAHogEx", "source": "bananas"}],
                   "live_like": {"rivals": [{"ai": "NoCAB", "version": "2.1.4"}]}}
            real_sleep = battle.time.sleep
            out = io.StringIO()
            with mock.patch.object(battle.time, "sleep", lambda s: real_sleep(min(s, 0.3))), redirect_stdout(out):
                battle.cmd_fetch(cfg)
        self.assertIn("fetched 1 package(s) for ['AAAHogEx', 'NoCAB']; missing: none", out.getvalue())
        self.assertIn("live-like: NoCAB: version 2.0 in", out.getvalue())


class Replay(unittest.TestCase):
    RULES = {"min_trains": 2, "one_train_years": 3, "min_leader_ratio": 0.05}

    def test_parse_console_reads_monthly_companies_and_skips_the_player(self):
        series = battle.parse_console(console(2, lambda i: 1 + i) + "Date: garbage\n#:3 not a company\n")
        self.assertEqual(sorted(series), [0, 1])        # the local player's company 9 is not an AI
        self.assertEqual([s["t"] for s in series[0]], ["1952-11-01", "1952-12-01"])
        self.assertEqual(series[0][1], {"t": "1952-12-01", "name": "Bot Transport", "money": 5, "loan": 0,
                                        "value": 100, "rail": 2, "road": 3, "air": 2, "water": 0,
                                        "vehicles": 7})
        self.assertEqual(series[1][0]["loan"], 300000)

    def test_scripts_log_the_console_and_reschedule_themselves(self):
        start, monthly = battle.replay_scripts(Path("/tmp/x/console.log"))
        self.assertTrue(start.startswith('script "/tmp/x/console.log"\n'))
        self.assertEqual(start.splitlines()[1:], monthly.splitlines())
        self.assertIn("schedule on-next-calendar-month scripts/monthly.scr", monthly)

    def test_substituted_ai_invalidates_the_replay(self):
        text = ("dbg: [script:0] The savegame has an AI by the name 'NoCAB', version 3 which is no longer "
                "available.\ndbg: [script:0] A random other AI will be loaded in its place.\n")
        self.assertEqual(battle.replay_substitutions(text),
                         ["invalid replay: the save's AI NoCAB version 3 is not available"])
        self.assertEqual(battle.replay_substitutions("all fine"), [])

    def test_an_lzma_save_is_unpacked_and_others_copied(self):
        import lzma
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "x.sav").write_bytes(b"OTTX\x01\x70\x00\x00" + lzma.compress(b"CHUNKS", format=lzma.FORMAT_XZ))
            self.assertTrue(battle.replay_copy(d / "x.sav", d / "y.sav"))
            self.assertEqual((d / "y.sav").read_bytes(), b"OTTN\x01\x70\x00\x00CHUNKS")
            self.assertFalse(battle.replay_copy(d / "y.sav", d / "z.sav"))
            self.assertEqual((d / "z.sav").read_bytes(), (d / "y.sav").read_bytes())

    def test_verdict_clauses(self):
        one = battle.parse_console(console(48, lambda i: 1, mine_value=40))
        fails = battle.replay_verdict(one, 0, 4, self.RULES)
        self.assertEqual(len(fails), 3, fails)
        self.assertIn("1 train(s) at 1956-10-01, at least 2 wanted", fails[0])
        self.assertIn("at most one train from 1953-11-01 to 1956-10-01", fails[1])
        self.assertIn("value 40 is 0.040 of the leader AAAHogEx (1,000), at least 0.05 wanted", fails[2])
        # A second line late in the replay: the end count passes, the window does not fail.
        late = battle.parse_console(console(48, lambda i: 2 if i >= 40 else 1))
        self.assertEqual(battle.replay_verdict(late, 0, 4, self.RULES), [])
        short = battle.replay_verdict(one, 0, 5, self.RULES)
        self.assertEqual(short, ["company 0: 48 monthly report(s), 60 needed: the replay did not run its years"])
        self.assertEqual(battle.replay_verdict(one, 0, 4, {}), [   # defaults: 2 trains, 3 years, no ratio
            fails[0].replace("at least 2", "at least 2"), fails[1]])
        zero = battle.parse_console(console(12, lambda i: 3, lead_value=0))
        self.assertEqual(battle.replay_verdict(zero, 0, 1, self.RULES), [])

    def test_table_has_a_line_per_year(self):
        series = battle.parse_console(console(15, lambda i: i))
        lines = battle.replay_table(series, 0)
        self.assertEqual(len(lines), 3)
        self.assertIn("replay: 1952-12-01  trains   1 road   3 air   2  value         100  leader AAAHogEx "
                      "1,000 (20 trains)  ratio 0.100", lines[0])
        self.assertIn("leader - 0 (0 trains)  ratio 0.000", battle.replay_table({0: series[0]}, 0)[0])
        self.assertEqual(battle.replay_table({}, 0), [])

    def _run(self, tmp, months, extra="", exit_code=0, rev=None, years=4, log_dir=None):
        """cmd_replay against a fake game: it writes `months` reports to the console log the start
        script names, lists the AI directory on stdout and exits with `exit_code`."""
        from unittest import mock
        import os
        repo = tmp / "repo"
        (repo / "ai" / "PrismaticAI").mkdir(parents=True, exist_ok=True)
        (repo / "ai" / "PrismaticAI" / "main.nut").write_text("class P {}\n")
        init_repo(repo)
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        with mock.patch.dict(os.environ, env, clear=True):
            subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
            subprocess.run(["git", "-C", str(repo), "commit", "-qm", "ai"], check=True)
        (tmp / "console.txt").write_text(console(months, lambda i: 1 if i < 30 else 3))
        fake = tmp / "openttd"
        fake.write_text("#!/bin/sh\nlog=$(sed -n 's/^script \"\\(.*\\)\"$/\\1/p' "
                        "\"$HOME/Documents/OpenTTD/scripts/game_start.scr\")\n"
                        f"cp {tmp / 'console.txt'} \"$log\"\nls \"$HOME/Documents/OpenTTD/ai\"\n"
                        "for a in \"$@\"; do case \"$a\" in *.sav) test -f \"$a\" || exit 4;; esac; done\n"
                        f"echo \"{extra}\"\nexit {exit_code}\n")
        fake.chmod(0o755)
        arena = tmp / "cache" / "arena"
        for sub in ("baseset", "ai/library"):
            (arena / sub).mkdir(parents=True, exist_ok=True)
        (arena / "ai" / "484f4745-AAAHogEx-115.tar").write_bytes(b"")
        import lzma
        (tmp / "live.sav").write_bytes(b"OTTX\x01\x70\x00\x00" + lzma.compress(b"save"))
        cfg = {"cache": tmp / "cache", "prismatic": {"path": "ai/PrismaticAI"},
               "live_like": {"rivals": [{"ai": "AAAHogEx", "version": "114"}]},
               "replay": {"save": str(tmp / "live.sav"), "company": 0, "years": years, "rules": self.RULES}}
        old = battle.REPO
        battle.REPO = repo
        out, err = io.StringIO(), io.StringIO()
        try:
            with mock.patch.dict(os.environ, env, clear=True), redirect_stdout(out), redirect_stderr(err):
                code = battle.cmd_replay(cfg, None, None, rev, str(fake), log_dir)
        finally:
            battle.REPO = old
        return code, out.getvalue(), err.getvalue()

    def test_replay_passes_when_the_bot_equips_more_lines(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            code, out, err = self._run(tmp, 48, log_dir=str(tmp / "logs"))
            game_log = (tmp / "logs" / "game.log").read_text()
            self.assertIn("Date: 1956-10-01", (tmp / "logs" / "console.log").read_text())
        self.assertEqual(code, 0, out + err)
        self.assertIn("replay: warning: AAAHogEx: version 115 in", out)
        self.assertIn("PrismaticAI of the working tree, company 0, 4 year(s), exit 0", out)
        self.assertIn("replay: PASS", out)
        self.assertIn("live.sav: lzma save unpacked", out)
        self.assertIn("PrismaticAI\n", game_log)    # the working tree's bot was copied in

    def test_replay_of_an_old_rev_that_fails_and_a_broken_game(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            code, out, err = self._run(
                tmp, 40, rev="HEAD", exit_code=1,
                extra="The savegame has an AI by the name 'NoCAB', version 3 which is no longer available.")
        self.assertEqual(code, 10, out + err)
        self.assertIn("PrismaticAI at HEAD", out)
        self.assertIn("replay: FAIL the game exited with 1", err)
        self.assertIn("replay: FAIL invalid replay: the save's AI NoCAB version 3 is not available", err)
        self.assertIn("40 monthly report(s), 48 needed", err)

    def test_replay_without_its_inputs_is_not_run(self):
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, redirect_stdout(out):
            code = battle.main(["replay", "--config", self._conf(Path(tmp)), "--save", "/nonexistent.sav"])
        self.assertEqual(code, 12)
        self.assertIn("replay: NOT RUN: needs the save /nonexistent.sav", out.getvalue())

    @staticmethod
    def _conf(tmp):
        conf = tmp / "battle.json"
        conf.write_text(json.dumps({"cache": str(tmp), "results": str(tmp), "hosts_file": str(tmp / "h"),
                                    "baseset": str(tmp / "b"), "prismatic": {"path": "x"}, "match": MC}))
        return str(conf)


class SiblingMark(unittest.TestCase):
    """rail.nut IsSibling knows a sibling PrismaticAI company by the auto-renew settings main.nut sets
    at every start; when one side changes and the other does not, siblings stop recognising each
    other and build second lines beside each other's (research/incident-2026-11-22-sibling-industries.md)."""

    def test_the_mark_rail_nut_reads_is_the_one_main_nut_sets(self):
        import re
        src = Path(battle.__file__).parent.parent / "prismatic-ai" / "PrismaticAI"
        main = (src / "main.nut").read_text()
        rail = (src / "rail.nut").read_text()
        self.assertIn("AICompany.SetAutoRenewStatus(true);", main)
        months = re.search(r"AICompany\.SetAutoRenewMonths\((-?\d+)\);", main)
        money = re.search(r"AICompany\.SetAutoRenewMoney\((\d+)\);", main)
        self.assertIsNotNone(months)
        self.assertIsNotNone(money)
        self.assertIn(f"const SIBLING_RENEW_MONTHS = {months.group(1)};", rail)
        self.assertIn(f"const SIBLING_RENEW_MONEY = {money.group(1)};", rail)
        # Not the game's defaults (company_settings.ini: 6 months, 100000), or every company would match.
        self.assertNotEqual((int(months.group(1)), int(money.group(1))), (6, 100000))


if __name__ == "__main__":
    unittest.main()
