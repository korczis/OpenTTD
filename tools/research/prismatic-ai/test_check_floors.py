"""Tests of check_floors.py (G6: a plan never lowers the autopilot floors)."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import check_floors  # noqa: E402

PREFIX = "[2026-11-23 03:58:44] dbg: [script:4] [{c}] [I] PrismaticAI behaviour: "
# What origin/master 11411cdc4d logged for a planned bot on the live settings (ablation seed 15805).
MASTER_PLANNED = ("aggressiveness=80 max_routes=30 cash_reserve=5000 intercity=1 self_steward=1 "
                  "own_lines=4 autopilot=growth-on-plan")
KEPT = ("aggressiveness=80 max_routes=30 cash_reserve=5000 intercity=1 self_steward=1 own_lines=10 "
        "autopilot=growth-on-plan rich=1")
FLOORS = {"AUTOPILOT_OWN_LINES": 10, "AUTOPILOT_MAX_ROUTES": 30, "AUTOPILOT_CASH_RESERVE": 5000}


def line(knobs: str, company: int = 0) -> str:
    return PREFIX.format(c=company) + knobs


class Floors(unittest.TestCase):
    def test_floors_come_from_main_nut(self):
        fl = check_floors.floors()
        self.assertEqual(set(fl), set(check_floors.FLOOR_CONSTS))
        text = check_floors.MAIN_NUT.read_text()
        for name, value in fl.items():
            self.assertIn(f"const {name} = {value};", text)

    def test_a_missing_constant_is_an_error(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "main.nut"
            p.write_text("const AUTOPILOT_OWN_LINES = 10;\n")
            with self.assertRaises(ValueError):
                check_floors.floors(p)


class Judge(unittest.TestCase):
    def test_master_planned_bot_breaks_the_own_lines_floor(self):
        judged, bad = check_floors.judge(line(MASTER_PLANNED), 0, FLOORS)
        self.assertEqual(judged, 1)
        self.assertEqual(len(bad), 1)
        self.assertIn("own_lines 4 < 10", bad[0])

    def test_every_floor_is_judged(self):
        knobs = "max_routes=4 cash_reserve=25000 intercity=0 own_lines=0 autopilot=engaged"
        _, bad = check_floors.judge(line(knobs), 0, FLOORS)
        for why in ("own_lines 0 < 10", "max_routes 4 < 30", "intercity 0 != 1", "cash_reserve 25000 > 5000"):
            self.assertIn(why, bad[0])

    def test_kept_floors_pass(self):
        self.assertEqual(check_floors.judge(line(KEPT), 0, FLOORS), (1, []))

    def test_other_companies_and_autopilot_off_are_not_judged(self):
        text = "\n".join([line(MASTER_PLANNED, company=3), line("own_lines=4 max_routes=4"), "noise"])
        self.assertEqual(check_floors.judge(text, 0, FLOORS), (0, []))


class Cli(unittest.TestCase):
    def run_cli(self, text: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "p.log"
            log.write_text(text + "\n")
            return subprocess.run([sys.executable, str(HERE / "check_floors.py"), str(log)],
                                  capture_output=True, text=True)

    def test_exit_codes(self):
        self.assertEqual(self.run_cli(line(KEPT)).returncode, 0)
        r = self.run_cli(line(MASTER_PLANNED))
        self.assertEqual(r.returncode, 1)
        self.assertIn("FAIL", r.stdout)
        r = self.run_cli("nothing here")
        self.assertEqual(r.returncode, 3)
        self.assertIn("NOT RUN", r.stdout)

    def test_in_process(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "p.log"
            log.write_text(line(KEPT) + "\n")
            self.assertEqual(check_floors.main([str(log), "--company", "0"]), 0)


if __name__ == "__main__":
    unittest.main()
