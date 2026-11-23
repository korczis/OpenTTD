#!/usr/bin/env python3
"""RESEARCH-ONLY. Offline tests for tools/research/ai-battle/regimes.py: the failure regimes, the
live judge (a fake panel over HTTP, a fake duckdb CLI, an exported record) and its CLI.

Run: python3 -B tools/research/test_regimes.py
"""
import http.server
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
import unittest.mock
import urllib.error
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("regimes", HERE / "ai-battle" / "regimes.py")
regimes = importlib.util.module_from_spec(spec)
sys.modules["regimes"] = regimes
spec.loader.exec_module(regimes)

# The owner's live game of seed 15805 (panel-8800, run 1795381184362), each company's last sample
# of each game year, value in thousands and vehicles, as research/results/live-15805-*.json
# (feature/live-outcome-learning) exported it from the store: PrismaticAI kept 25-32 vehicles
# from 1951 to 1964 while AAAHogEx went from 15 to 615.
LIVE_15805 = {
    "0": [(101, 19), (179, 27), (126, 29), (148, 32), (153, 28), (238, 27), (317, 27), (381, 26),
          (457, 26), (555, 27), (637, 26), (742, 25), (877, 26), (1010, 25), (1117, 26)],
    "1": [(116, 9), (344, 15), (623, 24), (1455, 51), (2747, 97), (4340, 230), (6301, 421),
          (7749, 532), (10562, 543), (14660, 533), (19237, 513), (24274, 548), (29370, 574),
          (34877, 587), (39932, 615)],
    "2": [(6, 14), (0, 16), (0, 18), (4, 31), (74, 38), (186, 64), (295, 94), (355, 110), (439, 125),
          (581, 155), (687, 154), (714, 150), (808, 150), (906, 150), (1018, 150)],
    "4": [(67, 30), (163, 38), (247, 52), (345, 68), (457, 90), (560, 102), (630, 114), (726, 138),
          (765, 127), (838, 161), (913, 182), (988, 205), (1062, 169), (1152, 208), (1210, 218)],
}


def live_series():
    return {c: [{"t": f"{1950 + i}-12", "value": v * 1000, "money": 1000 * i, "loan": 0, "vehicles": n}
                for i, (v, n) in enumerate(rows)] for c, rows in LIVE_15805.items()}


def quarterly(values, vehicles, start=1960, **extra):
    """Quarterly samples from two equally long lists, plus constant extra fields."""
    return [{"t": f"{start + i // 4}-{3 * (i % 4) + 1:02d}", "value": v, "money": 100, "loan": 0,
             "vehicles": n, **extra} for i, (v, n) in enumerate(zip(values, vehicles))]


def flat_field(n=13):
    """Us flat at 10 vehicles and slowly growing value; two rivals growing fleet and value."""
    return {"us": quarterly([100 + i for i in range(n)], [10] * n),
            "big": quarterly([200 * (i + 1) for i in range(n)], [5 + 2 * i for i in range(n)]),
            "mid": quarterly([150 + 10 * i for i in range(n)], [5 + i for i in range(n)])}


