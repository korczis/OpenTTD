---
schema: knowledge/v1
id: console-lines-the-panel-parses-are-an-interface
kind: knowledge
class: lesson
title: "The panel parses console output, so a reworded console line in src/ can stop every stack; run mcp-e2e after changing one"
description: "Before changing the text of a console line that tools/research parses (the token-file line, the MCP endpoint, approvals)."
status: verified
epistemics: observed
date: 2026-11-22
tags: [prismatic-mcp, console, panel, verification]
about: src/console_cmds.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/console_cmds.cpp
    - file:tools/research/mcp-rhai/src/panel.rs
    - commit:60aef3caa5529bbe769e8c5c423615aefb7d196a
    - commit:5a26a6189ea7d3e7616b946283ee0e77dd849dbf
    - commit:812defb0ce76d1af188bbc5a2a052a82c94f90a8
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/prismatic-mcp/README.md
---

# Statement

The panel (then the Python server.py, removed in 60aef3caa5; now `tools/research/mcp-rhai/src/panel.rs`) reads the dedicated server's console and takes the MCP bearer
token from the `Bearer token (...): <hex>` line (`TOKEN_RE` in server.py; `panel.rs:find_token` now). Commit 5a26a6189e (I0061) reworded that
line in `src/console_cmds.cpp:ConPrismaticMcp`. Its research gate passed, because the gate builds and
unit-tests the server but never starts the panel. Every stack started from that master then reported
"MCP did not come up", until 812defb0ce made the panel match the line by its shape.

Since red-team finding 8 (branch fix/mcp-token-never-printed) the game prints no token at all:
`prismatic_mcp start` and `rotate` print `Token written to <path> (mode 0600)`, and the panel
(`panel.rs:find_token_file`, used by `redact`) reads the token from that file. That line is now the
interface; `find_token` stays only to redact a token line an older game would print.

# What to do

A console line that a tool parses is an interface. Change its text only with the parser in the same
commit, and verify the change with a running stack (`tools/research/mcp-e2e --no-gui --no-browser`
with explicit free ports until "MCP up"), not with `tools/gate.sh research` alone.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- Revised on branch fix/mcp-token-never-printed: src/console_cmds.cpp:1983 and :2029 print "Token written to {} (mode 0600)" (start, rotate); :2019 `prismatic_mcp token` prints "Token file: {} (mode 0600, fingerprint {})". No line prints the token.
- tools/research/mcp-rhai/src/panel.rs:377 find_token_file matches "Token written to <path> (mode 0600)"; redact reads the token from that file; :799 runs it on every console line. find_token (:355) still matches the old "Bearer token (<anything>): <hex>" shape for redaction.
- History: 5a26a6189e reworded the line, 812defb0ce made the panel match it by shape, 60aef3caa5 removed the Python panel (git log).
- tools/research/mcp-e2e:17, :52-53 take --no-gui and --no-browser; :181 prints "MCP up".
