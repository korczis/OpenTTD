---
schema: knowledge/v1
id: auto-approve-is-the-owners-switch
kind: knowledge
class: convention
title: "Auto-approve of MCP decisions is switched on only by the owner, never inferred by an agent"
description: "When launching tools/research/mcp-e2e, scripting the panel, or building an agent loop over the MCP approval flow."
status: verified
epistemics: decided
date: 2026-11-23
tags: [mcp, approval, oversight, agents]
about: tools/research/mcp-rhai/src/panel.rs
route: enforcement
provenance:
  origin: authored
  derived_from:
    - issue:I0031
    - issue:I0032
    - file:tools/research/mcp-rhai/src/panel.rs
    - commit:60aef3caa5529bbe769e8c5c423615aefb7d196a
relations:
  - type: relates_to
    target: file:.ai/repo/adrs/0008-an-agent-reaches-the-game-through-an-embedded-loopback-mcp-s.md
  - type: supports
    target: file:.ai/repo/rules/project/game-state-mutates-only-through-commands.v1.md
  - type: relates_to
    target: file:research/incident-2026-11-23-trainless-lines.md
---

# Convention

The approval of an MCP decision is the owner's act: the panel's Approve button or `/approve` in
the `~` console. The panel's auto-approve (`--auto-approve`, the window's switch,
`POST /api/auto-approve`) hands that act to the panel, so only the owner turns it on. An agent
never turns it on, not even on an inference from "full auto". A script or agent loop never
approves its own submissions: `ttd::set_pause` waits for the approval, and a peer session's
harness refused a loop that called `/api/approve` on its own decisions.

# What happened

On 2026-11-15 I relaunched the :8800 stack with `--auto-approve` on an inference, and session
openttd-59 raised it. The owner was asked: "When I relaunched the :8800 stack I passed
--auto-approve on my own, inferring it from your "full auto" and from your earlier toggle in the
panel. I have now switched it OFF. Should the running game approve its pending MCP decisions
automatically?" The answer was "Keep it off (Recommended)". `mcp-e2e` starts with auto-approve
off unless the owner passes the flag.

# The owner's answer of 2026-11-23: their own company's approvals settle by a countdown

Session 9ae82e7a-b0e4-44c0-a305-9c69cd1acf6a, about 03:15Z on 2026-11-23. Behind the question: in
the owner's live game of seed 15805, `replan.rhai` filed 199 `openttd.ai.set_setting` requests for
company 0 between 1952-05-07 and 1962-12-04. Every one was logged `pending approval`, none was
approved, and the bot played the plan of 1950-02-08 to the end
([incident 2026-11-23](../../../../research/incident-2026-11-23-trainless-lines.md)).

The question, asked in a reply, in Czech (translated here): "Should the panel approve the
re-plans for the co-play company by itself, or will you approve them?"

The first answer, verbatim: "sam" ("by itself"). Taken alone it reads as the panel's blanket
auto-approve. The session relayed it that way to the game's operator session and then withdrew
the relay within minutes, because a second message followed.

The owner's clarification, verbatim: "samozrejme potvrzuji jen sveho hrace ci sveho co-play hrace +
stale mame impmentovany ten countdown + deffault value, ne? konzistentne jako s "your bot asks""
("of course I confirm only my own player or my co-play player, and we still have that countdown and
a default value implemented, don't we? Consistently with 'your bot asks'").

The decision, therefore:

- **Not the blanket switch.** The panel's auto-approve stays off, as the convention above says.
  It would approve every pending request of the panel, for any company.
- **Only the owner's own company and the co-play company.** No other company's request is ever
  approved on the owner's behalf.
- **A countdown with a default, as "your bot asks" has** (the owner's order of 2026-11-16 in
  [`the-owner-can-auto-answer-the-bots-questions`](the-owner-can-auto-answer-the-bots-questions.md)).
  The owner sees the pending change, its countdown and its default, and can answer earlier. When
  the countdown runs out unanswered, the default applies, and for these companies the default is
  approve.

What exists on 2026-11-23 and what does not:

- There is a deadline: `SettleDueApprovals` in `src/prismatic_mcp/prismatic_mcp_server.cpp`. Its
  default is 30 game days with the verdict `keep`, which retires the request unenacted. The verdict
  `approve` exists (`PRISMATIC_MCP_APPROVAL_DEADLINE=30:approve`, `mcp-e2e --approval-deadline`).
- That `approve` verdict is not scoped to the owner's companies. `detail::ChatScoped` lets it
  execute any company's AI setting change and any company-level command.
- No window shows an approval's countdown. "Your bot asks" has a bar that empties; the control
  window lists the pending change without its deadline.

Closing those two gaps is the work this answer asks for. The design goes to review before any
code, under the owner's order of the same night to consult other models and peers first.
