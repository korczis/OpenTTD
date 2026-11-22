"""The PrismaticAI README's settings and policy tables derive from info.nut, battle.json, policy/.

tools/research/prismatic-ai/derive_docs.py parses every AddSetting block (loops included), marks
the battle GA's genes in either battle.json format, reads each Rhai policy stage's header, and
`--check` fails while the README region is stale. Run by tools/gate.sh change
(python3 -m unittest discover -s tools/research -p 'test_*.py').
"""
import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "prismatic_ai_docs", HERE / "prismatic-ai" / "derive_docs.py")
docs = importlib.util.module_from_spec(spec)
sys.modules["prismatic_ai_docs"] = docs
spec.loader.exec_module(docs)

INFO = '''class X extends AIInfo {
	function GetSettings() {
		AddSetting({
			name = "cash_reserve",
			description = "Money kept in the bank (a | pipe, \\"<quoted>\\").",
			min_value = 0, max_value = 500000,
			/* 25000: a comment with name = "decoy", inside the block */
			easy_value = 1, medium_value = 2, hard_value = 3, custom_value = 25000,
			step_size = 5000, // trailing comment
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_growth",
			description = "1: lift the caps.",
			min_value = 0, max_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		for (local i = 2; i <= 4; i++) {
			AddSetting({
				name = "fr" + i + "_from",
				description = "Producer of line " + i + " (-1: none).",
				min_value = -1, max_value = 65535, custom_value = -1,
				flags = 0,
			});
		}
	}
}
'''

README = """# Bot

<!-- prismatic-ai-settings:begin -->
old
<!-- prismatic-ai-settings:end -->

<!-- prismatic-ai-policy:begin -->
<!-- prismatic-ai-policy:end -->
"""

STAGE = '''// Gap stage (RESEARCH-ONLY): raises growth knobs while behind. More text here.
// Second line.
fn propose(ctx, params) {
    let n = env("TUNE_GAP_STEPS") + params.min_gap + params["steps_max"];
    let url = `http://127.0.0.1/${n}`;
    ttd::set_bot("rich_growth", 1);
    let s = #{ cash_reserve: 0 };
}
'''


CHAIN = '''// Chain: the order.
fn chain() {
    #{
        stages: ["gap", "other"],
        merge: "priority",
        params: #{
            common: #{ growth: #{ max_routes: 1 } },
            gap: #{ switches: #{ compete: 1 }, knobs: ["max_routes"] },
            other: #{ kind: "safe" },
        },
    }
}
'''


class ParseSettings(unittest.TestCase):
    def setUp(self):
        self.settings = docs.parse_settings(INFO)

    def test_fields(self):
        cash = self.settings[0]
        self.assertEqual(cash["name"], "cash_reserve")
        self.assertEqual((cash["min"], cash["max"], cash["step"], cash["default"]),
                         (0, 500000, 5000, 25000))
        self.assertTrue(cash["live"])
        self.assertFalse(cash["boolean"])
        self.assertEqual(cash["description"], 'Money kept in the bank (a | pipe, "<quoted>").')
        self.assertEqual(cash["line"], 3)

    def test_comment_is_not_a_setting(self):
        self.assertNotIn("decoy", [s["name"] for s in self.settings])
        self.assertEqual(len(self.settings), 3)

    def test_boolean_and_loop(self):
        rich, fr = self.settings[1], self.settings[2]
        self.assertTrue(rich["boolean"])
        self.assertEqual(rich["step"], 1)
        self.assertEqual(fr["name"], "fr<N>_from")
        self.assertEqual(fr["slots"], [2, 4])
        self.assertEqual(fr["description"], "Producer of line N (-1: none).")
        self.assertFalse(fr["live"])
        self.assertEqual(docs.concrete_names(self.settings),
                         {"cash_reserve", "rich_growth", "fr2_from", "fr3_from", "fr4_from"})


class Genes(unittest.TestCase):
    def test_space_format(self):
        genes, fixed = docs.parse_genes({"tuning": {"space": {"cash_reserve": [0, 40000, 5000]},
                                                    "fixed": {"rich_growth": 1}}})
        self.assertEqual(genes, {"cash_reserve": [0, 40000, 5000]})
        self.assertEqual(fixed, {"rich_growth": 1})

    def test_genes_and_narrow_format(self):
        genes, _ = docs.parse_genes({"tuning": {"genes": ["cash_reserve", {"name": "fr2_from"}],
                                                "narrow": {"cash_reserve": [0, 10000, 1000]}}})
        self.assertEqual(genes, {"cash_reserve": [0, 10000, 1000], "fr2_from": None})

    def test_no_tuning(self):
        self.assertEqual(docs.parse_genes({}), ({}, {}))


