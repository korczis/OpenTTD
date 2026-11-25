---
schema: feature/v1
id: embedded-mcp-server
kind: feature
title: Embedded loopback MCP server with approved, argument-bound mutation
short_title: MCP server
headline: An agent reads live game state over MCP and changes it only with an operator's approval of the exact arguments, through the command system.
summary: OPTION_PRISMATIC_MCP_SERVER (requires research instrumentation) serves Streamable HTTP on loopback from the game loop; the DecisionEnvelope gates every mutation.
status: stable
weight: 50
areas: [agent-integration, observability]
audiences: [prismatic-integrator, coding-agent]
rules: [project.game-state-mutates-only-through-commands, project.research-code-is-gated-and-labelled, project.savegame-fields-are-version-gated]
adrs: [adr-0008]
docs: [research/prismatic-mcp/README.md]
related: [research-instrumentation, prismatic-bridge]
tags: [mcp, security, approval]
---

## What it does

Live resources, bearer auth, Origin rejection,
`openttd.decision.validate`, and an approval-gated `submit` that pauses the game via
`Command<Commands::Pause>::Post`.

## What it does not do

It is compiled into research builds only (`OPTION_RESEARCH_INSTRUMENTATION` and
`OPTION_PRISMATIC_MCP_SERVER` both ON) and listens on loopback only. It is not a stable API. The
read-path determinism A/B ran once, on one seed (I0026); no gate repeats it.

## Maturity

`stable`, graded 2026-11-25 against [the levels](README.md#maturity-levels).

- **shipped**: the option `cmake/Options.cmake:66:OPTION_PRISMATIC_MCP_SERVER`; an approval binds
  the tool and its canonical arguments, not only their digest
  (`src/prismatic_mcp/prismatic_mcp_detail.h:204:canonical_args`,
  `src/prismatic_mcp/prismatic_mcp_detail.h:214:ClassifySubmit`; I0023); the approved mutation is
  posted through the command system (`src/prismatic_mcp/prismatic_mcp_server.cpp:964:Post`); a
  company-scoped tool, `src/prismatic_mcp/prismatic_mcp_ai.cpp:354:set_setting`.
- **tested**: the approval state machine case
  `src/tests/test_prismatic_mcp.cpp:1040:TEST_CASE` (submit, approve, execute once, refuse replay,
  changed arguments and another tool), with the other `[prismatic_mcp]` cases of
  `src/tests/test_prismatic_mcp.cpp`.
- **gated**: the research tier `tools/gate.sh:715:cmd_research` runs them
  (`tools/gate.sh:748:openttd_test`) when the server is built; `tools/gate.sh change` names that tier
  for a change to research-gated code.
- **documented**: `research/prismatic-mcp/README.md`.
