# Design note: the owner's companies settle their approvals by a countdown

**RESEARCH-ONLY. Status: revision 2, not implemented.** It went to review before any code,
under the owner's order of 2026-11-23 to consult other models and peers first. Peer session
openttd-36 reviewed it. A three-pass analysis (analysis, adversarial check, synthesis) then corrected
it, and "Revision 2" below is authoritative where it differs from the sections above it. Codex and
Gemini have not reviewed it: sending repository content to external models was refused to this
session, and the owner decides whether they should.

## The owner's decision

Recorded verbatim in
[`auto-approve-is-the-owners-switch`](../.ai/repo/knowledge/curated/auto-approve-is-the-owners-switch.md):
the owner approves only for their own company and their co-play company. Those approvals settle by a
countdown with a default, consistently with "your bot asks"
([`the-owner-can-auto-answer-the-bots-questions`](../.ai/repo/knowledge/curated/the-owner-can-auto-answer-the-bots-questions.md)).
The panel's blanket auto-approve stays off.

Why it matters: in the live game of seed 15805, 199 re-plans for company 0 stayed pending from 1952
to 1962. The bot played the plan of 1950 to the end, and company 0 finished at 0.88M against
AAAHogEx's 29.4M ([incident](incident-2026-11-23-trainless-lines.md)).

## What exists (master 311efd3f0b)

- **A deadline for every pending approval.** `SettleDueApprovals` (`src/prismatic_mcp/prismatic_mcp_server.cpp`)
  uses `detail::DeadlinePolicy {ticks, verdict}` from `prismatic_mcp_detail.h`. The default is
  30 game days with `keep`. `keep` retires the approval and enacts nothing. `approve` executes it,
  but only when `detail::ChatScoped` holds: an AI setting change of any company, or a company-level
  command of any company. The verdict comes from `prismatic_mcp approval_deadline` at the server's
  own console or from `PRISMATIC_MCP_APPROVAL_DEADLINE` at start (`mcp-e2e --approval-deadline`).
- **A chat approval scoped to the approver's company.** `ChatApproval` needs a registered approver
  key and requires `CompanyOf(approval) == the client's company` (`f.same_company`).
