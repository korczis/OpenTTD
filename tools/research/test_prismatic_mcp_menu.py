"""tools/research/prismatic-mcp-menu derives the in-game menu table from ITEMS in the C++ source."""
import importlib.machinery
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
_loader = importlib.machinery.SourceFileLoader("prismatic_mcp_menu", str(HERE / "prismatic-mcp-menu"))
_spec = importlib.util.spec_from_loader("prismatic_mcp_menu", _loader)
menu = importlib.util.module_from_spec(_spec)
_loader.exec_module(menu)

FIXTURE = '''
static constexpr std::array<Item, 4> ITEMS = {{
	{"Prismatic control...", Action::Window, "control"},
	/* a comment with a "quoted" word and {braces} */
	{"Serve the towns here", Action::Request, "serve_here", {}, "Requests here"},
	{"Human player: paused", Action::Toggle, "openttd.input.yield", "paused", "Human player"},
	{"Watch my buses", Action::Request, "watch", {}, "Requests here"},
}};
'''


class Items(unittest.TestCase):
    def test_rows_keys_and_groups_parse(self):
        rows = menu.items(FIXTURE)
        self.assertEqual([r["label"] for r in rows],
                         ["Prismatic control...", "Serve the towns here", "Human player: paused", "Watch my buses"])
        self.assertEqual(rows[2]["key"], "paused")
        self.assertEqual(rows[2]["group"], "Human player")
        self.assertEqual(rows[1]["key"], "")
        self.assertEqual(menu.groups(rows), ["Requests here", "Human player"])

    def test_a_row_the_parser_misses_fails_loudly(self):
        broken = FIXTURE.replace("Item, 4>", "Item, 5>")
        with self.assertRaises(SystemExit):
            menu.items(broken)

    def test_the_real_table_parses_whole(self):
        rows = menu.items(menu.SOURCE.read_text())
        self.assertTrue(rows)
        self.assertTrue(all(r["action"] in menu.ACTIONS for r in rows), "every action is documented")


class Render(unittest.TestCase):
    def test_top_level_first_then_each_submenu_in_order(self):
        out = menu.render(menu.items(FIXTURE))
        rows = [l for l in out.splitlines() if l.startswith("| top level |") or " > | " in l]
        self.assertTrue(rows[0].startswith("| top level | Prismatic control..."))
        self.assertIn("| Requests here > | Serve the towns here |", out)
        self.assertLess(out.index("Watch my buses"), out.index("Human player: paused"))
        self.assertIn("`openttd.input.yield` `{paused}`", out)
        self.assertIn("4 items, 2 submenus", out)


class Check(unittest.TestCase):
    def test_check_fails_on_a_stale_region_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            src, readme = Path(tmp) / "menu.cpp", Path(tmp) / "README.md"
            src.write_text(FIXTURE)
            readme.write_text(f"x\n{menu.BEGIN}\nhand-typed\n{menu.END}\n")
            with mock.patch.object(menu, "SOURCE", src), mock.patch.object(menu, "README", readme), \
                    mock.patch.object(menu, "ROOT", Path(tmp)):
                self.assertEqual(menu.main(["--check"]), 1)
                self.assertIn("hand-typed", readme.read_text())
                self.assertEqual(menu.main([]), 0)
                self.assertEqual(menu.main(["--check"]), 0)
                self.assertIn("Watch my buses", readme.read_text())

    def test_missing_markers_fail(self):
        with self.assertRaises(SystemExit):
            menu.replace("no markers here", "body")


if __name__ == "__main__":
    unittest.main()
