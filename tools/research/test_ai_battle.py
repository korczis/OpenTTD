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
        self.assertEqual(got[7], {"name": "Gundhead Coal Mine", "owners": {0, 1, 2}, "first": "1960-04"})
        self.assertEqual(got[9]["owners"], {1, 3})
        self.assertEqual(battle.parse_shared("no referee here"), {})

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
                     trainless(1961, 1, 1, 4, [5, 6, 7]), trainless(1961, 1, 2, 8, [1, 2, 3])]
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


if __name__ == "__main__":
    unittest.main()
