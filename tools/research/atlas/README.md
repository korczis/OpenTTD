# Research Atlas

**RESEARCH-ONLY.** One projection of what the repository records about its experiments and its
architecture, drawn three ways: the [Research Atlas](https://korczis.github.io/OpenTTD/atlas.html)
page of the site, the "Evidence before claims" region of the [README](../../../README.md#evidence-before-claims),
and the README's hero illustration. Nothing in it is typed by hand: every number comes from
`research/results/*.json`, every graph from a Mermaid diagram the README already carries, every
colour from [NoirGFX's palette](../noirgfx/style.toml).

```bash
tools/research/atlas/atlas data             # the research-atlas/v1 contract, on stdout
tools/research/atlas/atlas render           # README atlas region + docs/atlas/hero-{light,dark}.svg
tools/research/atlas/atlas render --check   # write nothing; exit 11 while any of them is stale
```

`atlas` is a small Rust program (`serde_json` only); the wrapper builds it into its own `target/` on
first use. Exit codes follow Majordomus: 0 ok, 2 usage, 10 findings, 11 drift.

## Where it is wired

| Surface | How it reads the projection | Gate |
|---|---|---|
| Site page `atlas.html` | `surfaces.toml` `[[pages.items]] atlas`: `surfaces site` runs `atlas data`, publishes it as `data/atlas.json`, renders its cards as HTML and loads [`atlas.js`](atlas.js) | [`tools/surfaces/pages-check`](../../surfaces/pages-check) drives the page in Chrome and runs [`atlas.e2e.mjs`](atlas.e2e.mjs) |
| README region `atlas:begin`/`atlas:end` | `surfaces.toml` generator `atlas`: `surfaces sync` runs `atlas render` | `surfaces check` runs `atlas render --check` (commit hook, `tools/gate.sh change`, CI) |
| `docs/atlas/hero-*.svg` | the same generator | the same check |

## What it projects

**Comparisons.** Every `duel-*.json` (one game, company A from the planner's portfolio, company B the
same AI without it) and every `ab-*.json` (two games side by side from one seed). Each row keeps the
record's own numbers (both companies' final value, money, loan and fleet, the monthly value series,
seed, length, label, rival, settings, the revision it names) and adds three derived fields, by these
rules and nothing else:

| Class | Rule |
|---|---|
| `valid` | a paired comparison: both companies in one game, from a held start (`held_start` recorded), every month recorded, not reconstructed |
| `limited` | comparable with a stated caveat: the start was not held, the two arms ran as two games (every A/B record), or the record was rebuilt after a name collision (`rebuilt`, `recovered`) |
| `invalid` | not a comparison: fewer months recorded than asked, missing months, or a fallback |

- `caveats` and `problems` say why a row is `limited` or `invalid`, in words the page shows.
- `confounders` change what a row means without making it invalid: *rival ran no train* (B's
  `trains` is 0), *rival's fleet not recorded* (no fleet split), *rival AI not recorded*.
- `relative` is (A − B) / B only when B is above 1; OpenTTD reports a company with nothing left at
  value 1, so a ratio against it is absent rather than a five-digit percentage, and the row carries
  the confounder *a company ended with no assets (value 1)*.
- `revision_known` says whether the revision a record names is a commit of this repository, so the
  page links only revisions that resolve.

The classes are about what a record can support, never about who won. Different lengths, seeds and
bot revisions are never pooled into one number: a tally such as "21 of 29 paired records where the
planned bot ended ahead" is shown as a tally across separate experiments, with the
[duel evidence audit](../../../research/duel-evidence-audit.md) and the rule
[`project.a-tuning-claim-needs-held-start-pairs`](../../../.ai/repo/rules/project/a-tuning-claim-needs-held-start-pairs.v1.md)
as its validation.

**LAN battle.** Every `battle-*.json` of schema `ai-battle-record/v1`: its headline (scored matches,
field, winner and the winner's points per match, PrismaticAI's best entry and its average rank,
whether PrismaticAI ended last, replays and how many were identical), and the newest record in full
(leaderboard, hosts, match configuration, determinism). The leaderboard is the record's own,
written by `battle.py report`; nothing is re-scored here.

**Graphs.** Every ```` ```mermaid ```` flowchart in a tracked Markdown document that carries the comment
`%% atlas: <id> | <title>`. The diagram stays the one source: GitHub draws it, the site draws it,
and the atlas reads the same nodes and edges ([`src/flowchart.rs`](src/flowchart.rs)):

| Mermaid | Meaning in the atlas |
|---|---|
| `-->` (solid) | `control`: a command, a request, a call |
| `-.->` (dotted) | `data`: observed data, a read, an event, a publication |
| `==>` (thick) | `governance`: approval, validation, evidence |
| `click <node> "<path>" "<tooltip>"` | the node's page; required for every node, and it must be a tracked file |
| `class <nodes> proposed` | designed but not built; shown hollow and labelled `proposed` |
| `subgraph` | the node's group |

A statement the reader does not understand, a node without a link and a link to nothing are
findings (exit 10), so `surfaces check` fails on them. Two diagrams are marked today:
`architecture` ("How it fits together") and `approval` ("Plug in an LLM", stepped through on the
page as "From intent to evidence").

**Cards.** The evidence cards the README table and the page's cards both show. Each has a value, what
it counts, how to read it, the atlas view that explores it, and a chain: dataset → experiment →
source → validation, each a tracked path.

## The page

[`atlas.js`](atlas.js) draws five views from `data/atlas.json` through the site's client layer
(`window.surfaces` in `tools/surfaces/assets/app.js`: URL state, the chart registry, maximize and
Escape, the theme, ECharts from the site's own copy):

- **Duel explorer**: filters for kind, comparability, seed, length, rival, start, confounder and a
  label search; a scatter of rival against planned value with the equal-value line, log or linear;
  a table bound to it both ways; and a detail panel per record with both values, the difference,
  the relative difference when it means something, the class with its reasons, the fleet, the month
  by month chart and the chain write-up → JSON → tool → rule.
- **Bot evolution**: records that share a kind, seed and length, oldest first; every step down is
  marked, never smoothed.
- **LAN battle**: the newest leaderboard and every record's headline over time.
- **Architecture**: the `architecture` diagram as an ECharts graph; select a part (click, the part
  list, or `?node=`), filter edge kinds, follow what feeds it and what it feeds, open its page.
- **From intent to evidence**: the `approval` diagram walked in edge order with play, pause, replay
  and a step slider; under `prefers-reduced-motion` play jumps to the last step.

Every control writes the URL (`view`, `kind`, `class`, `seed`, `months`, `opponent`, `start`,
`confounder`, `q`, `sel`, `scale`, `group`, `graph`, `node`, `kinds`, `step`, `max`), so a configured
view is a link. The page is read-only: nothing on it reaches a game, a panel or an MCP server, and
it is built from versioned records, never from a running process.

`node --test tools/research/atlas/test/*.test.mjs` tests the page's pure functions; `cargo test` in this
directory tests the projection.

## Limits

- **Majordomus traceability is not drawn here.** Intent → milestone → issue → rule → ADR → test →
  evidence lives in the site's [AI layer page](https://korczis.github.io/OpenTTD/ai.html), which is
  derived from the same `.ai/` records; the atlas links to it rather than drawing a second copy.
- **No claimed / implemented / verified / exposed / exercised ledger yet.** The one measured gap
  (every tool exposed, about 2% exercised) is a knowledge record
  ([`mcp-exercise-coverage-is-two-percent`](../../../.ai/repo/knowledge/curated/mcp-exercise-coverage-is-two-percent.md)),
  not a dataset the projection can read.
- **No terrain or planner replay.** The duel records hold the planner's portfolio as bot settings and
  log lines, not the map, the candidate routes or their costs, so the atlas does not draw them; doing
  so needs a recorded world snapshot.
- **Ablation, post-mortem and Battlebrain records are not projected yet**; their pages are linked
  from [`research/ai-battle/README.md`](../../../research/ai-battle/README.md).
- **`readme-scoreboard` still derives its own duel tally** (the scoreboard region). Both read the
  same records and agree; folding the scoreboard into this projection is the next step that leaves
  one implementation.
