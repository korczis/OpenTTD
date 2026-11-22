#!/usr/bin/env python3
"""RESEARCH-ONLY. Offline tests for `battle.py power` (Battlebrain Phase 06,
research/battlebrain/experiment-freight-margin.md): the split-half Kendall tau of the GA score, the
paired deviation across seeds, and the held-start pairs a margin needs. No game, no arena.

Run: python3 -B tools/research/test_ai_battle_power.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/gate.sh change does).
"""
import importlib.util
import math
import statistics
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("battle_power_under_test", HERE / "ai-battle" / "battle.py")
battle = importlib.util.module_from_spec(spec)
sys.modules["battle_power_under_test"] = battle
spec.loader.exec_module(battle)


def record(edition, rows):
    """A battle record of matches (seed, base value, X value, field value): the default PrismaticAI
    and configuration X, each in its own match against one field AI, as the arena runs them, plus a
    replica that must not count."""
    contestants = [{"key": "PrismaticAI", "ai": "PrismaticAI"},
                   {"key": "PrismaticAI x", "ai": "PrismaticAI", "settings": "a=1"},
                   {"key": "Field", "ai": "Field"}]
    matches = []
    for i, (seed, b, x, f) in enumerate(rows):
        for j, (key, v) in enumerate((("PrismaticAI", b), ("PrismaticAI x", x))):
            st = sorted([{"key": key, "value": v}, {"key": "Field", "value": f}], key=lambda s: -s["value"])
            matches.append({"id": f"m-{i}-{j}", "seed": seed, "status": "done", "replica_of": None, "standings": st})
    matches.append({"id": "m-r", "seed": rows[0][0], "status": "done", "replica_of": "m-0-1",
                    "standings": [{"key": "PrismaticAI x", "value": 1}, {"key": "Field", "value": 9}]})
    return {"edition": edition, "contestants": contestants, "matches": matches}


class Power(unittest.TestCase):
    def test_kendall_tau_counts_concordant_and_discordant_pairs(self):
        self.assertEqual(battle.kendall_tau([1, 2, 3], [1, 2, 3]), 1.0)
        self.assertEqual(battle.kendall_tau([1, 2, 3], [3, 2, 1]), -1.0)
        self.assertAlmostEqual(battle.kendall_tau([1, 2, 3, 4], [1, 3, 2, 4]), 4 / 6)

    def test_the_sample_size_follows_the_paired_deviation_and_the_margin(self):
        # X is the base times e^(+-0.2) on alternate seeds, so the deviation of the log ratios is known.
        xs = [round(1000 * math.exp(0.2 if s % 2 else -0.2)) for s in range(20)]
        rows = [(100 + s, 1000, xs[s], 500) for s in range(20)]
        out = battle.power([record("e1", rows)], margin=1.10, min_pairs=16, min_seeds=8)
        sigma = statistics.stdev([math.log(x / 1000) for x in xs])
        self.assertAlmostEqual(out["paired"]["sigma_median"], round(sigma, 4))
        self.assertEqual(out["paired"]["configurations"], 1, "the base is not paired with itself; the replica is not counted")
        z = statistics.NormalDist().inv_cdf
        expect = math.ceil(((z(0.95) + z(0.80)) * sigma / math.log(1.10)) ** 2 + z(0.95) ** 2 / 2)
        self.assertEqual(out["pairs"]["n"], expect)
        self.assertGreater(out["pairs"]["sensitivity"]["1.05"], expect)
        self.assertLess(out["pairs"]["sensitivity"]["1.25"], expect)
        seeds = out["pairs"]["seeds"]
        self.assertEqual(seeds[:3], [4242, 31337, 20208], "the rule's three seeds first")
        self.assertEqual(seeds[3:6], [100, 101, 102], "then the base's seeds in record order")
        self.assertEqual(len(seeds), min(expect, 23), "never more seeds than the records hold")

    def test_the_split_half_tau_ranks_each_configuration_on_its_odd_and_even_seeds(self):
        rows = [(s, 1000, 1100, 500 if s % 2 else 1050) for s in range(10)]
        out = battle.power([record("e1", rows)], min_seeds=8)
        self.assertEqual(out["split_half"]["configurations"], 2)
        self.assertIsNotNone(out["split_half"]["kendall_tau"])

    def test_too_few_pairs_give_no_sample_size(self):
        out = battle.power([record("e1", [(1, 10, 11, 5), (2, 10, 9, 5)])])
        self.assertIsNone(out["pairs"]["n"])
        self.assertEqual(out["pairs"]["seeds"], [])

    def test_pairs_are_made_within_one_edition_only(self):
        rows = [(s, 1000, 1100 + s, 500) for s in range(20)]
        a, b = record("e1", rows), record("e2", rows)
        a["matches"] = [m for m in a["matches"] if all(s["key"] != "PrismaticAI" for s in m["standings"])]
        self.assertEqual(battle.power([a, b])["paired"]["configurations"], 1,
                         "e1's X has no base in e1, and e2's base does not pair with it")
        self.assertEqual(battle.power([a])["paired"]["configurations"], 0)


if __name__ == "__main__":
    unittest.main()
