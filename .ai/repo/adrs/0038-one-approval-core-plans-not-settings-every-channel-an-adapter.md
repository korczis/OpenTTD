---
schema: adr/v1
id: adr-0038
kind: adr
title: "One approval core on the game server: a plan is approved as a whole and executes on approval, and every channel (web, console, game command, chat) is an adapter to it"
status: accepted
date: 2026-11-22
tags:
  - approval
  - panel
  - prismatic-mcp
  - web
related:
  - file:src/prismatic_mcp/prismatic_mcp_server.cpp
  - file:src/console_cmds.cpp
  - file:tools/research/mcp-rhai/src/panel.rs
  - file:tools/research/mcp-rhai/src/panel_play.rs
  - file:.ai/repo/rules/project/web-ui-is-covered-end-to-end.v2.md
provenance:
  origin: authored
  derived_from:
    - file:src/prismatic_mcp/prismatic_mcp_server.cpp
    - file:tools/research/mcp-rhai/src/panel.rs
---

# 38. One approval core on the game server: a plan is approved as a whole and executes on approval, and every channel is an adapter to it

## Context

On 2026-11-22 the owner, playing the live 8-company game, reported that approval worked from
nowhere: "potrebuji lepsi approval, nefunguje ani ze hry, ani z webui, odnikud" (I need a better
approval; it works neither from the game nor from the web UI, from nowhere). The live panel (:8800)
held 83 pending approvals and had recorded not a single operator approval. Four defects, each a
second implementation of something that already existed:

1. **Web UI.** The panel's POST guard (`panel.rs` `origin_ok`) accepted only
   `Origin: http://127.0.0.1:<port>`. The WebSocket bridge (`panel_play.rs` `origin_allowed`)
   already accepted `localhost` too. On a page opened as `localhost:8800` every button
   (Approve, Pause, Resume, Start AI, Auto-approve) got 403 "foreign origin", and the page's
   `api()` helper swallowed the status, so nothing on the page said so.
2. **The game.** `prismatic_mcp approve <id>` only marks the approval Approved. The action ran
   only when the original caller resubmitted the identical tool call, and only the panel did,
   and only for approvals that came through its own `/api/approve`. An approval typed in the game
   therefore never executed. On a multiplayer client the console runs on the client's own MCP
   server, which holds none of the server's approvals at all.
3. **Two queues.** The server keeps at most 64 approvals and drops the oldest. The panel kept its
   own copy, which it never reconciled with the server, so it offered ids the server had already
   forgotten.
4. **The unit.** A replan asks for a plan as N separate settings (18 in the owner's screenshot).
   Approving some of them leaves two plans mixed. A newer plan never retires the older one, so
   the queue only grows.

## Decision

The owner's answers (AskUserQuestion, session b8d35f8c-6c23-45c6-b8a9-99535e44864e, 2026-11-22,
verbatim):

- *How should an approval typed or clicked in the game reach the dedicated server (game command,
  chat command, web UI only)?* "all of above, but rocksolid and derived infered documented dynamic
  generic - nikoliv nekolik paralellni implementaci, odebirej, nikoliv nepridavej, redukuj, zhustuj
  kod - porad se s ostatnimi llms/ais/nodes/peers" (all of them, but rock solid, derived, inferred,
  documented, dynamic, generic; not several parallel implementations; remove, don't add; reduce
  and condense the code; consult the other LLMs, AIs, nodes and peers).
- *What is the unit of approval?* "Per plan".
- *LAN/web access to the panel and hub?* "Token outside loopback".

Therefore:

1. **One core, on the game server.** The approval queue lives only in the game server's MCP
   (`prismatic_mcp_server.cpp`). An approval is a **plan**: a label, a company and one or more
   actions. Approving a plan **executes** its actions there and then, on the main thread, through
   the command path. Nothing is resubmitted. A newer plan with the same label and company
   **supersedes** the pending older one. The panel keeps no queue of its own: it reads the
   server's.
2. **Every channel is an adapter to the one decision function.** None of them decides anything.
   - **Web UI:** POST `/api/approve` sends the console line on the server.
   - **Console:** on the server, `prismatic_mcp approve <plan>` decides. On a network client it
     posts the research-only game command, which the server carries out for the owner of the
     plan's company.
   - **Chat:** `!approve <plan>` and `!deny <plan>`, received by the server, take the same path.
   - **In-game window:** its buttons issue the console line.

   "No approve all" stays: a channel names the plan it approves.