class NoExpansion(unittest.TestCase):
    def test_flat_fleet_while_rivals_grow_and_we_fall_behind(self):
        r = regimes.no_expansion(flat_field(), "us", ["big", "mid"], regimes.RULES)
        self.assertEqual((r["since"], r["until"], r["detected_at"]), ("1960-01", "1963-01", "1962-01"))
        self.assertEqual(r["evidence"]["vehicles"], [10, 10])
        self.assertEqual(r["evidence"]["capital"], "starved")   # net money stayed 100
        self.assertGreater(r["evidence"]["share_of_strongest"][0], r["evidence"]["share_of_strongest"][1])

    def test_idle_capital_when_net_money_rises(self):
        s = flat_field()
        for i, x in enumerate(s["us"]):
            x["money"] = 100 + 1000 * i
        self.assertEqual(regimes.no_expansion(s, "us", ["big", "mid"], regimes.RULES)["evidence"]["capital"], "idle")

    def test_a_leader_pulling_away_with_a_flat_fleet_is_not_in_the_regime(self):
        s = flat_field()
        s["us"] = quarterly([1000 * (i + 1) for i in range(13)], [10] * 13)
        self.assertIsNone(regimes.no_expansion(s, "us", ["big", "mid"], regimes.RULES))

    def test_rivals_flat_too_or_too_short_or_no_rivals(self):
        s = flat_field()
        s["big"] = quarterly([200 * (i + 1) for i in range(13)], [5] * 13)
        s["mid"] = quarterly([150] * 13, [5] * 13)
        self.assertIsNone(regimes.no_expansion(s, "us", ["big", "mid"], regimes.RULES))
        self.assertIsNone(regimes.no_expansion(flat_field(8), "us", ["big", "mid"], regimes.RULES))
        self.assertIsNone(regimes.no_expansion(flat_field(), "us", [], regimes.RULES))

    def test_the_real_15805_game_is_flagged_two_years_into_the_decade(self):
        r = regimes.no_expansion(live_series(), "0", ["1", "2", "4"], regimes.RULES)
        self.assertEqual((r["since"], r["detected_at"], r["until"]), ("1953-12", "1955-12", "1964-12"))
        self.assertEqual(r["evidence"]["vehicles"], [32, 26])


class WideningGap(unittest.TestCase):
    def field(self, ours):
        n = len(ours)
        return {"us": quarterly(ours, [10] * n),
                "lead": quarterly([1000 * 2 ** (i / 4) for i in range(n)], [10] * n),
                "peer": quarterly([500 * 2 ** (i / 4) for i in range(n)], [10] * n)}

    def test_falling_behind_faster_than_the_field(self):
        r = regimes.widening_gap(self.field([500] * 13), "us", ["lead", "peer"], regimes.RULES)
        self.assertEqual(r["evidence"]["leader"], "lead")
        self.assertLess(r["evidence"]["slope_per_year"], r["evidence"]["peer_median"])
        self.assertEqual(r["evidence"]["ratio"], [0.5, 0.0625])

    def test_named_leader_and_no_regime_cases(self):
        f = self.field([500 * 2 ** (i / 4) for i in range(13)])   # we keep pace with the peer
        self.assertIsNone(regimes.widening_gap(f, "us", ["lead", "peer"], regimes.RULES))
        f = self.field([500 * 2 ** (i / 4.4) for i in range(13)])  # behind the peer, within ln 2
        self.assertIsNone(regimes.widening_gap(f, "us", ["lead", "peer"], regimes.RULES, leader="lead"))
        f = self.field([2000 * 2 ** (i / 2) for i in range(13)])   # we gain on the leader
        self.assertIsNone(regimes.widening_gap(f, "us", ["lead", "peer"], regimes.RULES))
        self.assertIsNone(regimes.widening_gap(self.field([500] * 8), "us", ["lead", "peer"], regimes.RULES))
        self.assertIsNone(regimes.widening_gap(self.field([500] * 13), "us", [], regimes.RULES))

    def test_without_peers_the_reference_is_a_flat_ratio(self):
        f = self.field([500] * 13)
        del f["peer"]
        self.assertEqual(regimes.widening_gap(f, "us", ["lead"], regimes.RULES)["evidence"]["peer_median"], 0.0)

    def test_helpers(self):
        self.assertEqual(regimes.theil_sen([1.0], [2.0]), 0.0)
        two = {"a": quarterly([0, 0], [1, 1]), "b": quarterly([1, 1], [1, 1])}
        self.assertEqual(regimes.log_ratio_slope(two, "a", "b"), (0.0, 0.0, 0))
        self.assertEqual(regimes.month_index("1960-04-17") - regimes.month_index("1959-12"), 4)


