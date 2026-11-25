#!/usr/bin/env python3
"""RESEARCH-ONLY: a plan never lowers PrismaticAI's autopilot floors (test G6).

Reads a game log at script debug level 4 (`battle.py ablate --log-dir`, a duel, the live game) and
judges every `PrismaticAI behaviour:` line of one company. With the autopilot on, planned
(`autopilot=growth-on-plan`) or not (`autopilot=engaged`), the knobs in force must keep the floors
`main.nut` declares: own_lines and max_routes at least AUTOPILOT_OWN_LINES and AUTOPILOT_MAX_ROUTES,
intercity on, cash_reserve at most AUTOPILOT_CASH_RESERVE. The floors are read from main.nut, never
repeated here. Before 2026-11-23 a plan dropped them: the live settings' own_lines=4 stood, and a
planned start cost 25-35% (research/mechanisms/prismatic-ai.md).

Exit 0: every judged line keeps the floors (PASS). 1: a line breaks one (FAIL, each printed).
3: no line of that company ran with the autopilot on (NOT RUN).
"""

import argparse
import re
import sys
from pathlib import Path

MAIN_NUT = Path(__file__).resolve().parent / "PrismaticAI" / "main.nut"
FLOOR_CONSTS = ("AUTOPILOT_OWN_LINES", "AUTOPILOT_MAX_ROUTES", "AUTOPILOT_CASH_RESERVE")
BEHAVIOUR = re.compile(r"\] \[(\d+)\] \[.\] PrismaticAI behaviour: (.*)$")


def floors(main_nut: Path = MAIN_NUT) -> dict:
    """The autopilot floors main.nut declares, by constant name."""
    text = main_nut.read_text()
    out = {}
    for name in FLOOR_CONSTS:
        m = re.search(rf"^const {name} = (\d+);", text, re.M)
        if not m:
            raise ValueError(f"{main_nut}: no `const {name} = <n>;`")
        out[name] = int(m.group(1))
    return out


def judge(log_text: str, company: int, fl: dict) -> tuple[int, list[str]]:
    """(lines judged, the breaches) for `company`'s behaviour lines with the autopilot on."""
    judged, bad = 0, []
    for line in log_text.splitlines():
        m = BEHAVIOUR.search(line)
        if not m or int(m.group(1)) != company:
            continue
        knobs = dict(kv.split("=", 1) for kv in m.group(2).split() if "=" in kv)
        if knobs.get("autopilot") not in ("engaged", "growth-on-plan"):
            continue
        judged += 1
        why = []
        if int(knobs.get("own_lines", -1)) < fl["AUTOPILOT_OWN_LINES"]:
            why.append(f"own_lines {knobs.get('own_lines')} < {fl['AUTOPILOT_OWN_LINES']}")
        if int(knobs.get("max_routes", -1)) < fl["AUTOPILOT_MAX_ROUTES"]:
            why.append(f"max_routes {knobs.get('max_routes')} < {fl['AUTOPILOT_MAX_ROUTES']}")
        if knobs.get("intercity") != "1":
            why.append(f"intercity {knobs.get('intercity')} != 1")
        if int(knobs.get("cash_reserve", 1 << 62)) > fl["AUTOPILOT_CASH_RESERVE"]:
            why.append(f"cash_reserve {knobs.get('cash_reserve')} > {fl['AUTOPILOT_CASH_RESERVE']}")
        if why:
            bad.append(f"{', '.join(why)}: autopilot={knobs['autopilot']} | {m.group(2)}")
    return judged, bad


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("log", type=Path)
    ap.add_argument("--company", type=int, default=0, help="the company id whose lines are judged")
    ap.add_argument("--main-nut", type=Path, default=MAIN_NUT, help="where the floors are declared")
    a = ap.parse_args(argv)
    fl = floors(a.main_nut)
    judged, bad = judge(a.log.read_text(errors="replace"), a.company, fl)
    if judged == 0:
        print(f"check_floors: NOT RUN: no behaviour line of company {a.company} with the autopilot on")
        return 3
    for b in bad:
        print(f"check_floors: FAIL {b}")
    print(f"check_floors: {judged} line(s) judged against {fl}, {len(bad)} below a floor: "
          f"{'FAIL' if bad else 'PASS'}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
