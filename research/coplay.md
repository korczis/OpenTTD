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
panel and scripts can still press the client's widgets.

The same change fixes the client supervisor in `mcp-e2e`. Under `set -e`, a crashed client
(`wait` returning 134) ended the restart loop, so the client was never restarted (seen
2026-11-22, an abort in libiconv at Cocoa start-up).

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

## Limits

- Loopback R&D games that the owner runs only. It changes nothing about stock servers, and a
  stock client still gets the upstream refusal from a stock server.
- The owner and the bot share one bank account and can work against each other: the bot does not
  know which tiles the owner is building on.
- The company's AI debug window and `ai.*` settings still treat it as an AI company.

## Related

[`net-compat.md`](net-compat.md) (the other research network policy), [`mcp-human.md`](mcp-human.md),
[`live-arena.md`](live-arena.md), [`mechanisms/prismatic-ai.md`](mechanisms/prismatic-ai.md).
