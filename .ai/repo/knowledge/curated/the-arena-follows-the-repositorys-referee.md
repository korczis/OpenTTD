---
schema: knowledge/v1
id: the-arena-follows-the-repositorys-referee
kind: knowledge
class: convention
title: "The AI-battle arena runs the repository's current BattleReferee, so its matches log what the fitness rules judge (owner, 2026-11-25)"
description: "When provisioning or updating the ttd-battle arena, calibrating a fitness rule on arena matches, or wondering why arena match logs lack a field the referee logs."
status: verified
epistemics: decided
date: 2026-11-25
tags: [ai-battle, arena, referee, fitness, owner-answer]
about: tools/research/ai-battle/referee/BattleReferee/main.nut
provenance:
  origin: authored
  derived_from:
    - file:tools/research/ai-battle/referee/BattleReferee/main.nut
relations:
  - type: relates_to
    target: file:research/ai-battle/README.md
---

# Convention

The arena on the ttd-battle orchestrator runs the repository's current BattleReferee game script.
A referee change that adds a field, such as `loan=` in `REF Q`, reaches new arena matches, and a
fitness rule can be calibrated on them. Only extra fields come with it; the scoring is unchanged.

Before this, all 11,207 `match.log` files on the owner's Mac came from a referee older than
`loan=` (2026-11-23). The net-investment hoard rule (its design is in pull request #282,
https://github.com/korczis/OpenTTD/pull/282, held until calibration) could therefore not be
calibrated on arena data, because a bank balance that includes the loan reads every repayment as
investment.

# Evidence

- When: 2026-11-25.
- Asker: session 9ae82e7a-b0e4-44c0-a305-9c69cd1acf6a (openttd-7d), through AskUserQuestion,
  after the owner sent the Majordomus Fabric pack files again without an instruction.
- Question, verbatim: "The arena on the remote ttd-battle orchestrator runs an older BattleReferee
  that logs no loan (none of the 11,207 match logs here has loan=). Without it the net-investment
  hoard rule (#282) can't be calibrated on arena data. May the arena move to the repository's
  current referee?"
- Answer, verbatim: "Yes, update the referee (Recommended)". The option read: "The arena's
  BattleReferee.tar follows the repository's referee, so new matches log loan=. Its scoring is
  unchanged, only extra fields."
- The same answer is recorded in prismatic-majordomus's knowledge record
  `fabric-starts-with-p00-s00-s01`, which holds the whole question set.
