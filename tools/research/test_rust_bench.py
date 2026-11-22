#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for tools/research/rust-bench, offline: no cargo build, no benchmark run.

Run: python3 -B -m unittest tools/research/test_rust_bench.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/coverage/changed-lines does).

Each test builds a scratch git repository holding the real schema, and a fake `cargo` that writes
a criterion tree (benchmark.json, estimates.json, sample.json per bench) as `cargo bench` would.
What is tested is the tool itself, loaded from its file.
"""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCRIPT = HERE / "rust-bench"


def load_tool():
    loader = importlib.machinery.SourceFileLoader("rust_bench", str(SCRIPT))
    spec = importlib.util.spec_from_loader("rust_bench", loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


rb = load_tool()

FAKE_CARGO = """#!/usr/bin/env python3
# A fake `cargo bench`: writes the criterion tree of $FAKE_BENCH_SPEC under $CARGO_TARGET_DIR.
import json, os, sys
from pathlib import Path
spec = json.loads(os.environ.get("FAKE_BENCH_SPEC", "[]"))
target = Path(os.environ["CARGO_TARGET_DIR"])
for b in spec:
    new = target / "criterion" / b["group"] / b["name"] / "new"
    new.mkdir(parents=True, exist_ok=True)
    full = b["group"] + "/" + b["name"]
    (new / "benchmark.json").write_text(json.dumps({"group_id": b["group"], "function_id": b["name"],
        "value_str": None, "throughput": b.get("throughput"), "full_id": full, "directory_name": full, "title": full}))
    est = lambda v: {"point_estimate": v, "standard_error": 0.1, "confidence_interval": {"confidence_level": 0.95, "lower_bound": v, "upper_bound": v}}
    (new / "estimates.json").write_text(json.dumps({"mean": est(b["median"] * 1.01), "median": est(b["median"]),
        "median_abs_dev": est(1.0), "slope": None, "std_dev": est(b["median"] / 50)}))
    (new / "sample.json").write_text(json.dumps({"sampling_mode": "Flat", "iters": [2.0, 2.0, 2.0],
        "times": [b["median"] * 1.8, b["median"] * 2.0, b["median"] * 2.4]}))