class Modes(unittest.TestCase):
    @staticmethod
    def series(ours_air, rival_air):
        def years(p):
            out = [{"t": f"{1960 + i}-01", "value": 1, "money": 0, "loan": 0, "vehicles": 4, "rail": 0,
                    "road": 0, "air": 2, "water": 0, "prail": 0, "proad": 0, "pair": x, "pwater": 0}
                   for i, x in enumerate(p)]
            return out + [dict(out[-1], t=f"{1960 + len(p) - 1}-04")]
        return {"us": years(ours_air), "them": years(rival_air), "other": years(rival_air)}

    def test_never_pays_and_underperforms(self):
        s = self.series([0, -100, -300, -500], [50000] * 4)
        names = {r["regime"]: r for r in regimes.mode_regimes(s, "us", ["them", "other"], regimes.RULES)}
        self.assertEqual(set(names), {"mode_never_pays", "mode_underperforms"})
        self.assertEqual(names["mode_never_pays"]["evidence"],
                         {"mode": "air", "per_vehicle": -250, "years": 3,
                          "rivals_per_vehicle": {"them": 25000, "other": 25000}})
        self.assertEqual(names["mode_never_pays"]["detected_at"], "1962-01")

    def test_no_regime_when_we_earn_or_without_per_mode_profits(self):
        s = self.series([40000] * 4, [50000] * 4)
        self.assertEqual(regimes.mode_regimes(s, "us", ["them"], regimes.RULES), [])
        plain = {"us": quarterly([1, 1], [1, 1]), "them": quarterly([1, 1], [1, 1])}
        self.assertEqual(regimes.mode_regimes(plain, "us", ["them"], regimes.RULES), [])
        self.assertIsNone(regimes.per_vehicle({"air": 0}, {"air": 3, "pair": 9}, "air", "pair"))


class UnappliedPlan(unittest.TestCase):
    @staticmethod
    def round(date, adopt=True, proposed=("a",), held=None, pending=()):
        return {"date": date, "model": {"adopt": adopt}, "proposed": list(proposed), "held": held,
                "pending": list(pending)}

    def test_holding_rounds_do_not_end_the_run(self):
        rounds = [self.round("1952-01", pending=[1, 2]), self.round("1952-03", adopt=False),
                  self.round("1952-05"), self.round("1952-07"), self.round("1952-09", proposed=()),
                  self.round("1953-01"), self.round("1953-03")]
        r = regimes.unapplied_plan(rounds, regimes.RULES)
        self.assertEqual((r["since"], r["until"], r["detected_at"]), ("1952-01", "1952-07", "1952-07"))
        self.assertEqual(r["evidence"], {"rounds": 4, "adopted": 3, "settings": 1, "filed": 2, "filed_rounds": 1})
        self.assertIn("4 rounds (3 adopted), 1 setting(s) differ, 2 approval(s) filed in 1 round(s)",
                      regimes.describe(r))

    def test_too_few_held_or_never_adopted(self):
        self.assertIsNone(regimes.unapplied_plan([self.round("1952-01"), self.round("1952-02")], regimes.RULES))
        held = [self.round(f"1952-0{i}", held={"why": "x"}) for i in range(1, 6)]
        self.assertIsNone(regimes.unapplied_plan(held, regimes.RULES))
        self.assertIsNone(regimes.unapplied_plan([self.round("1952-01", adopt=False)] * 5, regimes.RULES))


class Verdicts(unittest.TestCase):
    def test_detect_describe_failures_penalty(self):
        self.assertEqual(regimes.detect({}, "us", ["x"]), [])
        found = regimes.detect(live_series(), "0", ["1", "2", "4", "9"])
        self.assertEqual([r["regime"] for r in found], ["no_expansion"])
        self.assertEqual(regimes.failures(found, "P"), ["P: regime:" + regimes.describe(found[0])])
        self.assertEqual(regimes.failures(found, "P", {"gate": []}), [])
        self.assertEqual(regimes.penalty(found), 11.0)
        self.assertEqual(regimes.penalty([]), 0)
        self.assertIn("fleet 32->26", regimes.describe(found[0]))
        gap = regimes.widening_gap(WideningGap().field([500] * 13), "us", ["lead", "peer"], regimes.RULES)
        self.assertIn("ln(value/lead)", regimes.describe(gap))
        mode = regimes.mode_regimes(Modes.series([-2] * 4, [9] * 4), "us", ["them"], regimes.RULES)[0]
        self.assertIn("air earns -1 a vehicle", regimes.describe(mode))


