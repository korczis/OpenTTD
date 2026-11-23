---
id: project.prismatic-ai-passes-the-headless-fitness-check
version: 1
kind: rule
title: A change to PrismaticAI or the battle tooling passes the headless fitness check before it is committed
description: tools/research/ai-battle/battle.py fitness runs the working tree's PrismaticAI in one short, deterministic null-driver match against a reference AI and fails on a Squirrel error from any PrismaticAI, an unfinished match, or a collapsing fleet of a contestant held healthy; tools/gate.sh change runs it whenever tools/research/prismatic-ai or tools/research/ai-battle changes.
statement: Do not commit a change under tools/research/prismatic-ai or tools/research/ai-battle unless battle.py fitness passes, or says NOT RUN because no arena cache exists on this machine and you say so.
status: active
class: blocking
depends_on: [project.never-report-unrun-validation-as-pass@1, project.battle-records-are-reproducible@1]
tags: [prismatic-ai, ai-battle, regression, fitness, enforcement]
---

# Rationale

PrismaticAI ran in the LAN AI battle with its fleet shrinking by a third to a half in every match,
and no test noticed (knowledge `prismatic-ai-without-a-steward-keeps-one-train-per-line`). The
bot's behaviour is tested by duels, which take wall-clock minutes per game month and run in no
gate. A null-driver match runs six game years in about ten seconds and is deterministic for one
tree and one binary, so it can run on every change to the bot.

# Required behaviour

- Run `tools/gate.sh change`, which runs `battle.py fitness` when the bot or the battle tooling
  changed, or run `battle.py fitness` directly. Do it before committing a change under
  `tools/research/prismatic-ai` or `tools/research/ai-battle`.
- A contestant whose health a change is meant to keep goes into `fitness.must_be_healthy` in
  `tools/research/ai-battle/battle.json`. Do not remove one to make a change pass.
- When the check says NOT RUN (no `build/openttd`, or no arena cache from `battle.py fetch` and
  `battle.py prepare`), report it as not run (rule `project.never-report-unrun-validation-as-pass`).

# Failure behaviour

`battle.py fitness` exits 10 in these cases:

- the match did not finish its final year,
- any PrismaticAI logged a Squirrel error,
- a contestant held healthy ended below `fitness.rules.fleet_keep` of its fleet peak (when that
  peak was at least `fleet_min_peak` vehicles),
- a contestant held healthy hoarded: in some window of `hoard_window` quarters its fleet stayed
  within `hoard_fleet_slack` of the window's start while its money grew `hoard_factor` times to
  at least `hoard_floor` (`hoards`; the owner's co-play game of 2026-11-22 held 35-37 vehicles
  from 1955 to 1960 while its bank rose from 0.6M to 3.77M), its stations not growing either.
  Only the contestants listed in `fitness.must_not_hoard` of battle.json fail on it (default:
  every healthy one); the list is read there, not repeated here, and a contestant joins it once
  the fitness seed shows it spending (research/ai-battle/README.md keeps the evidence). Do not
  remove a contestant from `must_not_hoard` to make a change pass.
- a PrismaticAI company kept more than `fitness.rules.trainless_max` rail stations without a
  train of its own through each of the last `trainless_reports` quarterly reports: the same
  stations for a game year, counted from the game in deity mode by the referee (`REF T`;
  `trainless_stations`). In the owner's live game of seed 15805 two finished lines stood without
  a train for a decade (research/incident-2026-11-23-trainless-lines.md). Do not raise
  `trainless_max`, or add a contestant to `trainless_skip` (only the test hook's own lineup is
  there), to make a change pass.
- a contestant listed in `fitness.must_run_trains` ended the match with no train (`no_trains`).
  The lineup `PrismaticAI trainless test` (setting `test_trainless=2`) refuses the first train
  of every new line and forgets the line each month. Only the bot's own recovery
  (KeepTrainlessLine, AdoptTrainlessLines, KeepLinesServed) can give it a train, so the check
  runs that recovery on every change.

The gate fails with it.

# Verification

`python3 tools/research/ai-battle/battle.py fitness`; the offline tests of the verdict in
`tools/research/test_ai_battle.py` (`Health`).