3. **One web access policy for the panel and the hub**:
   - A POST, or a WebSocket upgrade, is accepted from its own origin under any host name (the
     `Origin` authority equals `Host`), or from an origin listed with `--allow-origin`, which
     gets full CORS (preflight, credentials, `Vary: Origin`).
   - The default bind stays 127.0.0.1, and `--bind` opens it to the LAN or the web.
   - Any request that is not loopback-to-loopback needs the token: from `?token=`, which sets
     an HttpOnly cookie, or a Bearer header. This includes a proxied request or a foreign
     `Host`, so it also blocks DNS rebinding.
   - The policy is one function. The POST guard and both WebSocket upgrades call it.
4. **No silent control.** The page's one API client surfaces every non-2xx answer to the person
   who clicked. The generic browser crawl (rule `project.web-ui-is-covered-end-to-end`) drives
   every control of every page and fails on a 4xx/5xx it caused, a page error or a control that
   does nothing.

## Invariants the redesign keeps

These come from the peer review by session openttd-94 (2026-11-22), the owner having asked for
peers to be consulted. Each one needs a test that proves it.

1. **Exact binding.** A plan's approval binds to a digest of its whole canonical `actions[]`,
   together with its company and label. A superseding plan never inherits an approval: approve
   plan A, submit A' with one action changed, and A' is still pending.
2. **Directive tracking.** The directive tracking in `panel_decision.rs` keeps its shape:
   - `plan.directive` carries the plan id;
   - the decision events carry a verdict per action;
   - `plan.directive_outcome` keeps per-step state and `approved_by`;
   - the one-hour expiry still applies.

   `tools/research/mcp-decision-check` (its stale, blind and keyed cases) and the Majordomus
   mirror (`panel_mesh.rs`) stay green.
3. **Idempotent pull.** A pull replayed under the same `key` (`POST /api/decision/pull`) creates
   no second plan and no second approval.
4. **Verification per action.** An action counts as applied only on `VERIFIED_SUCCESS`, read
   back from the game. A plan's outcome is applied, partial or failed, and never hides a failed
   action.
5. **Policy lines.**
   - Nothing approves its own request.
   - Auto-approve stays an explicit, recorded channel, off by default.
   - The embedded MCP server stays loopback-only and compile-gated. Only the panel and the hub
     widen, behind the token.
6. **A full queue refuses.** At the cap, a new plan is refused with a reason and an event on the
   bus. A pending plan is never evicted silently.

## How the channels were reduced (slice 2)

The owner asked for every channel (a game command, chat and the web UI), "but rock solid … not
several parallel implementations … reduce". A client reaches the server through chat, which is
the protocol's existing client-to-server message. One server-side adapter
(`PrismaticMCP::ChatApproval`) therefore serves two channels:

- the chat lines `!approvals`, `!approve <id>` and `!deny <id>`;
- the client console's `prismatic_mcp approvals|approve|deny`, which a network client sends as
  that same private line. The toolbar's "MCP approvals waiting" item runs this console line, so
  it comes through here too.

A dedicated command type would have changed the research build's network protocol and added a
second path to the same decision, so it was not added. A client may decide only approvals whose
arguments name its own company; anything else is the server operator's. The answer goes to the
asker alone, and the line is never broadcast.

## Alternatives rejected

- **Keep resubmit-to-execute and teach every channel to resubmit.** This adds a resubmitter per
  channel, which is exactly the parallel implementations the owner refused.
- **Approve per setting.** Partial approval mixes plans.
- **Open the API on the LAN without a token.** Anyone on the network could approve, run console
  lines and start AIs.

## Consequences

- The panel's pending map, its resubmit, and the second origin check are removed. So is the
  rhai scripts' per-setting approval bookkeeping, which becomes a plan submission.
- A client and a server must run the same research build, because the game command changes the
  research build's protocol. It is compile-gated with `OPTION_RESEARCH_INSTRUMENTATION`, like the
  rest of `src/prismatic_mcp/`.
- The slices land separately, each through a gated PR:
  1. the web access policy and the one API client;
  2. the approval core and its adapters;
  3. the generic crawl and the rule's element-level coverage.
