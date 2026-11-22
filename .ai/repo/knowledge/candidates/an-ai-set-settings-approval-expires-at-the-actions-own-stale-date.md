---
schema: knowledge/v1
id: an-ai-set-settings-approval-expires-at-the-actions-own-stale-date
kind: knowledge
class: fact
title: "One openttd.ai.set_settings approval covers several AI settings and expires at its expires_tick; any other approval lives 100000 ticks (~3.7 game years)"
description: "Before applying several AI settings through the MCP server, or relying on an approval's lifetime to keep a decision from going stale."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [prismatic-mcp, approvals, battlebrain, ai-settings]
about: src/prismatic_mcp/prismatic_mcp_server.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/prismatic_mcp/prismatic_mcp_server.cpp
    - file:src/prismatic_mcp/prismatic_mcp_detail.h
    - commit:a4043ef2fbd8cbc2216776f683a9025c946ea1ac
relations:
  - type: relates_to
    target: file:.ai/repo/knowledge/curated/armed-input-can-approve-its-own-decisions.md
---

# Fact

At 194552a5fa, `prismatic_mcp_server.cpp:583` sets `APPROVAL_TTL_TICKS = 100000`. At 74 ticks a
day that is about 1351 game days, about 3.7 game years, so a per-setting `ai.set_setting`
approval stays usable long after the decision that asked for it.

`openttd.ai.set_settings` (PR #57, merge a4043ef2fb) changes up to `SET_SETTINGS_MAX` settings
of one company under one approval (`prismatic_mcp_detail.h:206-221`). It is the only action that
binds its own expiry (`expires_tick`, `BoundExpiresTick` at :255-258), and the approval expires
at the sooner of `now + APPROVAL_TTL_TICKS` and that tick (`ApprovalExpiry`, :262-266, applied at
`prismatic_mcp_server.cpp:1056-1058`).

At 194552a5fa the panel's decision executor (`tools/research/mcp-rhai/src/panel_decision.rs`) does
not yet call `set_settings`; binding a Battlebrain live decision to one expiring approval is PR #61,
open when this was written. Extend this record when it merges.

# How to apply

Apply a decision's settings through one `ai.set_settings` call with `expires_tick` set to the
decision's stale date; never through a chain of `ai.set_setting` approvals.

# Review

Authored 2026-11-22 from origin/master at 194552a5fa and PR #57; awaiting the owner's review.
`majordomus knowledge promote an-ai-set-settings-approval-expires-at-the-actions-own-stale-date`, or
`majordomus knowledge reject an-ai-set-settings-approval-expires-at-the-actions-own-stale-date --reason ...`.