class Render(unittest.TestCase):
    def test_settings_table(self):
        settings = docs.parse_settings(INFO)
        out = docs.render_settings(settings, {"cash_reserve": [0, 40000, 5000], "ghost": None},
                                   {"rich_growth": 1})
        self.assertIn("5 settings in 3 declarations, 2 of them live-changeable", out)
        self.assertIn("| [`cash_reserve`](PrismaticAI/info.nut#L3) | 25000 | 0 .. 500000 / 5000 "
                      "| yes | gene 0 .. 40000 / 5000 "
                      "| Money kept in the bank (a \\| pipe, \"&lt;quoted&gt;\"). |", out)
        self.assertIn("| 1 | 0 / 1 | yes | pinned 1 |", out)
        self.assertIn("[`fr<N>_from`](PrismaticAI/info.nut#L", out)
        self.assertIn("(N = 2 .. 4) | -1 | -1 .. 65535 | no |", out)
        self.assertIn("Genes the bot does not declare: `ghost`.", out)

    def test_policy_stage(self):
        st = docs.parse_stage(STAGE, {"rich_growth", "cash_reserve", "max_routes"})
        self.assertEqual(st["purpose"], "Raises growth knobs while behind.")
        self.assertEqual(st["env"], ["TUNE_GAP_STEPS"])
        self.assertEqual(st["params"], ["min_gap", "steps_max"])
        self.assertEqual(st["role"], "stage")
        self.assertEqual(st["settings"], ["cash_reserve", "rich_growth"])
        chain = docs.parse_chain(CHAIN, {"rich_growth", "cash_reserve", "max_routes", "compete"})
        self.assertEqual(chain["order"], ["gap", "other"])
        self.assertEqual(chain["merge"], "priority")
        self.assertEqual(chain["settings"]["gap"], ["compete", "max_routes"])
        self.assertEqual(docs.parse_stage(CHAIN, set())["role"], "chain")
        out = docs.render_policy({"lib.rhai": docs.parse_stage("// Helpers: shared.\n", set()),
                                  "gap.rhai": st}, chain)
        self.assertIn("the chain runs 2 stages in this order, merged by `priority`", out)
        self.assertIn("| 1 | [`gap.rhai`](../mcp-panel/rhai/policy/gap.rhai) | stage "
                      "| Raises growth knobs while behind. "
                      "| `min_gap`, `steps_max`, env `TUNE_GAP_STEPS` | `cash_reserve`, "
                      "`compete`, `max_routes`, `rich_growth` |", out)
        self.assertLess(out.index("gap.rhai"), out.index("lib.rhai"), "chain order first")

    def test_no_policy(self):
        self.assertIn("No Rhai policy stage yet", docs.render_policy({}))


class Check(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = pathlib.Path(self.tmp.name)
        (root / "PrismaticAI").mkdir()
        (root / "PrismaticAI" / "info.nut").write_text(INFO)
        self.readme = root / "README.md"
        self.readme.write_text(README)
        self.patches = [mock.patch.object(docs, "ROOT", root),
                        mock.patch.object(docs, "INFO", root / "PrismaticAI" / "info.nut"),
                        mock.patch.object(docs, "README", self.readme),
                        mock.patch.object(docs, "BATTLE", root / "battle.json"),
                        mock.patch.object(docs, "POLICY", root / "policy")]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def test_stale_then_current(self):
        with mock.patch("sys.stderr"):
            self.assertEqual(docs.main(["--check"]), 1)
        self.assertEqual(self.readme.read_text(), README, "--check must not write")
        with mock.patch("sys.stdout"):
            self.assertEqual(docs.main([]), 0)
        self.assertEqual(docs.main(["--check"]), 0)
        (docs.POLICY).mkdir()
        (docs.POLICY / "gap.rhai").write_text(STAGE)
        with mock.patch("sys.stderr"):
            self.assertEqual(docs.main(["--check"]), 1, "a new policy stage makes the region stale")

    def test_missing_markers(self):
        self.readme.write_text("# Bot\n")
        with self.assertRaises(SystemExit):
            docs.main(["--check"])


if __name__ == "__main__":
    unittest.main()
