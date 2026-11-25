---
schema: adr/v1
id: adr-0041
kind: adr
title: "The owner's bulk consent and standing approval rules: approve or deny all over a filter, and always-allow or always-deny rules of a tool glob and a JSON matcher, applied in the one approval core"
status: proposed
date: 2026-11-23
tags:
  - approval
  - prismatic-mcp
  - panel
related:
  - file:.ai/repo/adrs/0038-one-approval-core-plans-not-settings-every-channel-an-adapter.md
  - file:.ai/repo/knowledge/curated/auto-approve-is-the-owners-switch.md
  - file:src/prismatic_mcp/prismatic_mcp_rules.h
  - file:src/prismatic_mcp/prismatic_mcp_server.cpp
  - file:src/prismatic_mcp/prismatic_mcp_panel.cpp
  - file:src/console_cmds.cpp
provenance:
  origin: authored
  derived_from:
    - file:src/prismatic_mcp/prismatic_mcp_rules.h
    - file:src/prismatic_mcp/prismatic_mcp_server.cpp
---

# 41. The owner's bulk consent and standing approval rules

## Context

ADR 0038 kept one approval core and one id per consent: "No approve all stays: a channel names
the plan it approves", and `prismatic_mcp.h` said "there is deliberately no bulk approval anywhere
in this build". Playing the live game, the owner had to click a row of the Prismatic control
window, then Approve at the bottom of the window, then the next row, for every approval. On
2026-11-23 the owner ordered (session 4dfda610, recorded as the Majordomus decision "bulk consent",
verbatim): "pridej approve all, rozsir moznosti a kvalitu Prismatic control dialogu a jeho childs a
napojeni na vse ostatni, vyuzij maximalne potencial; vic ui/ux/dx - tlacitko u kazdeho radku,
approve/deny all, selectory; next; klikani, takhle musim klikat jak vocas tam a zpet; + pridej
podobne jako ma treba claude, proste wildcarding 'always allow ai.set_setting {json wildcard json
path matcher}', 'always deny ...'" (add approve all; a button on every row, approve/deny all,
selectors, next; standing rules like Claude's, "always allow ai.set_setting {matcher}", "always
deny ..."). The owner then added: "selectory (yes, no - at to muzu rychle odklikat), checkboxy pro
multiselect, etc" (selectors, yes/no so I can click through quickly, check boxes for multiselect).

## Decision

Bulk consent and standing rules join the one approval core of ADR 0038. They are the owner's acts,
like a single approval, and every one of them is recorded.

1. **Approve all and deny all over a filter.** `prismatic_mcp approve all [<tool-glob>
   [<json-matcher>]]` and `prismatic_mcp deny all ...` settle every pending approval the filter
   selects. Approving skips an expired approval and, for the panel, an owner-only one. Each id still
   goes through `ApproveById` or `DenyById` on its own and records its channel with the filter
   (`approved_via`, or `settled_by` of a denial). The amendment replaces ADR 0038's "No approve all
   stays".
