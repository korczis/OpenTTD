---
id: project.decision-authority-boundaries
version: 1
kind: rule
title: One component decides a decision type, and the boundaries around it are machine-checked
description: For a decision type Battlebrain governs (today the freight line), only battlebrain writes an executable decision. Kernel and planner outputs are candidates, the blackboard never acts on a game, no catalogue is copied by hand, every decision carries its provenance, policy and snapshot, stale inputs are re-checked before acting, a fallback solver is named as one, and research code stays out of a default build. Each boundary has an owner and a test or gate step, or says why it has none.
statement: Do not add a second writer of an executable decision, a path from a kernel output to a game, a command path from the blackboard, a hand copy of the MCP or kernel catalogue, or a fallback labelled as the kernel; keep cargo test --bin battlebrain and tools/gate.sh change passing, and when a boundary must move, edit its named list, this rule and ADR 0037 in the same change.
status: active
class: blocking
depends_on: [project.game-state-mutates-only-through-commands@1, project.mcp-surfaces-are-derived@1, project.research-code-is-gated-and-labelled@1, project.never-report-unrun-validation-as-pass@1]
tags: [battlebrain, decision-authority, architecture, research]
---

# Rationale

[ADR 0037](../../adrs/0037-battlebrain-in-this-fork-is-the-one-authority-for-the-freigh.md) makes
`battlebrain` the one authority for the freight-line decision. The audit of 2026-11-22 behind it
found eleven components choosing an investment in a live game. A rule a reviewer has to remember
does not stop a twelfth one, so each boundary below is a test or a gate step that fails when the
boundary is crossed. A place the tree already crosses on purpose is an entry of a named list in
the test, with its reason. An entry that no longer matches anything fails too, so a list only
shrinks by being edited.

The tests are in `tools/research/mcp-rhai/src/authority_boundaries.rs`, compiled into the
`battlebrain` binary's tests. `tools/gate.sh change` runs them (offline) and checks the default
binary it just built (B8).

# Required behaviour

| # | Boundary | Owner (component) | Validator |
|---|---|---|---|
| B1 | Exactly one component writes an executable decision for a decision type: only `battlebrain` builds a `decision` record or a `battlebrain-decision/v1` block. | `battlebrain` (`tools/research/mcp-rhai/src/bin/battlebrain.rs`); the contract `tools/research/mcp-rhai/src/battlebrain_contract.rs` | `only_the_authority_writes_a_decision_and_never_a_snapshot_or_outcome` (the contract refuses any other writer with `writer_not_allowed`); `b1_only_battlebrain_builds_a_decision_record_or_block` (no other code line names `battlebrain-decision/`, `DECISION_SCHEMA`, `AUTHORITY` or `record("decision"`) |
| B2 | A kernel output is never an executable command. Planner and AlgoGlyph outputs become candidate records; only battlebrain turns a candidate into a decision. | `battlebrain`; the executor `tools/research/mcp-rhai/src/panel_decision.rs` | `a_shadow_run_writes_valid_records_and_a_shadow_block_only` (the planner's output is a `candidate` by `mcp-plan`, the decision is battlebrain's); `a_shadow_or_broken_decision_block_never_passes_the_executor`; `b2_freight_slots_are_written_only_by_the_authority_and_the_named_legacy_paths` (who may name an `fr_*` slot) |
| B3 | A blackboard write never changes the game. The mesh record client and the bus have no command path. | `tools/research/mcp-rhai/src/mesh_records.rs`, `tools/research/mcp-rhai/src/panel_mesh.rs`, `tools/research/mcp-rhai/src/board.rs`, `tools/research/mcp-rhai/src/bus_sinks.rs`, `tools/research/mcp-rhai/src/bin/mcp-bus-blackboard.rs`, `tools/research/mcp-rhai/src/bin/mcp-bus-tail.rs` | `b3_a_blackboard_write_has_no_command_path` (none of these files names a game tool, an acting panel route, `set_bot_setting`, `start_bot`, `.tool(` or rcon; every `tools/call` is a `majordomus_` tool) |
| B4 | No hand-kept copy of the MCP or kernel catalogue. The only per-kernel code is the adapter registry of domain checks. | the registry: `domain_check` in `battlebrain_contract.rs`, owned by `battlebrain`; MCP surfaces: rule `project.mcp-surfaces-are-derived` | `b4_no_list_literal_copies_the_kernel_or_tool_catalogue`; `b4_the_adapter_registry_is_the_named_domain_checks`; `a_kernel_outside_the_discovered_catalogue_is_unknown` |
| B5 | Every decision record carries provenance, policy identity and its snapshot. | the contract (`validate_record`) | `every_invalid_fixture_is_refused_with_its_code` (`decision-not-derived-from-its-snapshot.json`, `decision-by-the-planner.json`, ...); `b5_a_decision_without_provenance_policy_or_snapshot_is_refused` |
| B6 | Stale dependencies are revalidated before execution. | the executor (`decision_gate`, `freshness` in `panel_decision.rs`); `battlebrain live` | `a_shadow_or_broken_decision_block_never_passes_the_executor` (`key_mismatch`, `stale_decision`); `a_directive_is_applied_only_against_a_current_observation`; `a_live_decision_passes_only_its_gates_and_carries_one_slot` (`stale_companies`, `incoherent_snapshot`); `b6_the_executor_checks_block_and_freshness_before_any_step` (the order in `pull_with`) |
| B7 | A fallback solver is never labelled as the kernel. | `tools/research/mcp-rhai/src/cargo.rs` (`BUILTIN_SOLVER`) | `the_builtin_fallback_is_never_labelled_as_the_kernel` (`cargo test --bin mcp-plan`); `b7_the_fallback_label_and_battlebrains_fallback_metric_agree` |
| B8 | Research-only code is not in a default or release build. | `cmake/Options.cmake`; `tools/gate.sh` | `b8_research_options_default_off`; `tools/gate.sh change` steps `research_code_is_guarded` (every `src/` code line naming `prismatic` or `research` is inside a research guard) and `default_build_free_of_research` (`strings` on the built default binary finds no string naming `prismatic` or `research`, paths, version string and bare StringID names aside); the items a default build still carries, and why, are listed in rule `project.research-code-is-gated-and-labelled`; `tools/gate.sh research` (the research binary does carry `research_status`); rule `project.research-code-is-gated-and-labelled` |

