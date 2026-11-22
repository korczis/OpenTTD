# Rust performance benchmarks

How fast the Rust host runs the hot paths of the panel and the planner, measured by criterion
benchmarks in [`tools/research/mcp-rhai/benches/hot_paths.rs`](../tools/research/mcp-rhai/benches/hot_paths.rs),
recorded as `rust-bench/v1` JSON under [`results/`](results/) by
[`tools/research/rust-bench`](../tools/research/rust-bench), and checked for regressions by the
gate's `full` tier.

## What is measured

Each bench calls the crate's own function on the real game capture the Battlebrain tests replay
(`tools/research/schema/fixtures/battlebrain/captures/coplay-8800-1962-05-29`: a 256 x 256 map,
25 towns, the world graph) or on the fixtures the unit tests use. The crate has no library
target, so the bench compiles the modules in by path, as every binary of the crate does. The
groups:

- `json`: parsing large resource payloads (`openttd://world/graph`, `map/ways`, `map`).
- `planner`: the road graph (`graph.rs`), the passenger plan (`plan.rs`), and the whole planner
  pipeline (`plan_pipeline.rs`) without and with rail line pricing and transfer yards.
- `cargo`: freight pairs and the built-in portfolio knapsack (`cargo.rs`).
- `lines`: station sites (`sites.rs`), rail line pricing (`linecost.rs`) and yard proofs
  (`yards.rs`).
- `rhai`: compiling the panel's Rhai libraries, and running `rail_lib` and `director_lib` over
  bot records and events.
- `store`: named queries of the DuckDB store (`bus_sinks.rs` with `sql/queries/`) over a filled
  store file.

An AlgoGlyph kernel is never run. Where the planner calls one, the bench gives it a runner that
answers a straight plan at once, so the numbers are the planner's own cost, not the runner's
(an observer effect to keep in mind: a real `track-planning` call adds its own time). Each bench
is short, with flat sampling, so the suite runs in about a minute after the build.

## Run, record, check

```sh
tools/research/rust-bench run [--host NAME]     # cargo bench, then research/results/bench-<UTC time>-<rev8>.{json,md}
tools/research/rust-bench check [--threshold 0.25] [--record FILE]
tools/research/rust-bench render [--check]      # every record's page (tools/surfaces/surfaces sync runs it)
tools/research/rust-bench validate FILE...
```

Run it under a build slot (`~/dev/.build-slot-N`, see the workspace's `.build-slots.README`): the
first run builds the crate in the bench profile. `CARGO_TARGET_DIR` and `CARGO`/`RUSTC` are
honoured. The host name in a record is `--host`, else `RUST_BENCH_HOST`, else the first label of
the hostname: pass a neutral one when the hostname names a person.

`run` compares the new record with the newest committed record of the same host (name, OS and CPU
count), and writes each bench whose median grew by more than the threshold into its `baseline`.
`check` does the same for a fresh run, written nowhere, or for `--record FILE`, and exits 1 on any
such regression, printing each. Without a committed record of this host, or without cargo, it says
NOT RUN and exits 3; the `full` tier reports that as PARTIAL. Records of different hosts are never
compared: their numbers measure the machine as much as the code.

A record is committed like any other result: commit the `.json` and its derived `.md` together.
The page is never edited by hand; `tools/surfaces/surfaces check` (the `rust-bench` generator of
`surfaces.toml`) fails while one is stale.

## The record

The schema is [`tools/research/schema/rust-bench.v1.json`](../tools/research/schema/rust-bench.v1.json);
the tool validates every record it writes or reads against it. Times are nanoseconds per
iteration: `mean`, `median` and `stddev` are criterion's estimates, `min` and `max` the fastest and
slowest sample, `samples` their count. `throughput` is `null`, or `{kind, per_iteration}` with
`kind` one of `bytes`, `elements`, `bits` when the bench declares what one iteration processes.
`tree_dirty` says the crate directory held uncommitted changes when it was built.

## Where it runs

| Where | What | Outcome |
|---|---|---|
| `tools/gate.sh full` | `rust-bench check` against this host's newest record | fails on a regression; NOT RUN (PARTIAL) without a record of this host |
| `tools/gate.sh change` | nothing: the tier stays fast | NOT APPLICABLE |
| CI, `research-rust.yml` | `cargo bench --no-run`, the tool's tests, every record validated, every page current | the benchmarks are compiled, never run: hosted runners vary run to run |
