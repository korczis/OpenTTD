#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for the services tools/research/mcp-e2e --coplay starts by default,
offline: no game, no panel, no stack.
Run: python3 -B tools/research/test_mcp_e2e_services.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py').
The lines that choose DEFAULT_SERVICES are cut out of tools/research/mcp-e2e and run by bash as
they stand there, so what is tested is what runs. The tests prove the two lists (a new world with
the arena; --lineup or --load without it), that the read-only monitors run in both, that the
leader-gap watch runs only where the owner's company is the only PrismaticAI one, that every name
is a panel service (a Rhai file under tools/research/mcp-panel/rhai), and that the two watches added
here declare their stall limit.
"""
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "research" / "mcp-e2e"
RHAI = ROOT / "tools" / "research" / "mcp-panel" / "rhai"


def selection_source():
    """The lines from `MONITORS=` to the `--lineup`/`--load` override, exactly as mcp-e2e holds them."""
    text = SCRIPT.read_text()
    match = re.search(r"^MONITORS=.*?\n^\[\[ -n \"\$LINEUP\".*?DEFAULT_SERVICES=.*?\n", text, re.S | re.M)
    if not match:
        raise AssertionError("mcp-e2e no longer chooses DEFAULT_SERVICES from MONITORS")
    return match.group(0)


def services(lineup="", load=()):
    """DEFAULT_SERVICES as mcp-e2e chooses it for a --lineup value and --load arguments."""
    load_words = " ".join(f"'{w}'" for w in load)
    prog = f"LINEUP='{lineup}'\nLOAD=({load_words})\n{selection_source()}printf '%s' \"$DEFAULT_SERVICES\"\n"
    out = subprocess.run(["bash", "-c", prog], capture_output=True, text=True, check=True)
    return out.stdout.split()


class DefaultServices(unittest.TestCase):
    MONITORS = ["watchdog", "build_audit", "rail_monitor", "capital_watch"]

    def test_a_new_world_runs_the_arena_replan_and_every_monitor(self):
        self.assertEqual(services(), ["arena", "replan"] + self.MONITORS)

    def test_a_lineup_or_a_save_runs_no_arena_and_the_gap_watch_too(self):
        want = ["replan"] + self.MONITORS + ["gap_watch"]
        self.assertEqual(services(lineup="AAAHogEx,NoCAB"), want)
        self.assertEqual(services(load=("--load", "golive.sav")), want)

    def test_the_gap_watch_never_runs_beside_the_arena(self):
        # In an arena the other PrismaticAI companies are rivals': the gap watch must not tell or
        # propose for them (R9), so it waits for its owner gate there.
        self.assertNotIn("gap_watch", services())

    def test_every_default_is_a_panel_service(self):
        for name in set(services()) | set(services(lineup="x")):
            path = RHAI / f"{name}.rhai"
            self.assertTrue(path.is_file(), f"{name}: no {path.relative_to(ROOT)}")

    def test_the_watches_added_here_declare_how_long_they_may_stay_quiet(self):
        # The panel's supervisor reads `// stall: <seconds>` on any comment line
        # (src/panel_scripts.rs stall_of) and restarts a service quiet for longer.
        for name in ["capital_watch", "gap_watch"]:
            text = (RHAI / f"{name}.rhai").read_text()
            self.assertRegex(text, r"(?m)^\s*//[/!]*\s*stall:\s*\d+", f"{name}.rhai declares no stall limit")


if __name__ == "__main__":
    unittest.main()