- **The in-game control window** (`openttd.ui.control`, #231) shows the co-play company's pending
  changes to a client in it. It does not show a deadline.
- **"Your bot asks"** (`WindowClass::PrismaticAsk`): a bar that empties in real time, a default
  answer when it runs out, an earlier answer by click, and a click on the bar to stop the countdown.

## The gaps

1. **Scope.** `approve` at the deadline can execute a change for any company, not only the owner's.
2. **Visibility.** No window shows how long a pending change has left or what will happen then.
3. **Default.** The default verdict is `keep`, so a co-play game that nobody configures retires
   every re-plan unenacted. The panel then plans again, and the bot never gets a plan.

## Proposal

1. **Scope by derivation, not by a new list.** At its deadline, an approval is executed only when
   both hold. First, `ChatScoped` holds, as today. Second, the company it acts on (`CompanyOf`) is
   one where a client with a registered approver key currently plays. That client is the owner's
   own company or the co-play company the owner joined, the same test `ChatApproval` applies with
   `same_company`. An approval for any other company, or with no company argument, is retired as
   under `keep`. This needs no new configuration. Who the owner's companies are follows from who
   holds an approver key and where they play now.
2. **The default verdict becomes `approve`, under that scope.** With no registered approver in a
   company, nothing qualifies, so a server with no owner present behaves as `keep` today. Consent
   stays tied to an act of the owner, registering their key, and is never a bare default of the
   code. `approval_deadline <days> keep` still turns it off.
3. **The countdown is visible where the owner plays.** Each pending row of the control window
   shows its remaining game days, as a bar that empties like the ask window's, with its default
   ("approves in 12 days"). Approve and Deny act at once, as today. A click on the bar holds that
   approval: its deadline is suspended until the owner answers, as a click on the ask window's bar
   stops its countdown.
4. **Countdown length.** The default stays one game month (30 days), the cadence `replan.rhai`
   plans at, so a plan settles before the next one is asked for. Real time does not apply, because
   approvals live in game time, unlike the ask window's seconds.
5. **Evidence and tests.**
   - Unit tests of the pure helpers in `src/tests/test_prismatic_mcp.cpp`: the scope rule, the
     default, and the hold.
   - `tools/research/mcp-approval-check` gains a part: a pending AI setting change for the co-play
     company with an approver in it executes at its deadline, and one for another company is retired.
   - The approval's `settled_by` names the scope ("deadline approve (approver in company 0)").

## Review (peer session openttd-36, 2026-11-23) and the revised design

The review answered the five questions below and found two blockers. The design now takes all of
it in:

1. **Scope, captured and checked.** The company the approval acts on (`CompanyOf`) is recorded when
   the approval is created. At the deadline, a registered approver must still play in that company.
   Deriving the scope only at settlement would let a key holder who joins another company later
   widen it retroactively. `settled_by` names the captured company.
2. **Default approve, announced first.** Approve as the default under that scope matches the
   owner's "countdown with a default". The approval is announced in the control window or the chat
   when it is created, so a default never fires unseen.
3. **A hold covers one approval, and shows its age.** The control window (and the narrator's
   digest) shows "held N days", or 199 pending re-plans come back as 199 held ones.
4. **Revalidate at the deadline.** A setting is compare-and-set: a change computed against a value
   the setting no longer has is retired as stale. A command runs its test pass again before it
   executes.
5. **Game ticks on the server.** The deadline counts game ticks on the server. Execution goes
   through the regular command and network path. A hold clicked on a client reaches the server
   (chat or admin message), because a client-local hold does nothing. A paused game stops the
   countdown, and the docs say so.

- **Blocker 1, supersession.** A newer pending approval for the same company and the same setting
  key retires the older one ("superseded by #id"). Without that, a default of approve replays the
  backlog of stale re-plans one after another.
- **Blocker 2, a re-plan settles as one batch.** One re-plan files several per-key approvals
  (`fr_from`, `fr_to`, `fr_cargo`, ...). They carry a batch id from creation and settle all or
  none, so the countdown never writes half a freight slot. This is a batch settlement over the
  existing per-key approvals. The owner-only atomic `set_settings` is not needed.
- **Tests added to the plan.** `mcp-approval-check` gains a case for supersession and one for a half
  batch that must never execute.

The reasoning record of the task (`majordomus reasoning status`) holds the assessment and the
conclusion that adopts this.

## Questions for review (as first asked)

1. Is "a registered approver currently plays in that company" the right derivation of "my player
   or my co-play player", or should the company be captured when the approval is created?
2. Is a default verdict of `approve` acceptable when it is gated on a registered approver being
   present, or should the owner opt in each game (`--approval-deadline 30:approve`)?
3. Should holding a countdown suspend only the deadline of that one approval, or every approval of
   that company?
4. Does executing at the deadline need a fresh validation (stale arguments, the world moved on), or
   is the existing `ApproveById` path's validation enough?
5. Anything in the determinism or network model (the deadline is settled on the server's main
   thread, the client only displays it) that this breaks?

## Revision 2 (after the three-pass analysis and openttd-36's store measurements)

**Correction to "Why it matters".** Pending re-plans were not the binding cause of the 15805 loss.
The bot had run out of candidates: a rival's rail station hides a producer for good
(`ForeignRailStationAt`, before the rich test), and the loan policy froze construction on 47.9% of
the days of 1950-52. The countdown makes a plan reach the bot. It does not close the gap by itself.

**Corrections to the design:**

1. **The countdown has a floor in wall time.** 30 game days pass in about 60 seconds of wall time.
   openttd-36 measured a median of 60.2 s in run 31337 and 59.9 s in run 12609, from filed to
   retired. A countdown in game days alone would fire before the owner could read it. The countdown
   is `max(N game days, M real seconds)`, shown as a bar that empties in real time, like the ask
   window's.
2. **A re-plan settles as one plan (see the open conflict below: option (a), a plan id over per-key approvals, was chosen over `ai.set_settings`).** `ai.set_settings`
   validates every key and applies them all in one main-thread call
   (`prismatic_mcp_ai.cpp:268-300`). Today the panel files one `ai.set_setting` per key, in
   alphabetical order (`panel_bot.rs:402-407`, `build_plan.rhai:66`). Under `keep` only an
   alphabetical prefix of a plan ever got through: 26 of 756 in run 12609, never the rail core.
   `panel_decision.rs:765-770` already uses the atomic tool. This reverses the earlier "batch ids,
   set_settings not needed".
3. **Supersession exists on master, and its semantics need fixing.** `detail::Supersedes`
   (`prismatic_mcp_detail.h:596-601`, `prismatic_mcp_server.cpp:1133-1142`, `855a9c83dc`) denies an
   older pending approval when any one key overlaps. So a later one-key manual change withdraws a
   whole re-plan. The rule becomes: a newer whole plan replaces an older whole plan, and a manual
   change of one key is refused while a plan holding that key is pending (and says why).
4. **An approval binds one session of one world.** `_approvals` is not saved, and `ClearApprovals`
   runs on every reset (`prismatic_mcp_server.cpp:1562-1575`, `detail.h:62-70`). The countdown, a
   hold and the captured company end with a load, and the re-planner files again. This is by
   design and is documented, not changed.
5. **A plan must never turn the autopilot floors off.** Any `fr_*` slot sets `planned` and drops
   the floors (`main.nut:393-402`). The plan pending at 03:29 on 2026-11-23 would have cut
   `own_lines` from 18 to 4 and `line_budget` from 50000 to 30000. Until the bot keeps its floors
   under a plan (branch `fix/prismatic-planned-and-early-rail`), a plan that lowers a growth cap below
   its floor never settles by default; it waits for the owner.
6. **The scope governs every settle path.** The panel's blanket auto-approve executed `ai.start`
   #721 ("auto-approve, no person", 1954-05-19), which founded a PrismaticAI company nobody ordered.
   The captured-company scope applies to the deadline, the panel's auto-approve and Rhai alike.
   `ai.start` and `ai.stop` never settle by default.
7. **Compare-and-set at settlement.** `ApproveById` checks only `StillFresh`, and `set_setting` has
   no compare-and-set. A plan computed against values that have since changed is retired as stale.
8. **The owner secret is a precondition.** Until #242, the panel refused `/api/approve` and
   `/api/auto-approve` ("no owner secret"), and chat was the only working channel.

**An open conflict in correction 2, to be settled by review before code.** The panel treats
`openttd.ai.set_settings` as owner-only (`OWNER_ONLY_TOOLS`, `tools/research/mcp-rhai/src/panel.rs:566:OWNER_ONLY_TOOLS`).
`POST /api/approve` refuses it with 403, so it is approved only at the server's console, and it is
the channel of a Battlebrain live decision (rule `project.decision-authority-boundaries`, B2: who may
name an `fr_*` slot). Filing re-plans through it would take them out of the chat and panel channels
and put the re-planner on the authority's channel. The options for review:

- (a) keep per-key `ai.set_setting` approvals, with a plan id assigned at filing that settles them all
  or none (openttd-36's first proposal);
- (b) a new non-owner-only batched tool for a company's own AI settings, which excludes `fr_*` slots
  unless the writer holds the company's planner key;
- (c) make `ai.set_settings` approvable by chat for the company's own AI, which moves the B2 boundary.
  The rule and ADR 0037 would then change in the same commit.

Option (a) moves no boundary.

**Review of the conflict (openttd-36, 2026-11-23): option (a).** Option (c) moves the
decision-authority boundary, which is the owner's call together with a change to ADR 0037. It is
not a means to an end. Option (b) adds a second channel for the same thing. The details of (a):

- The panel assigns the plan id when it files the plan, and the server keeps it in each approval.
  `SettleDueApprovals` then settles the plan atomically, revalidation included.
- An approve or deny by chat or panel on any row of a plan acts on the whole plan, and the
  interface says so ("approves plan #N, 23 keys").
- Supersession works per plan, not per key.
- A test proves that half a plan is never written, even when one key fails revalidation: the whole
  plan is retired, with its reason.

**The order before a default of approve may be switched on:**

1. The bot keeps its floors under a plan (`fix/prismatic-planned-and-early-rail`).
2. The re-planner files a plan as per-key approvals under one plan id, settled all or none (option (a) below).
3. Compare-and-set and plan-level supersession are implemented and tested in
   `mcp-approval-check`: a half plan never settles, a load drops pending approvals, and a one-key
   change cannot kill a plan.
4. The captured-company scope applies on every settle path, the panel's auto-approve included.
5. The countdown is visible in the control window, with its wall-time floor and a server-side hold.
6. Held-start pairs show that the autopilot with default-approved re-plans does not lose to the
   autopilot alone (rule `project.a-tuning-claim-needs-held-start-pairs`).
7. The owner switches it on.

**Questions only the owner can answer:**

- Should a default of approve apply to re-plans in every game, or be opted into per game?
- May a plan that lowers a growth cap ever settle by default?