2. **Standing rules.** A rule is `{id, effect: allow|deny, tool: glob, match: JSON matcher or none,
   created_via, created_at, hits, persisted}`.
   - The tool glob (`*` any run, `?` one character) matches the MCP tool name
     (`openttd.ai.set_setting`) or the action type (`ai.set_setting`, `command.BuildRail`).
   - The matcher is a JSON object over the approval's canonical arguments. A plain key is a member
     the arguments must have, whose value matches recursively (a subset match). A key starting
     with `$` is a path (`$.a.b`, `$['a b']`, `$.list[0]`, `$.list[*]`, `$.*`, `$..name`) that must
     select at least one value, and every value it selects must match. A string pattern is a glob
     against a string value or the JSON text of any other value, `"*"` matches any value present;
     a number, boolean or null is JSON equality; an array matches element by element.
   - The glob escapes `\*`, `\?` and `\\`, and `?` is one UTF-8 character. A matcher may also
     bound a value with `$lt`, `$lte`, `$gt`, `$gte`, `$in` and `$ne`. A number pattern matches a
     string holding its JSON text. A path that selects nothing (`[*]` of an empty list) fails its
     key, for an allow rule and a deny rule alike.
   - A deny rule wins over every allow rule. An allow rule made through the panel
     (`created_via panel`), the panel's auto-approve loop (`auto`) or loaded from the rules file
     (`file`) never decides an owner-only tool (`ai.set_settings`, a Battlebrain plan); one made at
     the game's console or by the owner's own click in the window does.
   - An allow rule over a glob of several tools (`*`, `openttd.cmd.*`) whose matcher constrains no
     value is refused, except at the console with `--force`. A matcher constrains no value when
     it is none or `{}`, or every entry, under any key or path, is a non-empty run of `*`, a lone
     `$ne`, or an object of the same kind (`{"company": "*"}`, `{"$.*": "*"}`); a literal, the
     empty string, any other glob, `$in`, a bound or an array narrows. The panel's
     `broad_matcher` is the same definition, and the window refuses with the same words.
   - Removing a deny rule is guarded like an allow rule, since an allow rule may then decide what
     it held back.
   - Rules apply to every approval as it is created, and to every approval already waiting when a
     rule is added. Each decision a rule makes is recorded like a manual one: the decision's
     `approved_via`, or the denied approval's `settled_by`, names the rule
     (`rule #3 allow openttd.ai.set_setting {"name":"max_*"}`), so `openttd://decisions`,
     `DecisionTrace` (`via=`) and the ledger show it; the rule counts its hits.
   - Rules decide a new request before the queue's cap is measured, so a full queue never refuses
     what a rule decides at once.
   - Default: no rules. A rule lives as long as the game: a new or loaded world ends every rule
     that is not persisted, since it may name a company or a tile of the old one. With `--persist`
     it is also kept in `<personal dir>/prismatic_mcp_approval_rules.<server port>.json`, written
     atomically with mode 0600. That file is loaded when the MCP server starts, only when it belongs
     to the user and nobody else may read it, and every loaded rule is printed to the console and
     the log, so a standing rule is never silent.