sys.exit(int(os.environ.get("FAKE_CARGO_EXIT", "0")))
"""

FAKE_RUSTC = """#!/bin/sh
[ -n "$FAKE_RUSTC_FAIL" ] && { echo "no toolchain" >&2; exit 1; }
echo "rustc 1.90.0 (1159e78c4 2025-09-14)"
"""

SPEC = [{"group": "planner", "name": "pipeline", "median": 2.5e7},
        {"group": "json", "name": "parse_map", "median": 4.0e5, "throughput": {"Bytes": 270758}},
        {"group": "rhai", "name": "rail_lib_lines", "median": 900.0, "throughput": {"Elements": 120}}]


def git(root, *args):
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)


class Scratch(unittest.TestCase):
    """A scratch repository with the schema, a crate directory, a fake cargo and a fake rustc."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="rust-bench-test-"))
        self.root = self.tmp / "repo"
        (self.root / "tools/research/schema").mkdir(parents=True)
        shutil.copy(ROOT / rb.SCHEMA_FILE, self.root / rb.SCHEMA_FILE)
        (self.root / rb.CRATE).mkdir(parents=True)
        (self.root / rb.CRATE / "Cargo.toml").write_text("[package]\n")
        (self.root / ".gitignore").write_text("target/\n")
        git(self.root, "init", "-q")
        git(self.root, "-c", "user.name=t", "-c", "user.email=t@t", "add", ".")
        git(self.root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init")
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        for name, text in (("cargo", FAKE_CARGO), ("rustc", FAKE_RUSTC)):
            (bin_dir / name).write_text(text)
            (bin_dir / name).chmod(0o755)
        self.target = self.tmp / "target"
        self.env = {"CARGO": str(bin_dir / "cargo"), "RUSTC": str(bin_dir / "rustc"),
                    "CARGO_TARGET_DIR": str(self.target), "FAKE_BENCH_SPEC": json.dumps(SPEC), "RUST_BENCH_HOST": "h1"}
        patcher = mock.patch.dict(os.environ, self.env)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = rb.main(["--root", str(self.root), *argv])
        return code, out.getvalue(), err.getvalue()

    def records(self):
        return sorted((self.root / rb.RESULTS).glob("bench-*.json"))

    def commit_all(self):
        git(self.root, "-c", "user.name=t", "-c", "user.email=t@t", "add", ".")
        git(self.root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "record")

    def run_once(self, spec=None):
        if spec is not None:
            os.environ["FAKE_BENCH_SPEC"] = json.dumps(spec)
        code, out, err = self.main("run")
        self.assertEqual(code, 0, out + err)
        return json.loads(self.records()[-1].read_text()), out

    def slower(self, factor, which="planner"):
        return [dict(b, median=b["median"] * factor) if b["group"] == which else b for b in SPEC]


class Run(Scratch):
    def test_run_converts_criterion_estimates_into_one_valid_record_and_its_page(self):
        rec, out = self.run_once()
        self.assertEqual(rb.schema_errors(rb.load_schema(self.root), rec), [])
        self.assertEqual(rec["schema"], "rust-bench/v1")
        self.assertRegex(rec["id"], r"^bench-\d{8}T\d{6}-[0-9a-f]{8}$")
        self.assertEqual(rec["id"][-8:], rec["rev"][:8])
        self.assertTrue(rec["generated_at"].endswith("+00:00"))
        self.assertEqual((rec["tree_dirty"], rec["toolchain"], rec["profile"], rec["crate"]),
                         (False, "rustc 1.90.0", "bench", "tools/research/mcp-rhai"))
        self.assertEqual(rec["host"]["name"], "h1")
        self.assertEqual([b["id"] for b in rec["benches"]], ["json/parse_map", "planner/pipeline", "rhai/rail_lib_lines"])
        pipe = rec["benches"][1]
        self.assertEqual((pipe["group"], pipe["name"], pipe["unit"], pipe["median"], pipe["samples"]),
                         ("planner", "pipeline", "ns", 2.5e7, 3))
        self.assertEqual((pipe["min"], pipe["max"]), (2.5e7 * 0.9, 2.5e7 * 1.2))
        self.assertAlmostEqual(pipe["mean"], 2.5e7 * 1.01)
        self.assertEqual(rec["benches"][0]["throughput"], {"kind": "bytes", "per_iteration": 270758})
        self.assertEqual(rec["baseline"], {"id": None, "regressions": []})
        self.assertIn("3 benches; slowest planner/pipeline at 25 ms (median)", out)
        md = self.records()[-1].with_suffix(".md").read_text()
        self.assertIn(rb.DERIVED, md)
        self.assertIn("| `planner/pipeline` | 25 ms |", md)
        self.assertIn("None compared: this run had no baseline.", md)

    def test_a_second_run_is_compared_with_the_newest_committed_record_of_its_host(self):
        first, _ = self.run_once()
        self.commit_all()
        time.sleep(1.1)  # a new id, and criterion files newer than the first run's
        second, out = self.run_once(self.slower(1.5))
        self.assertEqual(second["baseline"]["id"], first["id"])
        self.assertEqual(second["baseline"]["regressions"],
                         [{"id": "planner/pipeline", "median": 3.75e7, "baseline_median": 2.5e7, "ratio": 1.5}])
        self.assertIn("REGRESSION planner/pipeline: median 37.5 ms against 25 ms (x1.5)", out)
        md = self.records()[-1].with_suffix(".md").read_text()
        self.assertIn(f"[{first['id']}]({first['id']}.md)", md)
        self.assertIn("| `planner/pipeline` | 37.5 ms | 25 ms | x1.5 |", md)

    def test_old_criterion_directories_of_benches_this_run_did_not_measure_are_left_out(self):
        stale = self.target / "criterion/gone/bench/new"
        stale.mkdir(parents=True)
        (stale / "estimates.json").write_text("{}")
        os.utime(stale / "estimates.json", (time.time() - 3600, time.time() - 3600))
        rec, _ = self.run_once()
        self.assertNotIn("gone/bench", [b["id"] for b in rec["benches"]])

    def test_an_uncommitted_change_in_the_crate_marks_the_tree_dirty(self):
        (self.root / rb.CRATE / "new.rs").write_text("")
        rec, _ = self.run_once()
        self.assertTrue(rec["tree_dirty"])
        self.assertIn("(uncommitted changes in the crate)", self.records()[-1].with_suffix(".md").read_text())

    def test_a_relative_target_dir_is_the_crates(self):
        os.environ["CARGO_TARGET_DIR"] = "tgt"
        self.run_once()
        self.assertTrue((self.root / rb.CRATE / "tgt/criterion/planner/pipeline/new/estimates.json").exists())

    def test_without_cargo_the_run_is_not_run(self):
        os.environ["CARGO"] = str(self.tmp / "no-such-cargo")
        code, out, _ = self.main("run")
        self.assertEqual(code, rb.EXIT_NOT_RUN)
        self.assertIn("NOT RUN -- ", out)
        self.assertEqual(self.records(), [])

    def test_a_failing_cargo_a_silent_cargo_and_a_failing_rustc_fail_the_run(self):
        os.environ["FAKE_CARGO_EXIT"] = "101"
        code, _, err = self.main("run")
        self.assertEqual((code, "cargo bench failed" in err), (1, True))
        os.environ["FAKE_CARGO_EXIT"] = "0"
        os.environ["FAKE_BENCH_SPEC"] = "[]"
        os.environ["CARGO_TARGET_DIR"] = str(self.tmp / "empty-target")
        code, _, err = self.main("run")
        self.assertEqual((code, "wrote no estimates" in err), (1, True))
        os.environ["FAKE_BENCH_SPEC"] = json.dumps(SPEC)
        os.environ["FAKE_RUSTC_FAIL"] = "1"
        code, _, err = self.main("run")
        self.assertEqual((code, "rustc --version: no toolchain" in err), (1, True))
        self.assertEqual(self.records(), [])

    def test_outside_git_the_run_fails(self):
        shutil.rmtree(self.root / ".git")
        code, _, err = self.main("run")
        self.assertEqual(code, 1)
        self.assertIn("git rev-parse HEAD", err)


class Check(Scratch):
    def baseline(self):
        rec, _ = self.run_once()
        self.commit_all()
        time.sleep(1.1)
        return rec

    def crafted(self, factor):
        """A record file outside the results: the committed one, slower by `factor`, as a new run."""
        rec = json.loads(self.records()[-1].read_text())
        rec["id"] = rec["id"][:6] + "99991231T235959" + rec["id"][21:]
        for b in rec["benches"]:
            b["median"] *= factor
        path = self.tmp / "crafted.json"
        path.write_text(json.dumps(rec))
        return path

    def test_check_passes_within_the_threshold_and_fails_on_a_crafted_regression(self):
        base = self.baseline()
        code, out, _ = self.main("check", "--record", str(self.crafted(1.2)))
        self.assertEqual(code, 0, out)
        self.assertIn(f"against {base['id']}, 3 benches in both, threshold 25%", out)
        self.assertIn("rust-bench check: PASS", out)
        code, out, _ = self.main("check", "--record", str(self.crafted(1.3)))
        self.assertEqual(code, 1, out)
        self.assertEqual(out.count("REGRESSION "), 3)
        self.assertIn("rust-bench check: FAIL (3 regressions)", out)
        code, out, _ = self.main("check", "--threshold", "0.5", "--record", str(self.crafted(1.3)))
        self.assertEqual(code, 0, out)

    def test_check_runs_the_benches_afresh_and_writes_no_record(self):
        self.baseline()
        os.environ["FAKE_BENCH_SPEC"] = json.dumps(self.slower(2.0, "rhai"))
        code, out, _ = self.main("check")
        self.assertEqual(code, 1, out)
        self.assertIn("REGRESSION rhai/rail_lib_lines: median 1.8 µs against 900 ns (x2.0)", out)
        self.assertEqual(len(self.records()), 1, "a check writes no record")

    def test_a_bench_in_only_one_record_is_not_compared(self):
        self.baseline()
        os.environ["FAKE_BENCH_SPEC"] = json.dumps([{"group": "new", "name": "bench", "median": 1.0}] + SPEC)
        code, out, _ = self.main("check")
        self.assertEqual(code, 0, out)
        self.assertIn("3 benches in both", out)

    def test_without_a_committed_record_of_the_host_the_check_is_not_run(self):
        code, out, _ = self.main("check")
        self.assertEqual(code, rb.EXIT_NOT_RUN)
        self.assertIn("rust-bench check: NOT RUN -- no committed record of host h1", out)
        # An uncommitted record is no baseline, and neither is the record itself or another host's.
        self.run_once()
        self.assertEqual(self.main("check")[0], rb.EXIT_NOT_RUN)
        self.commit_all()
        self.assertEqual(self.main("check", "--record", str(self.records()[-1]))[0], rb.EXIT_NOT_RUN)
        code, out, _ = self.main("check", "--host", "h2")
        self.assertEqual(code, rb.EXIT_NOT_RUN)
        self.assertIn("no committed record of host h2", out)

    def test_an_invalid_record_fails_the_check(self):
        bad = self.tmp / "bad.json"
        bad.write_text(json.dumps({"schema": "rust-bench/v1"}))
        code, _, err = self.main("check", "--record", str(bad))
        self.assertEqual(code, 1)
        self.assertIn("missing required property 'benches'", err)


class Render(Scratch):
    def test_render_writes_every_page_and_check_finds_a_stale_one(self):
        self.run_once()
        md = self.records()[-1].with_suffix(".md")
        self.assertEqual(self.main("render", "--check")[0], 0)
        md.write_text("hand edit\n")
        code, out, _ = self.main("render", "--check")
        self.assertEqual(code, rb.EXIT_STALE)
        self.assertIn(f"stale {md.relative_to(self.root)}", out)
        self.assertEqual(md.read_text(), "hand edit\n", "--check writes nothing")
        md.unlink()
        code, out, _ = self.main("render")
        self.assertEqual(code, 0)
        self.assertIn("wrote ", out)
        self.assertIn(rb.DERIVED, md.read_text())
        self.assertEqual(self.main("render", "--check")[0], 0)

    def test_the_page_names_a_baseline_without_regressions(self):
        rec = json.loads(json.dumps(VALID))
        rec["baseline"] = {"id": "bench-20261122T000000-0123abcd", "regressions": []}
        self.assertIn("None: no median grew by more than the run's threshold.", rb.page(rec))

    def test_times_and_rates_are_written_with_their_units(self):
        self.assertEqual([rb.fmt_ns(x) for x in (12.0, 1.5e3, 2.5e7, 3.2e9)], ["12 ns", "1.5 µs", "25 ms", "3.2 s"])
        b = lambda kind, n, median: {"median": median, "throughput": {"kind": kind, "per_iteration": n}}
        self.assertEqual(rb.fmt_rate({"median": 1.0, "throughput": None}), "")
        self.assertEqual(rb.fmt_rate(b("bytes", 270758, 4.0e5)), "677 MB/s")
        self.assertEqual(rb.fmt_rate(b("bytes", 2_000_000_000, 1e9)), "2 GB/s")
        self.assertEqual(rb.fmt_rate(b("elements", 120, 900.0)), "133 Melem/s")
        self.assertEqual(rb.fmt_rate(b("elements", 3, 1e6)), "3 kelem/s")
        self.assertEqual(rb.fmt_rate(b("bits", 1, 1e9)), "1 bit/s")
        self.assertEqual(rb.fmt_rate(b("bits", 1, 0.0)), "0 bit/s")


VALID = {"schema": "rust-bench/v1", "id": "bench-20261122T231500-ab12cd34", "generated_at": "2026-11-22T23:15:00+00:00",
         "rev": "ab12cd34" * 5, "tree_dirty": False, "host": {"name": "h", "os": "macos", "cpus": 18},
         "toolchain": "rustc 1.90.0", "profile": "bench", "crate": "tools/research/mcp-rhai",
         "benches": [{"id": "planner/plan_fixture", "group": "planner", "name": "plan_fixture", "unit": "ns",
                      "mean": 123.4, "median": 120.1, "stddev": 3.2, "min": 110.0, "max": 140.0, "samples": 100,
                      "throughput": None}],
         "baseline": {"id": None, "regressions": [{"id": "planner/plan_fixture", "median": 120.1, "baseline_median": 90.0, "ratio": 1.334}]}}


class Schema(unittest.TestCase):
    schema = rb.load_schema(ROOT)

    def errors(self, edit):
        rec = json.loads(json.dumps(VALID))
        edit(rec)
        return rb.schema_errors(self.schema, rec)

    def test_the_contracts_example_is_valid(self):
        self.assertEqual(self.errors(lambda r: None), [])

    def test_every_violation_is_named_with_its_path(self):
        def drop(key):
            return lambda r: r.pop(key)
        cases = [
            (drop("rev"), "$: missing required property 'rev'"),
            (lambda r: r.update(extra=1), "$: unexpected property 'extra'"),
            (lambda r: r.update(schema="rust-bench/v2"), "$.schema: expected 'rust-bench/v1'"),
            (lambda r: r.update(id="bench-x"), "$.id: 'bench-x' does not match"),
            (lambda r: r.update(tree_dirty="no"), "$.tree_dirty: expected boolean, got str"),
            (lambda r: r["host"].update(cpus=0), "$.host.cpus: 0 is below 1"),
            (lambda r: r["host"].update(cpus=True), "$.host.cpus: expected integer, got bool"),
            (lambda r: r["host"].update(name=""), "$.host.name: shorter than 1"),
            (lambda r: r.update(benches=[]), "$.benches: fewer than 1 items"),
            (lambda r: r["benches"][0].update(throughput={"kind": "furlongs", "per_iteration": 1}),
             "$.benches[0].throughput.kind: 'furlongs' is not one of"),
            (lambda r: r["baseline"].update(id=7), "$.baseline.id: expected string/null, got int"),
        ]
        for edit, want in cases:
            errs = self.errors(edit)
            self.assertTrue(any(e.startswith(want) for e in errs), f"{want!r} not in {errs}")

    def test_a_keyword_the_validator_does_not_implement_is_an_error(self):
        self.assertEqual(rb.schema_errors({"oneOf": []}, 1), ["$: schema keyword(s) ['oneOf'] not implemented"])

    def test_validate_names_each_file(self):
        with tempfile.TemporaryDirectory() as d:
            good, bad, broken = Path(d, "good.json"), Path(d, "bad.json"), Path(d, "broken.json")
            good.write_text(json.dumps(VALID))
            bad.write_text(json.dumps(dict(VALID, profile="release")))
            broken.write_text("{")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(rb.main(["--root", str(ROOT), "validate", str(good)]), 0)
            self.assertIn("valid rust-bench/v1", out.getvalue())
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                self.assertEqual(rb.main(["--root", str(ROOT), "validate", str(bad)]), 1)
                self.assertEqual(rb.main(["--root", str(ROOT), "validate", str(broken)]), 1)
            self.assertIn("$.profile: expected 'bench'", err.getvalue())
            self.assertIn("broken.json: Expecting property name", err.getvalue())
            # As a program, too: what the gate and CI run.
            p = subprocess.run([sys.executable, "-B", str(SCRIPT), "validate", str(good)], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)

    def test_every_committed_record_is_valid(self):
        for path in sorted((ROOT / rb.RESULTS).glob("bench-*.json")):
            rb.load_record(ROOT, path)


class Criterion(unittest.TestCase):
    def test_every_throughput_criterion_writes_becomes_a_kind_and_an_amount(self):
        self.assertIsNone(rb.throughput_of(None))
        self.assertEqual(rb.throughput_of({"Bytes": 5}), {"kind": "bytes", "per_iteration": 5})
        self.assertEqual(rb.throughput_of({"BytesDecimal": 6}), {"kind": "bytes", "per_iteration": 6})
        self.assertEqual(rb.throughput_of({"Elements": 7}), {"kind": "elements", "per_iteration": 7})
        self.assertEqual(rb.throughput_of({"Bits": 8}), {"kind": "bits", "per_iteration": 8})
        self.assertEqual(rb.throughput_of({"ElementsAndBytes": {"elements": 9, "bytes": 90}}),
                         {"kind": "elements", "per_iteration": 9})

    def test_a_bench_directory_without_its_files_or_samples_fails(self):
        with tempfile.TemporaryDirectory() as d:
            new = Path(d)
            with self.assertRaisesRegex(rb.Failure, "benchmark.json"):
                rb.read_bench(new)
            (new / "benchmark.json").write_text(json.dumps({"group_id": "g", "full_id": "other", "throughput": None}))
            est = {"point_estimate": 1.0}
            (new / "estimates.json").write_text(json.dumps({"mean": est, "median": est, "std_dev": est}))
            (new / "sample.json").write_text(json.dumps({"iters": [0.0], "times": [1.0]}))
            with self.assertRaisesRegex(rb.Failure, "no sample"):
                rb.read_bench(new)
            (new / "sample.json").write_text(json.dumps({"iters": [1.0], "times": [1.0]}))
            b = rb.read_bench(new)
            self.assertEqual((b["id"], b["group"], b["name"]), ("other", "g", "other"), "a full id outside its group stays whole")

    def test_the_host_name_comes_from_the_flag_the_environment_or_the_hostname(self):
        with mock.patch.dict(os.environ, {"RUST_BENCH_HOST": "env-host"}):
            self.assertEqual(rb.host_of("flag")["name"], "flag")
            self.assertEqual(rb.host_of(None)["name"], "env-host")
        with mock.patch.dict(os.environ, {"RUST_BENCH_HOST": ""}), mock.patch.object(rb.socket, "gethostname", return_value="box.local"):
            h = rb.host_of(None)
        self.assertEqual(h["name"], "box")
        self.assertGreaterEqual(h["cpus"], 1)
        with mock.patch.object(rb.sys, "platform", "darwin"):
            self.assertEqual(rb.host_of("x")["os"], "macos")


if __name__ == "__main__":
    unittest.main()