RECORD = {"schema": "ai-live-outcome/v1", "companies": [
    {"company": 0, "name": "Lenhill City Transport", "ai": "PrismaticAI"},
    {"company": 1, "name": "AAAHogEx", "ai": "AAAHogEx"},
    {"company": 2, "name": "LuDiAI", "ai": "LuDiAI AfterFix"},
    {"company": 4, "name": "AdmiralAI", "ai": "AdmiralAI"}],
    "series": {c: [{"month": s["t"], **{k: s[k] for k in ("value", "money", "loan", "vehicles")}}
                   for s in rows] for c, rows in live_series().items()}}


class Record(unittest.TestCase):
    def test_judge_record_and_cli(self):
        out = regimes.judge_record(RECORD, {})
        self.assertEqual(out["0"]["name"], "Lenhill City Transport")
        self.assertEqual([r["regime"] for r in out["0"]["regimes"]], ["no_expansion"])
        self.assertEqual((out["0"]["leader"], out["0"]["leader_from"], out["0"]["gap_watch"]),
                         ("1", "series", None))
        self.assertEqual(regimes.live_regimes(RECORD), {0: out["0"]["regimes"]})
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "live.json"
            f.write_text(json.dumps(RECORD))
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = regimes.main(["--record", str(f)])
            self.assertEqual(code, 10)
            self.assertIn("company 0 (Lenhill City Transport): 1 regime(s), penalty 11.0", buf.getvalue())
            buf = io.StringIO()
            with redirect_stdout(buf):
                regimes.main(["--record", str(f), "--json"])
            self.assertEqual(json.loads(buf.getvalue())["0"]["month"], "1964-12")
            run = subprocess.run([sys.executable, "-B", str(HERE / "ai-battle" / "regimes.py"), "--record", str(f)],
                                 capture_output=True, text=True)
            self.assertEqual(run.returncode, 10, run.stderr)
            self.assertIn("no_expansion: 1953-12..1964-12 (known 1955-12)", run.stdout)


def answer(sql):
    """The fake store's answer to one of regimes.SQL: company_month rows, or the planner's rounds
    of company 0 (four adopted rounds the bot never ran, and one write without a value)."""
    if "max(run_ms)" in sql:
        return [{"run_ms": 7}]
    if "replan.0" in sql:
        return [{"game_date": f"1955-0{i}-01", "data": json.dumps({"key": "replan.0", "value": {
            "date": f"1955-0{i}-01", "model": {"adopt": True}, "proposed": ["fr2_to"], "held": None,
            "pending": []}})} for i in range(1, 5)] + \
            [{"game_date": "1955-06-01", "data": json.dumps({"key": "replan.0", "value": None})}]
    if "replan." in sql:
        return []
    return [{"company": int(c), "name": f"co{c}", "month": s["t"][:7], "value": s["value"],
             "money": s["money"], "loan": None, "vehicles": s["vehicles"]}
            for c, ss in live_series().items() for s in ss]


class Store(unittest.TestCase):
    def test_duckdb_copy_through_a_fake_cli(self):
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "duckdb"
            fake.write_text("#!/usr/bin/env python3\nimport json, sys\n"
                            f"sys.path.insert(0, {str(HERE)!r})\n"
                            "from test_regimes import answer\n"
                            "if 'boom' in sys.argv[3]:\n    sys.stderr.write('no such file')\n    sys.exit(1)\n"
                            "print(json.dumps(answer(sys.argv[sys.argv.index('-c') + 1])))\n")
            fake.chmod(0o755)
            with unittest.mock.patch.dict(os.environ, {"PATH": f"{tmp}:{os.environ['PATH']}"}):
                buf = io.StringIO()
                with redirect_stdout(buf):
                    code = regimes.main(["--db", "store.duckdb", "--company", "0"])
                with self.assertRaises(SystemExit) as e:
                    regimes.duckdb_query("boom.duckdb")("SELECT 1")
                with self.assertRaises(SystemExit), unittest.mock.patch("sys.stderr", io.StringIO()):
                    regimes.main(["--db", "store.duckdb"])
        self.assertEqual(code, 10)
        self.assertIn("no such file", str(e.exception))
        self.assertIn("company 0 (co0): 2 regime(s)", buf.getvalue())
        self.assertIn("unapplied_plan: 1955-01-01..1955-04-01 (known 1955-03-01): 4 rounds (4 adopted)",
                      buf.getvalue())