3. **Channels.**
   - The game's console: `prismatic_mcp rule allow|deny <tool-glob> [<json-matcher>] [--persist]`,
     `prismatic_mcp rules`, `prismatic_mcp rule remove <id>`, `approve all`, `deny all`. A matcher is
     the tokens after the glob joined by spaces, so `{'company': 1, 'name': 'max_*'}` with single
     quotes and a quoted `"{\"name\":\"max_*\"}"` both work.
   - The control window: one-click Approve, Deny and Always... on every row at the same x, check
     boxes with Shift ranges and Space, Approve selected/all (confirmed with the count), Deny
     selected/all, a tool selector and Hide expired, the next waiting row selected after a decision,
     the keyboard (Up/Down or J/K, A or Y, D or N), the details of the selected approval, a Rules
     section with Remove (confirmed), and an allow rule confirmed with how many waiting approvals
     it decides now. It issues the console lines above with `--via window`. Guards against a
     decision the owner did not aim at:
     - its keys decide only after a click in the window, and Enter (the chat key) decides nothing;
     - a held key decides once: the same key seen again within 600 ms of its last sighting is a
       repeat, and each repeat restarts that time (`Panel::KeyTaken`);
     - for 400 ms after rows moved (by themselves, or after the owner's own decision), a row's
       buttons and Approve/Deny all decide nothing, and the second click of a double click never
       does; for 400 ms after the selection changed by itself, the keys decide nothing;
     - on a network client a decided row is marked "sent, waiting for the server" until the server
       stops listing it, and the client reads every page of the server's queue;
     - a dropdown's choice arrives frames after the click, outside any injection scope, so a menu an
       injected click opened never makes an allow rule.
   - The toolbar's "Open approvals in the browser" opens the panel's approvals
     (`$PRISMATIC_PANEL_URL`, else `http://127.0.0.1:8800`).
   - The panel (`mcp-panel`): its owner-authenticated routes write the same console lines on the
     dedicated server's stdin and append `--via panel` (its auto-approve loop `--via auto`), and
     `--nonce <n>`. The console then prints one machine-readable line per command,
     `[approval-core] {"nonce", "ok", "decided": [ids], "refused": [{id, error}], "rejected": [{id,
     error}], "skipped": [{id, error}], "rule_id", "error"}`, which is all the panel reads:
     refused is not run, rejected is run with its command refused.
   - Without `$PRISMATIC_HUB_URL` the hub's menu items lead to the panel's base, since a co-play
     stack starts the panel and no hub.
   - MCP stays read-only: `openttd://approvals` and `openttd.ui.control` carry `rules` and, per
     approval, `matched_rule {id, effect, decides}`. No MCP tool makes a rule or approves in bulk.
   - On a network client, `approve all` and `deny all` expand over the server's queue as the client
     last heard it and send one `!approve <id>` per id. Rules live on the server only, and a
     client's console refuses `rule` and `rules`.

## Attended and unattended decisions (with #264)

#264 holds the deadline's approve, an unattended default, to one company's own action for a
company a registered approver played when the approval was asked and plays now. The same
condition (`approver_at_creation` and `ApproverPlays`) holds every decision made without the
owner present:
- the panel's auto-approve loop, `--via auto`: per id, `approve all`, and any rule made via auto;
- an allow rule loaded from the rules file (`created_via file`). Any process running as this user
  can write that file, so a loaded rule is not the owner's present act. The load says so for each
  such rule.

A held decision is reported as "held: no registered approver plays company N".

A rule made at the console, in the window or through the owner-authenticated panel route, and an
approve all from those, is the owner's explicit, recorded act, the "always allow" asked for. It is
not held, so it works in a local game without approver seats. Deny rules are never held:
denying enacts nothing. Owner-only tools are never decided by a rule, and every rule decision
records the rule and the channel it was made through.

## The trust boundary of the console line

The core cannot tell who typed a console line. A line on the dedicated server's stdin without
`--via` is the console's: the dedicated server's operator, with the console's full authority. The
panel's owner routes are the only writers that append `--via panel`. The panel's `/api/console`
refuses any line holding a `rule` or `rules` token or a `--via` token, in every spelling the game
accepts (`console_word`), as it already refuses `approve`. So a Rhai script or a local agent can
neither make a rule nor pose as the console. `--via window` is written only by the control window,
which runs inside the game process; a console that typed it would only claim the console's own
authority.

## Policy lines kept from ADR 0038

- **Nothing approves its own request.** A rule or a bulk approval is only ever the owner's act:
  the console, the owner's own click, or the panel's owner secret. Never injected input, a script,
  an MCP client or `/api/console`.
  - Approve, Approve selected/all and an allow rule from the window refuse injected input
    (`Panel::ConsentRefusal`, `UI::InjectingInput`).
  - An armed or injected console refuses `approve`, `approve all` and `rule allow`, as it refuses
    a standing `approve` deadline. Deny, deny all and deny rules stay open: they enact nothing.
- **Auto-decision is an explicit, recorded channel, off by default.** No rule exists until the
  owner makes one. Every rule is listed (`prismatic_mcp rules`, the window's Rules section,
  openttd.ui.control), and every decision it makes names it.
- **An owner-only tool is decided only by the owner in the game.** That means the console or the
  owner's click: a panel-made allow rule, a panel approve all and a panel per-id approval
  (`--via panel`) skip it.

## Alternatives rejected

- **Rules in the panel (Rust) instead of the core.** That would be a second decision path, the
  parallel implementation ADR 0038 removed. The window, the console and the panel would disagree.
- **A privileged MCP tool to make rules.** An MCP client is exactly the requester that must never
  approve its own request; the panel already reaches the core through the console line.
- **Approve all without a filter only.** The owner asked for selectors; the filter is the same
  matcher the rules use, so there is one matching language.

## Consequences

- `prismatic_mcp.h` no longer says that there is no bulk approval; it names the channels that may
  approve in bulk.
- A rule's matcher binds to the canonical arguments, the same text an approval binds to. A rule for
  `ai.set_settings` should not name `expires_tick`, which the window's "Always ... matching" leaves
  out of the matcher it offers.
- A standing allow rule hands consent for what it matches to the rule, until it is removed. The
  knowledge `auto-approve-is-the-owners-switch` applies to it unchanged: an agent never makes one.
- The panel must:
  - append `--via panel` on its owner routes;
  - refuse `rule`, `rules` and `--via` at `/api/console`;
  - show `rules` and `matched_rule`.

  The panel's own change does this; it is not part of this one.
