"""RESEARCH-ONLY: tests of mcp-auto's decision contract against a scripted panel.

The loop submits a decision once and only watches the approval it is given (ADR 0038): these
tests drive decide() through every way an approval can end, with a fake clock so that the
waits cost nothing, and check that the loop never approves and never resubmits.

    python3 -B -m unittest discover -s tools/research/mcp-auto -p 'test_*.py'
"""
import importlib.machinery
import importlib.util
import io
import json
import pathlib
import unittest
from contextlib import redirect_stdout

SCRIPT = pathlib.Path(__file__).resolve().parent / "mcp-auto"


def load():
    loader = importlib.machinery.SourceFileLoader("mcp_auto_under_test", str(SCRIPT))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


M = load()


class Clock:
    """time.time and time.sleep for the module: sleeping advances the clock, nothing waits."""

    def __init__(self):
        self.now = 1000.0

    def time(self):
        return self.now

    def sleep(self, s):
        self.now += s

    def strftime(self, fmt):
        return "00:00:00"


class FakePanel:
    """A panel whose tool replies and resource reads are scripted, and which records calls."""

    def __init__(self, tools, approvals=(), decisions=()):
        self.tools = {k: list(v) for k, v in tools.items()}
        self.approvals = list(approvals)  # one approvals list per read; the last one repeats
        self.decisions = list(decisions)
        self.calls = []

    def tool(self, name, args):
        self.calls.append(name)
        replies = self.tools[name]
        return (replies.pop(0) if len(replies) > 1 else replies[0]), False

    def read(self, uri):
        if uri == "openttd://approvals":
            cur = self.approvals.pop(0) if len(self.approvals) > 1 else self.approvals[0]
            return {"data": {"approvals": cur}}
        if uri == "openttd://decisions":
            return {"data": {"decisions": self.decisions}}
        raise AssertionError("unexpected read %s" % uri)


ACTION = {"type": "game.set_pause", "arguments": {"pause": False}}
VALID = {"openttd.decision.validate": [{"valid": True}]}


def pending(aid=7):
    return {"openttd.decision.submit": [{"status": "pending_approval", "approval_id": aid}]}


class DecideTest(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.saved = M.time
        M.time = self.clock

    def tearDown(self):
        M.time = self.saved

    def run_decide(self, panel, wait_s=10.0):
        out = io.StringIO()
        with redirect_stdout(out):
            rec = M.decide(panel, ACTION, wait_s)
        return rec, [json.loads(line) for line in out.getvalue().splitlines()]

    def assert_never_approves_or_resubmits(self, panel):
        self.assertEqual(panel.calls.count("openttd.decision.submit"), 1, panel.calls)
        self.assertFalse([c for c in panel.calls if "approve" in c], panel.calls)

    def test_a_refused_validation_submits_nothing(self):
        panel = FakePanel({"openttd.decision.validate": [{"valid": False, "errors": ["x"]}]})
        rec, log = self.run_decide(panel)
        self.assertFalse(rec["valid"])
        self.assertEqual(panel.calls, ["openttd.decision.validate"])
        self.assertEqual(log[-1]["msg"], "refused by validation")

    def test_an_executed_submit_waits_for_its_verdict(self):
        tools = dict(VALID, **{
            "openttd.decision.submit": [{"status": "executed", "decision_id": 3}],
            "openttd.decision.get": [{"decision_id": 3, "verdict": "PENDING_OBSERVATION"},
                                     {"decision_id": 3, "verdict": "VERIFIED_SUCCESS",
                                      "command_result": "succeeded", "approval_id": None}],
        })
        panel = FakePanel(tools)
        rec, log = self.run_decide(panel)
        self.assertEqual(rec["verdict"], "VERIFIED_SUCCESS")
        self.assertEqual(panel.calls.count("openttd.decision.get"), 2)
        self.assertEqual(log[-1]["msg"], "executed")
        self.assertIsNone(log[-1]["error"])

    def test_a_verdict_that_never_comes_is_returned_as_it_stands(self):
        tools = dict(VALID, **{
            "openttd.decision.submit": [{"status": "executed", "decision_id": 4}],
            "openttd.decision.get": [{"decision_id": 4, "verdict": "PENDING_OBSERVATION"}],
        })
        rec, _ = self.run_decide(FakePanel(tools))
        self.assertEqual(rec["verdict"], "PENDING_OBSERVATION")

    def test_a_submit_that_neither_runs_nor_waits_is_not_executed(self):
        tools = dict(VALID, **{"openttd.decision.submit": [{"status": "refused"}]})
        rec, log = self.run_decide(FakePanel(tools))
        self.assertEqual(rec["status"], "refused")
        self.assertEqual(log[-1]["msg"], "not executed")

    def test_an_approval_carried_out_by_the_server_is_read_not_resubmitted(self):
        tools = dict(VALID, **pending(), **{
            "openttd.decision.get": [{"decision_id": 9, "verdict": "VERIFIED_SUCCESS",
                                      "command_result": "rejected", "approval_id": 7,
                                      "error": "no money"}],
        })
        panel = FakePanel(tools,
                          approvals=[[{"id": 7, "state": "pending"}], [{"id": 7, "state": "consumed"}]],
                          decisions=[{"approval_id": 6, "decision_id": 1},
                                     {"approval_id": 7, "decision_id": 9}])
        rec, log = self.run_decide(panel)
        self.assertEqual(rec["decision_id"], 9)
        self.assertEqual(log[-1]["msg"], "rejected")
        self.assertEqual(log[-1]["error"], "no money")
        self.assert_never_approves_or_resubmits(panel)

    def test_an_approval_gone_from_the_queue_is_found_by_its_decision(self):
        tools = dict(VALID, **pending(), **{
            "openttd.decision.get": [{"decision_id": 5, "verdict": "VERIFIED_SUCCESS"}],
        })
        panel = FakePanel(tools, approvals=[[{"id": 8, "state": "pending"}]],
                          decisions=[{"approval_id": 7, "decision_id": 5}])
        rec, _ = self.run_decide(panel)
        self.assertEqual(rec["verdict"], "VERIFIED_SUCCESS")
        self.assert_never_approves_or_resubmits(panel)

    def test_a_gone_approval_without_a_decision_is_not_executed(self):
        panel = FakePanel(dict(VALID, **pending()), approvals=[[]], decisions=[])
        rec, log = self.run_decide(panel)
        self.assertEqual(rec, {"approval_id": 7, "approval": "gone", "status": "not_executed"})
        self.assertEqual(log[-1]["approval"], "gone")

    def test_an_expired_approval_is_not_executed(self):
        panel = FakePanel(dict(VALID, **pending()), approvals=[[{"id": 7, "expired": True}]])
        rec, _ = self.run_decide(panel)
        self.assertEqual(rec["approval"], "expired")
        self.assert_never_approves_or_resubmits(panel)

    def test_a_denied_approval_is_not_executed(self):
        panel = FakePanel(dict(VALID, **pending()), approvals=[[{"id": 7, "state": "denied"}]])
        rec, _ = self.run_decide(panel)
        self.assertEqual(rec["approval"], "denied")

    def test_an_approval_still_pending_at_the_deadline_says_so(self):
        panel = FakePanel(dict(VALID, **pending()), approvals=[[{"id": 7, "state": "pending"}]])
        rec, log = self.run_decide(panel, wait_s=3.0)
        self.assertEqual(rec["approval"], "pending")
        self.assertEqual(log[-1]["approval"], "still pending after 3 s")
        self.assert_never_approves_or_resubmits(panel)


if __name__ == "__main__":
    unittest.main()