class FakePanel(http.server.BaseHTTPRequestHandler):
    """The panel endpoints the live judge reads and writes."""
    posted: list = []
    capped = False

    def log_message(self, *a):
        pass

    def send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/api/bots":
            self.send(200, {"running": [{"id": 0, "ai": "PrismaticAI"}, {"id": 2, "ai": "LuDiAI AfterFix"}]})
        elif self.path == "/api/rhai/shared?key=gap.0":
            self.send(200, {"key": "gap.0", "value": {
                "leader": {"company": 1.0, "name": "AAAHogEx"}, "flags": {"losing_ground": "x"},
                "streaks": {"losing_ground": 3}, "level": 2}})
        else:
            self.send(404, {"error": "no such key"})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        if self.path == "/api/db/query":
            if FakePanel.capped:
                return self.send(200, {"columns": [], "rows": [], "capped": True})
            rows = answer(body["sql"])
            cols = list(rows[0]) if rows else []
            return self.send(200, {"columns": cols, "rows": [[r[c] for c in cols] for r in rows]})
        if self.path == "/api/rhai/shared":
            FakePanel.posted.append(body)
            return self.send(200, {"key": body["key"], "rev": 1})
        return self.send(500, {"error": "unexpected"})


class Panel(unittest.TestCase):
    def setUp(self):
        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakePanel)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}"
        FakePanel.posted, FakePanel.capped = [], False

    def tearDown(self):
        self.srv.shutdown()
        self.srv.server_close()

    def test_judges_running_companies_reuses_the_gap_watch_leader_and_posts(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = regimes.main(["--panel", self.url, "--post", "--json"])
        self.assertEqual(code, 10)
        out = json.loads(buf.getvalue())
        self.assertEqual(list(out), ["0"])
        v = out["0"]
        self.assertEqual((v["leader"], v["leader_from"], v["run_ms"]), ("1", "gap", 7))
        self.assertEqual(v["gap_watch"], {"flags": {"losing_ground": "x"}, "streaks": {"losing_ground": 3},
                                          "level": 2})
        self.assertEqual([p["key"] for p in FakePanel.posted], ["regime.rules", "regime.0"])
        self.assertEqual({p["writer"] for p in FakePanel.posted}, {"regimes"})
        self.assertEqual(FakePanel.posted[0]["value"]["rules"], regimes.RULES)
        self.assertEqual(set(FakePanel.posted[0]["value"]["why"]), set(regimes.RULES))
        self.assertEqual([r["regime"] for r in FakePanel.posted[1]["value"]["regimes"]],
                         ["no_expansion", "unapplied_plan"])

    def test_a_capped_answer_and_a_server_error_stop_it(self):
        FakePanel.capped = True
        with self.assertRaises(SystemExit) as e:
            regimes.panel_query(self.url)("SELECT 1")
        self.assertIn("capped", str(e.exception))
        with self.assertRaises(urllib.error.HTTPError):
            regimes.http_json(self.url, "/api/nothing", {})
        self.assertIsNone(regimes.http_json(self.url, "/api/rhai/shared?key=gap.9"))

    def test_a_gap_leader_that_is_not_a_rival_is_ignored(self):
        v = regimes.judge_company(live_series(), "0", ["1", "2", "4"], [], {"leader": {"company": 0}}, {})
        self.assertEqual((v["leader"], v["leader_from"]), ("1", "series"))


if __name__ == "__main__":
    unittest.main()
