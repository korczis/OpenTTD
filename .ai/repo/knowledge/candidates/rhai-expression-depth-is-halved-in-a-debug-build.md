---
schema: knowledge/v1
id: rhai-expression-depth-is-halved-in-a-debug-build
kind: knowledge
class: constraint
title: "Rhai's default expression and call depth limits are half as deep in a debug build, so a deep script can parse in release and fail in debug"
description: "Before running the mcp-rhai tests in a debug build, or reading an ExprTooDeep / 'Expression exceeds maximum complexity' failure of a script that passes in release."
status: candidate
epistemics: inferred
date: 2026-11-22
tags: [rhai, mcp-rhai, testing, build-profile]
about: tools/research/mcp-rhai/Cargo.toml
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/mcp-rhai/Cargo.toml
    - file:tools/research/mcp-rhai/Cargo.lock
    - file:tools/research/mcp-rhai/src/main.rs
    - commit:364640205e1619753927db4287d25644911c2f2c
---

# Fact

Rhai picks its default limits by `debug_assertions` (rhai v1.27.0 `src/api/limits.rs`,
`default_limits`): in a debug build `MAX_EXPR_DEPTH` 32, `MAX_FUNCTION_EXPR_DEPTH` 16,
`MAX_CALL_STACK_DEPTH` 8; in a release build 64, 32 and 64. `tools/research/mcp-rhai` locks rhai
1.27.0 (`Cargo.lock`) and, at 194552a5fa, sets no limit of its own (no `set_max_expr_depths`
or `set_max_call_levels` anywhere under `tools/research/mcp-rhai/src`). A script that nests
deeper than 32 expressions or 16 inside a function — the rail steward and `rail_lib` the tests
in `src/main.rs` import (fixture at :703, the steward at :844) — is therefore refused by the parser (`ExprTooDeep`) only in a
debug build.

The failure itself is not recorded on origin/master; the mechanism is read from the source, so
the statement is inferred. Recorded validations of the crate run `cargo test --release`
(e.g. 364640205e: 13 suites, 489 passed; I0082, I0084).

# How to apply

Run the mcp-rhai tests with `--release`. A depth failure that appears only in debug is this limit,
not a script defect; raise the engine's limits explicitly if a debug run is needed.

# Review

Authored 2026-11-22 from rhai v1.27.0's source and a read of origin/master at 194552a5fa (the
brief also said I0058's validation uses `--release`; it does not, I0058 validates `mcp-hub`
without it). Promote with a recorded debug-build failure, or reject with the reason.
`majordomus knowledge promote rhai-expression-depth-is-halved-in-a-debug-build`, or
`majordomus knowledge reject rhai-expression-depth-is-halved-in-a-debug-build --reason ...`.
