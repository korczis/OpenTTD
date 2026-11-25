# Co-play: the owner and the bot in one company

RESEARCH-ONLY, UNSTABLE. A server-side join policy behind `OPTION_RESEARCH_INSTRUMENTATION`.
Default off.

## The problem

The owner asked on 2026-11-22 for a game where their company is "controlled jointly by me and
PrismaticAI". Upstream refuses every client an AI company. Three checks do this: the server's join
(`ReceiveClientJoin`), its move (`ReceiveClientMove`) and the console's `move`
(`src/network/network_server.cpp`, `src/console_cmds.cpp`). Until now the owner's company was
therefore a second company. `mcp-human` played it through the interface ([`mcp-human.md`](mcp-human.md)).

In the game of 2026-11-22 that company had three buses and no route that earned. The audit of
that game also found 2 of its 5 road depots with no exit. Meanwhile every PrismaticAI company had
only depots that connect, and drive-through stops first. The owner's company was played by a
second, weaker implementation of the strategy. The bot's own (`PrismaticAI`, with the planner and
AlgoGlyph) was not playing it.

## The change

The server operator opens one AI company for co-play:

```
research_coplay allow <company-id>     # 1-based, as 'companies' lists them
research_coplay deny <company-id>
research_coplay clear
```

A client may then join that company (`-n <server>#<company-id>`, `connect`) or be moved into it
(`move <client-id> <company-id>`). The AI keeps running on the server. The client's commands go
through the command system as any client's, and the server validates them as it would a person's.
The policy decides only who joins and never touches game state, so lockstep is unchanged. An opened
company skips the company allow list, because the operator opening it is the authorisation.
The command is server-only (`ConHookServerOnly`). The mask holds company ids: a company that
is removed and replaced in the same slot stays open until `deny` or `clear`.

`tools/research/mcp-e2e --coplay` does all of it. It reads the running bot's company from the game
(`/api/bots`), opens it through the panel's console, refuses to go on when the server does not
confirm it, and has the GUI client join that company under the name `owner`. With `--arm-input` the
panel and scripts can still press the client's widgets. That client then is not an approver (see
"Who approves").

### The bot's settings

`PRISMATIC_BOT_SETTINGS="name=value,name=value,..."` in the launcher's environment holds the settings
the bot starts with, as `start_ai` arguments. A later name wins, so overrides go at the end. It applies to a planned bot, on top
of what the planner's portfolio chose, and to an unplanned one (`--no-plan-bot`), on its own
(`panel_bot.rs` `unplanned_settings`). Before 2026-11-23 an unplanned bot ignored it and started on
the defaults, `max_routes 4` among them. A malformed list is refused whole, and the panel's `ai`
events say so. A loaded game keeps the settings it saved.

### Who approves

The owner decided on 2026-11-22 (recorded with `majordomus decision`). The owner's own client runs
unarmed and is registered on the server as an approver. An agent that co-plays does it through a
client of its own, and that client is armed and never an approver. Identities decide, not input
paths: no check inside the process tells a person's click from synthetic input at the OS level, but
the server's approver list (`prismatic_mcp approvers`, ADR 0038) holds keys, and only the owner's
key is on it.

