# Design note: the owner's companies settle their approvals by a countdown

**RESEARCH-ONLY. Status: reviewed by a peer session, revised; not implemented.** It went to review
before any code, under the owner's order of 2026-11-23 to consult other models and peers first.
Peer session openttd-36 reviewed it (below). Codex and Gemini have not: sending repository content
to external models was refused to this session, and the owner decides whether they review it.

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