## The named lists

- **B1.** `tools/research/mcp-rhai/src/battlebrain_contract.rs` (defines and validates) and
  `tools/research/mcp-rhai/src/bin/battlebrain.rs` (builds).
  The executor only validates a block (`decision_gate` calls `validate_decision_block`). A mesh
  `decision` record that `panel_mesh.rs` mirrors from `plan.directive` is a journal entry of what
  the executor did. It is not executable and carries no decision block, and B1 checks that.
- **B2.** Who may name a freight slot (`fr_*`):
  - the authority's live directive, and the contract's `FREIGHT_SETTINGS` and
    `freight_slot_prefix` (the one spelling of a slot name; a call to it names a slot);
  - the executor `panel_decision.rs`, which names a slot only to refuse a directive without a
    decision block that sets one (`fr_needs_decision`, I0117);
  - PrismaticAI's `info.nut`, `main.nut` and `rail.nut`, which declare and read the bot's own
    settings;
  - two **legacy paths, which are gaps until the cutover**: `panel_bot.rs` `settings_from_plan`
    (the planner's portfolio handed to the bot at `start_ai`, the first row of the authority
    matrix in `research/battlebrain/README.md`) and `intent.rs` (the answer to a plain-language
    request names a slot; its settings reach the bot through the approval-gated
    `openttd.ai.set_setting` or a later `start_ai`).
- **B4.** The adapter registry is `domain_check` for `knapsack-dp` and `track-planning`. The
  runner's outputs carry no schema, so these adapters sit at a protocol boundary on purpose. A
  kernel without one may not price or select (`no_domain_check`). One list is allowed: the
  tools `mcp-human` needs, in `play.rs` `require_tools`, because it is checked against the
  client's live tools/list before the hand acts.

# Failure behaviour

A failing test or gate step blocks the change. To move a boundary, edit its named list in
`authority_boundaries.rs`, this rule and, where the authority itself changes, ADR 0037, in the
same change.

What is not machine-checked, and why:

- **B2 in a running game.** The executor still applies a directive that carries no decision
  block, as before I0096, unless it sets a freight slot (`fr_needs_decision`, I0117). The legacy
  path in `panel_bot.rs` does not pass the executor: it can still hand the bot freight lines
  at `start_ai` that no battlebrain decision chose. The test stops a new path; only the cutover (Phase 04)
  removes the old ones.
- **B2 for decision types outside the slice.** Other components that consume kernel outputs
  (the Rhai rail steward's `knapsack-dp` in `tools/research/mcp-panel/rhai/rail_lib.rhai`,
  `invest.rs` with `transport-investment`, `mcp-human` with `track-planning`) choose things
  Battlebrain does not govern yet. The boundary applies to a decision type once Battlebrain
  owns it.
- **B3 at the language level.** The scan reads source text. A blackboard file could still reach
  a game through a function it imports from elsewhere. Review owns that case.
- **B4 for MCP read sets.** A function that names the resources it reads (for example
  `graph.rs` `derived_from`) is a dependency, not a catalogue copy, so URIs are not scanned.
  The MCP surfaces stay with `project.mcp-surfaces-are-derived` and its live checks.
- **B7 at the kernel boundary.** `cargo.rs` adopts a `knapsack-dp` answer without
  `domain_check`. `cargo.rs` is also compiled into `mcp-intent`, which does not link the
  contract, so the call is not a one-line change. Battlebrain's own path does not trust the
  label alone: it reports `fallback_solver` and `missing_kernel`.

# Verification

```bash
cargo test --release --offline --manifest-path tools/research/mcp-rhai/Cargo.toml --bin battlebrain --bin mcp-plan
tools/gate.sh change
```

# Related

ADR 0037, ADR 0004, `research/battlebrain/README.md`, I0095, I0096, I0097.