`mcp-e2e --coplay` registers the client named `owner` once it has joined
(`prismatic_mcp approvers add <client id>` through the panel's console). The key is stored, so a
restart of the client keeps it. With `--arm-input` it registers nothing and says so, and the owner
approves in the panel (`/?owner=<secret>`). An armed client may still look at the queue and deny,
but it never approves: both the client and the server refuse it.

The same change fixes the client supervisor in `mcp-e2e`. Under `set -e`, a crashed client
(`wait` returning 134) ended the restart loop, so the client was never restarted (seen
2026-11-22, an abort in libiconv at Cocoa start-up).

## Services

`mcp-e2e --coplay` starts the panel services that a co-play game runs, so nobody has to post them by
hand:

- `arena` fills the other seven companies ([live-arena.md](live-arena.md)).
- `replan` re-plans the bot every game month ([Planner and AlgoGlyph](mechanisms/planner-and-algoglyph.md#replanning-while-the-game-runs)).
- `watchdog` says when a company has built nothing for longer than its data-tuned threshold, and
  what holds it ([The progress watchdog](mechanisms/rhai-scripting.md#the-progress-watchdog)).

`--lineup "AAAHogEx,LuDiAI AfterFix,CivilAI,AdmiralAI,NoCAB,DictatorAI,SimpleAI"` names the other
companies of a new world instead of the arena's lineup. Each is started with the server's `start_ai`
once the bot's company exists, so the bot keeps company 1. With `--lineup`, and with `--load` (whose AIs
come from the save), the default services are `replan watchdog build_audit rail_monitor
capital_watch gap_watch`, without the arena; a new world with the arena runs `arena replan watchdog
build_audit rail_monitor capital_watch` (the monitors change nothing in the game, and the leader-gap
watch joins it once it tells and proposes only for the owner's company, #319). `MCP_SERVICES` names another list, and an empty value starts none. A service that does not start is
reported on stderr, and the stack runs on without it.

## Validation

Run on 2026-11-22 on this branch (macOS, `build-gate-research`: RelWithDebInfo,
`OPTION_RESEARCH_INSTRUMENTATION=ON`, `OPTION_PRISMATIC_MCP_SERVER=ON`), seed 20261122:

| Check | Status |
|---|---|
| `tools/gate.sh research` | PASS: ctest 128/128, `[prismatic_mcp]` 796 assertions in 27 test cases |
| `mcp-e2e --arm-input --coplay`: the server answers `research_coplay open=[1]`, and `clients` lists `owner` in company 1, the PrismaticAI company | PASS |
| The AI keeps playing that company while the owner is in it (`/api/bots`: company 1 PrismaticAI, running) | PASS |
| A command from the owner's client executes for that company. Repay £10,000 was pressed through the client's own finances window (`openttd.ui.press`). The bot repaid 110,000 in the same seconds (its log), and the server's loan went from 300,000 to 180,000, which is both | PASS |
| The arena fills the other seven companies with the owner co-playing | PASS: 8 companies |
| A client restart after a crash (the supervisor fix) | NOT RUN: no crash happened in this run |

A row without a result is not a claim (rule `project.never-report-unrun-validation-as-pass`).

## Deploying into a running game

A new panel or game binary needs the stack restarted, and restarting starts a new world. To keep
the owner's game:

1. Save it from the server's console. Use the panel's `POST /api/console {"command": "save <name>"}`
   or type it in the developer console. The file lands in the panel's temporary personal directory
   (`<tmp>/save/<name>.sav`), and that directory is removed when the panel stops. Copy the file to
   `~/Documents/OpenTTD/save/` first.
2. Stop the stack (Ctrl-C on `mcp-e2e`, or SIGINT to the panel).
3. Start it from the savegame: `tools/research/mcp-e2e --coplay --load <file>`. The panel
   passes `-g <file>` to the server instead of `-G <seed>`. It starts no new bot, because the saved AI
   companies play on, each restarted by the game from its saved data. `--coplay` opens the company
   the game reports for the bot and joins the client to it, as on a new world. The arena counts the
   saved companies and starts only what its lineup still lacks.

The co-play opening (`research_coplay`) is server state, not game state, so it is not in the save,
and `--coplay` opens it again.

Verified on 2026-11-22 against the seed 20261122 co-play game, saved in November 1960. The stack
started from the save on side ports (panel 8810, game 3990, `--no-gui`), with the binary of
the news-refs branch. All 8 companies came back with their names, money and loans, and
`/api/bots` listed all 8 AIs running: PrismaticAI four times, AAAHogEx, LuDiAI AfterFix, CivilAI
and AdmiralAI. No new bot was started. The client half (`--coplay` after `--load`): NOT RUN on
that check.

### Signing in as the owner

Approve and auto-approve in the panel need the owner's cookie, which one visit to `/?owner=<secret>`
sets. `mcp-e2e` opens that link in the owner's default browser when it starts (not with
`--no-browser`). It reads the secret from the owner's own file
(`$PRISMATIC_PANEL_OWNER_SECRET_FILE`, default `~/.config/prismatic/panel-owner-secret`, mode 0600)
and never prints it. Without the file it opens the plain panel.

### The standing verdict

The owner decided on 2026-11-22 (recorded with `majordomus decision`) that an approval nobody answers
in the co-play game is carried out after 30 game days, when it is one a chat approver could decide:
the company's own AI settings, which replan proposes every month. With the default `keep`, replan's
plans were retired unanswered and never took effect. Start the stack with
`mcp-e2e --coplay --approval-deadline 30:approve`. The owner can still approve or deny earlier in the
control window or in the panel. Anything wider than the company's own AI is retired as under `keep`
([approval-gate.md](mechanisms/approval-gate.md) "The deadline").

### After a load

A `load` on the server restarts its game, and every client reconnects in the same process. Upstream
it rejoins as a spectator (`NetworkClientConnectGame(last_joined, COMPANY_SPECTATOR)` in
`src/openttd.cpp`). In the game of 2026-11-22 the owner therefore landed among the spectators after
each deploy and had to be moved back by hand (`move 3 1`). In a research build a client that played
a company now asks for that company again (`ResearchRejoinCompany`,
`src/network/core/network_game_info.cpp`). The co-play opening is server state that a load does not
clear, so the server admits the owner into the co-play company as it did on the first join. A client
that was a spectator rejoins as one.

## Limits

- Loopback R&D games that the owner runs only. It changes nothing about stock servers, and a
  stock client still gets the upstream refusal from a stock server.
- The owner and the bot share one bank account and can work against each other: the bot does not
  know which tiles the owner is building on.
- The company's AI debug window and `ai.*` settings still treat it as an AI company.

## Related

[`net-compat.md`](net-compat.md) (the other research network policy), [`mcp-human.md`](mcp-human.md),
[`live-arena.md`](live-arena.md), [`mechanisms/prismatic-ai.md`](mechanisms/prismatic-ai.md).
