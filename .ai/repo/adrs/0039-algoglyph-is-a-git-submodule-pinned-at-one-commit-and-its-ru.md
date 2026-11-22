---
schema: adr/v1
id: adr-0039
kind: adr
title: AlgoGlyph is a git submodule pinned at one commit, and its Rust kernels run in-process as native-rust catalogue entries
status: proposed
date: 2026-11-22
tags:
  - algoglyph
  - research-only
related:
  - file:.gitmodules
  - file:tools/research/algoglyph-pin
  - file:tools/research/vendor/algoglyph.pin.json
  - file:tools/research/mcp-rhai/src/algoglyph_native.rs
  - file:tools/research/mcp-rhai/src/hub_algoglyph.rs
  - file:.github/workflows/research-rust.yml
provenance:
  origin: authored
  derived_from:
    - issue:I0227
    - session:2845e7d0-8f0e-45ae-9513-920a15d66807
    - file:tools/research/algoglyph-pin
---

# 39. AlgoGlyph is a git submodule pinned at one commit, and its Rust kernels run in-process as native-rust catalogue entries

## Context

The owner's order, 2026-11-22, relayed verbatim into session
2845e7d0-8f0e-45ae-9513-920a15d66807 (I0227):

> algoglyph by mel byt ze git submodule a jeho rust interface integrovano do openttd rust

("AlgoGlyph should be a git submodule, and its Rust interface integrated into OpenTTD's Rust.")

Until then AlgoGlyph was whichever checkout a machine had under `~/dev`: the hub took
`~/dev/algoglyph-wt/main`, else `~/dev/algoglyph` (`hub_algoglyph.rs` `default_dir`), and
`tools/research/mcp-e2e` tried seven directories, keeping the first whose flow CLI listed the
four kernels the stack uses. The AlgoGlyph version a planner answer came from was a property of
the machine, not of the repository, and AlgoGlyph's Rust crate (`crates/algoglyph-kernels`) was not
reachable from the fork's Rust at all. The standing rule that AlgoGlyph is consumed from outside and
never edited by this fork's work (memory and a recorded decision) still holds.

## Decision

1. AlgoGlyph (github.com/korczis/algoglyph, public) is the git submodule
   `tools/research/vendor/algoglyph`, pinned at one commit of its own history. Nothing is committed
   into it. The pin is the only AlgoGlyph version the fork trusts.
2. The pin has one record, `tools/research/vendor/algoglyph.pin.json`: the pinned commit, the kernels
   the stack needs, and the flow runner's catalogue at that commit (`npm run --silent flow --
   kernels`). `tools/research/algoglyph-pin bump [REF]` is the only way to move it. It moves the
   gitlink, lists the runner's kernels at REF, refuses a commit that lacks a needed kernel, then
   writes the record and stages both paths. `algoglyph-pin check` and a unit test of mcp-rhai refuse
   a record whose commit is not the gitlink.
3. The default runner, in the hub (`hub_algoglyph::default_dir`) and in mcp-e2e (`algoglyph-pin
   runner`), is the first installed checkout that is cleanly at the pin: the submodule, else
   `~/dev/algoglyph-wt/main` or `~/dev/algoglyph`. A checkout at any other commit is passed over.
   The former fallbacks to the stacked feature worktrees are retired, because they serve versions
   the fork does not pin. `$MCP_ALGOGLYPH_DIR` still overrides, for an experiment against another
   version.
4. mcp-rhai depends on `crates/algoglyph-kernels` by path. Every kernel of its sorting registry that
   the runner does not list at the pin becomes an entry of the discovered catalogue with engine
   `native-rust`. Those entries are listed after the runner's, by `algoglyph.kernels` and Rhai
   `kernels()`. They are answered in-process by `algoglyph.kernel` and Rhai `kernel()`, in the
   runner's answer form. The runner keeps its own ids: a native id it also lists (today
   `topological-sort`) stays the runner's. `algoglyph-cli` has no library target, so its catalogue
   is reached through the flow CLI. `fluids` runs only its own seeded scenario and declares no form
   for caller data, so it is not an entry.
5. CI: the Research Rust workflow (`.github/workflows/research-rust.yml`), which already builds
   mcp-rhai, checks out the submodule. It runs when the pin changes, checks the pin, runs the pin
   script's tests, and builds against the pinned crate. Its binaries (mcp-plan and mcp-panel include
   the hub's AlgoGlyph module, and battlebrain) run the native tests.
   `tools/gate.sh change` checks the pin (it checks the submodule out at the pin when it is
   missing). When a change touches the pin or the native code, the gate also runs the native tests.

## Alternatives rejected

- **Keep discovering checkouts under `~/dev`.** The version would stay a property of the machine,
  and the Rust crate would still be unreachable.
- **Pin by a commit SHA in an environment variable or a script, without a submodule.** Cargo cannot
  take a path dependency on a crate it does not have, and a second copy of the SHA drifts.
- **A cargo git dependency on algoglyph-kernels.** It pins the Rust crate only, not the TypeScript
  runner, which serves the planner's kernels. The two would then be pinned at different places.
- **Native ids namespaced (`native/<id>`).** That would invent ids AlgoGlyph does not use. The
  runner-first precedence keeps AlgoGlyph's own ids and names the one collision.
- **Pin origin/main (30a56efeff, 71 commits later).** The package and crates are identical, but the
  fork's recorded answers (`tools/research/schema/fixtures/battlebrain/catalogue.json`) are at
  1b8e009. Bumping is one `algoglyph-pin bump`, checked like any change.

## Consequences

- Every cargo build of mcp-rhai needs the submodule's work tree. A worktree without one runs
  `tools/research/algoglyph-pin install --no-npm` once (network, about 9 MB), and `tools/gate.sh
  change` does that itself.
- The flow runner needs node_modules. In a worktree, the submodule does not have them until
  `algoglyph-pin install` runs, so a machine with one installed checkout at the pin serves every
  worktree. That avoids a 277 MB install per worktree under the disk floor.
- The planner's own kernels (knapsack-dp, track-planning, transport-investment, action-plan) have no
  Rust port in AlgoGlyph, so they stay on the flow runner. The native entries are, for now, the
  sorting kernels. Battlebrain sees them in the catalogue as unadapted kernels and never chooses one
  without an adapter.
- A bump is a reviewed change: the gitlink, the record and the CI run move together.
