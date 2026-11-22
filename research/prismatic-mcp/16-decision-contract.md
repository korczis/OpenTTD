# 16 — Decision Contract & Approval Workflow

Implemented in `src/prismatic_mcp/prismatic_mcp_server.cpp`. Verified live (Vertical B), and on every
run of `tools/research/mcp-live-check`.

## Flow

```
client: tools/call openttd.decision.validate {action:{type,arguments}}   -> non-mutating validation
client: tools/call openttd.decision.submit   {action:{type,arguments}}   -> {status: pending_approval, approval_id, argument_digest}
operator (OpenTTD console): prismatic_mcp approve <approval_id>            -> approval bound to that exact digest
client: tools/call openttd.decision.submit   {same action}                -> {status: executed, command_result, verdict, observed}
```

## DecisionEnvelope (accepted shape)

`openttd.decision.submit` accepts `{"action": {"type": "<action>", "arguments": {...}}}` (plus optional
`decision_id`, `correlation_id`, `company_id`, `snapshot_id`, `confidence`, `rationale`, `objective`,
`expected_effects` — retained for forward-compat, not interpreted as authority). Unknown fields never
grant privilege.

## Validation (`openttd.decision.validate`, never mutates)

Returns `valid`, `errors`, `current_snapshot_id` (`tick-<n>`), `required_scope`, `approval_required`
and `mutating` (`ValidateAction()` in `prismatic_mcp_server.cpp`). Checks: known action type,
argument shape/types; for `command.<Name>` also the actor (`company`), the `location` and a
side-effect-free test run of the command (`would_succeed`, `estimated_cost`). `approval_required` is
true for every action type the gate executes. An `api.Script<Class>.<Function>` action is validated
by `Api::Validate()` instead, and an `openttd.api.*` tool call without `execute: true` runs in
`GSTestMode`, changes nothing and needs no approval (`research/mcp-api.md`, "The Script API as MCP
tools"). Snapshot-staleness checks are staged.

## Approval (two-phase, digest-bound, one-time)

- **Digest**: FNV-1a of `type + "|" + canonical(args)`, reported as `argument_digest`. The approval
  itself is matched on the exact canonical arguments, not on the 64-bit digest, which could collide
  (`detail::ClassifySubmit`, unit-tested in `src/tests/test_prismatic_mcp.cpp`).
- **Two-phase**: a mutation never executes on first submit; it returns `pending_approval` until an
  operator approves in the OpenTTD console.
- **Argument binding**: an approval for `pause:true` does NOT authorize `pause:false` (different digest)
  — verified live. Defends the "change args after approval" abuse case.
- **Executes on approval** (ADR 0038): `ApproveById` carries the action out at once, on the main
  thread, through `ExecuteApproval`, the one execution path for every channel: the server console,
  the panel's Approve (which gives that console line), and a client's chat `!approve <id>` (which
  the client console's `prismatic_mcp approve` sends). Nothing has to resubmit the call. The queue
  is `openttd://approvals`, which the panel reads instead of keeping its own copy.
- **Approving from inside the game** (ADR 0038, "Who may approve"):
  - On the server, `!approvals [page]`, `!approve <id>` and `!deny <id>` in the chat are decided
    by `PrismaticMCP::ChatApproval` through the pure `detail::DecideChat`. A network client's
    console `prismatic_mcp approvals|approve|deny` sends the same line.
  - Approving needs all of these:
    - the approval names the client's company;
    - the action acts on one company (`ai.set_setting`/`ai.set_settings`, or a command without
      the Server, Offline or Deity flag, outside the `server_setting` and `cheat` categories);
    - the client's key is a registered approver (`prismatic_mcp approvers add <client id>`),
      and only that: a key in the company's allow list is not one (ADR 0038, amended
      2026-11-22);
    - the client is not armed for MCP input (`PRISMATIC_MCP_INPUT=1`).
  - Every refusal names the condition that failed.
  - One id per line. `!approvals` answers one `[approvals] {json}` line per approval and a count
    line. Each decision records `approved_via`.
- **A full queue refuses**: at `MAX_APPROVALS`, decided and expired approvals are dropped. A new
  request that still does not fit is refused with `status: refused` and the reason. A pending
  approval is never pushed out.
- **One-time**: on execute the approval is `Consumed`; a replay of the same approved call returns a new
  `pending_approval` — verified live. Defends replay.
- **TTL**: approvals expire (`APPROVAL_TTL_TICKS`), bounded store (`MAX_APPROVALS`). An action that
  names its own stale date expires sooner: `ai.set_settings` takes an optional `expires_tick`, bound
  with the other arguments, and its approval expires at `min(now + APPROVAL_TTL_TICKS, expires_tick)`
  (`detail::ApprovalExpiry`); its validation, run again at execution, refuses it after that tick even
  when it was approved (`detail::StillFresh`).
- **One decision, one approval**: `ai.set_settings {company, settings: {name: int, ...}, expires_tick?}`
  is several AI settings under ONE approval (at most `detail::SET_SETTINGS_MAX`, 32; an empty map is
  refused). Every name and value is checked against the company's running AI through the same check
  as `ai.set_setting`; if any is unknown, out of range or fixed while the AI runs, none is applied
  and the error names each. Otherwise all are applied in the same main-thread call, so neither the
  operator (approving some and not others) nor the script (reading between two changes) can leave
  the AI on a mix nobody chose.

## Execution & honest verdict

Execution goes through the normal deterministic command path (`Command<...>::Post`), on the main
thread (handlers run inside `Poll()`), never a raw pool write (`ExecuteAction()` in
`prismatic_mcp_server.cpp`). `game.set_pause` posts `Commands::Pause`; `command.<Name>` posts that
command as the given actor; `ai.start`, `ai.stop` and `ai.reload` post `Commands::CompanyControl`, while `ai.set_setting` and
`ai.set_settings` write the AI's configuration (`AIConfig::SetSetting`), the one its script reads on
its next `GetSetting`; an `api.*` action
runs the Script API function in `GSAsyncMode`.

The submit answer carries `verdict: PENDING_OBSERVATION` while the command is queued. The verdict is
taken when the command has run, from its result callback (`Conclude()`); read it with
`openttd.decision.get {decision_id}`:

- `VERIFIED_SUCCESS` — command ok AND, where the action declares an observable effect (pause state,
  AI state), the effect observed after the command ran matches intent.
- `EXECUTED_UNVERIFIED` — command ok but the effect does not match or cannot be observed. An
  `api.*` action on a network server is posted with no callback and always reports this, with
  `command_result: posted`; read its effect back with the API's own getters.
- `VERIFIED_FAILURE` — command failed.

`isError` in the MCP `tools/call` result reflects `command_result`, never mere HTTP acceptance.

## Provenance (`openttd://decisions`, `prismatic_mcp decisions`)

Bounded trace of `{decision_id, approval_id, action_type, arg_digest, command_result, verdict,
requested_tick, observed_tick}` — no bearer token, no secrets. Distinguishes command execution from
observed outcome per the mission's provenance requirement.

## Action catalogue (this build)

The fixed types are named once, in `detail::ACTION_TYPES` (`src/prismatic_mcp/prismatic_mcp_detail.h`):
`game.set_pause` (SERVER_ADMIN), `ai.set_setting`, `ai.set_settings`, `ai.start`, `ai.stop`, `ai.reload` (AI_CONTROL).
`ActionTypes()` adds one `command.<Name>` (GAME_MUTATION) per game command whose parameters convert
from JSON, derived from the command table, and `openttd.decision.list_action_types` lists all of
them. `api.Script<Class>.<Function>` (SCRIPT_API) is the action of an `openttd.api.*` call with
`execute: true`. Company renames, vehicle start/stop and send-to-depot are no separate adapters:
they are the commands `command.RenameCompany`, `command.StartStopVehicle` and
`command.SendVehicleToDepot`, offered while `openttd://commands` marks them `exposable`.
