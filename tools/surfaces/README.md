# Published surfaces

One generator turns what a repository already states into everything it publishes. The
sources are its tracked Markdown, its git history and its `.ai/` layer. The outputs are:

| Surface | Command | What it is | Where it goes |
|---|---|---|---|
| Site | `surfaces site` | Every Markdown document as a page, plus an overview with interactive charts, the `.ai/` layer (features, decisions, rules, plan, dependency graph) and search | GitHub Pages |
| Wiki | `surfaces wiki` | The same documents as wiki pages: `Home`, a sidebar, links rewritten to pages | The repository's GitHub wiki |
| Markdown regions | `surfaces sync` | Tables inside the documents themselves (document indexes, features, decisions, rules, plan, surfaces, facts, a table of contents, a region included from another document), between markers, after every declared generator has run | The repository |
| Gate | `surfaces check` | Fails while a region is stale, a declared generator's check fails, a relative link or anchor leads nowhere, a document's front matter does not parse, a tool's flags and its usage disagree, or a diagram is drawn in characters instead of Mermaid | Pre-commit, `tools/gate.sh`, CI |
| Diagrams | `diagrams-check` | Opens every built page with a Mermaid block in Chrome and fails on a diagram that does not render to an SVG | CI, after `site` |
| Declared pages | `pages-check` | Opens every page `[[pages.items]]` declares in Chrome, desktop and phone, light and dark, then runs the page's own scenario | CI, after `site` |

Nothing in the generator names a repository. Every fact comes from git (`origin`, `HEAD`,
the tracked files, the log), from the `.ai/manifest.yaml` sections, and from each
document's own front matter and headings. The only declared input is `surfaces.toml` at
the repository root, and only for the choices data cannot make. Today that is which
documents to publish and which to exempt from the link check.

## Why

Documentation drifts the moment it restates something. A hand-kept index misses the
newest document. A status table says "planned" a week after the work merged. A landing
page describes a repository that changed visibility, and links point at files that were
renamed. Each copy is a second source of truth, so it is wrong as soon as the first one
moves.

The fix is not more diligence. Publish nothing that a person has to keep in step:

- **Derive, never restate.** A page is rendered from its document. A chart is computed
  from the history. A table is generated from the records it summarises. The published
  copy cannot disagree with the source because it has no content of its own.
- **One source, many surfaces.** A reader on GitHub, on the site, in the wiki or in an
  editor sees the same text. The region tables put the derived summaries inside the
  documents, where an agent working in the repository reads them too.
- **Enforced, not advised.** A stale region or a dangling link fails the commit, the gate
  and CI. Drift becomes a failing check with a name and a line number, not something a
  reviewer has to notice.
- **Readers drive the views.** Every chart is a model the reader controls: a window, a
  metric or a cut-off, kept in the URL so a view can be linked. Each chart maximizes to the
  viewport and Escape restores it. A diagram is Mermaid source in the document, rendered in
  place, never a pasted image.
- **Generic.** The same files work in any repository. A repository supervised by
  Majordomus also gets its `.ai/` layer rendered. The site, wiki and region report each
  declare themselves as `web-surface/v1` under `target/web/<id>/`, where
  `majordomus_web_surfaces` discovers them.

## Use

```bash
tools/surfaces/surfaces check          # the gate: exit 10 on a finding
tools/surfaces/surfaces sync           # run every declared generator, then regenerate every region
tools/surfaces/surfaces sync --check   # write nothing; exit 11 when a region or generator would change
tools/surfaces/surfaces site           # build target/web/site (needs node for the assets)
tools/surfaces/surfaces wiki           # build target/web/wiki
tools/surfaces/surfaces list           # what each surface publishes, one document per line
```

Prerequisites: `python3` 3.11 or newer. The wrapper creates `tools/surfaces/.venv` from
`requirements.txt` on first use. `site` also needs `node`; it runs `npm ci` in this
directory the first time. Exit codes follow Majordomus: 0 ok, 2 usage, 10 findings,
11 drift.

To preview the site, serve `target/web/site` with any static server, for example
`python3 -m http.server -d target/web/site`.

The workflow uploads the site with `include-hidden-files: true`. Every page of the `.ai/` layer
lives under a dot directory, and `actions/upload-pages-artifact` leaves those out by default, so
without it the site would link pages that Pages answers with 404.

## Tool flags

A document that shows `tool --flag` restates the tool's parser, so `check` treats it like a link:
the flag has to lead somewhere. The tools are the tracked executable scripts. Shell scripts are
read through their `case` arms (and `[[ "$1" == --x ]]` tests), Python scripts through argparse
(and `"--x" in argv`). Each finding names the file and line:

| Finding | When |
|---|---|
| `usage` | the script accepts a flag that its own usage header never names |
| `help` | `--help` is missing from a script with a usage header, exits non-zero, or prints a fixed line range that stops before the header ends |
| `flag` | a document's code shows `tool --flag` and the tool's parser does not accept it |

A shell script whose catch-all `*)` arm does not `exit`, or that only tests flags with `[[ ]]`,
passes unknown flags on, so it cannot refuse one and the `flag` check skips it. The usage header
is the comment block after the shebang. `-h|--help) sed -n '2,/^[^#]/s/^# \{0,1\}//p' "$0"; exit 0 ;;`
prints all of it, however long it grows. `[flags] exclude` in `surfaces.toml` takes path globs
for scripts the check must not read.

## Markdown regions

A region is a pair of HTML comments whose contents the generator owns:

```text
<!-- surfaces:begin docs-index dir=research -->
(generated table)
<!-- surfaces:end docs-index -->
```

| Generator | Arguments | Table |
|---|---|---|
| `docs-index` | `dir=<path>` (default: all), `depth=<n>`, `fields=<a,b>`, `as=pie by=<field>` | Every published document under the path, with its derived summary and a column per front-matter field in `fields`; or a pie of the documents by one front-matter field, each slice opening that table filtered to its value |
| `ai-features` | `as=pie` | `.ai/` features: name, status, headline; or a pie by status |
| `ai-adrs` | `as=pie` | Architecture decisions: id, status, title; or a pie by status |
| `ai-rules` | `origin=project\|vendored\|all`, `as=pie` | Rules: id, class, statement; or a pie by class |
| `ai-plan` | `as=bar` | Milestones (`M1` ... in plan order) with issues done and still open (status derived from the issue stamps); or a bar chart of done against planned |
| `table-chart` | `src=<document>`, `region=<name>`, `label=<column>`, `value=<column>`, `line=<column>` | A bar chart of one column of the first table that has those columns, in a region of another document (or the whole document); `line` adds a second column as a line, `_` stands for a space, numbers may carry `k`, `M` or `G` |
| `surfaces` | — | Where the site and the wiki are published |
| `toc` | `depth=<n>` (default 3) | The headings that follow the region, levels 2 to `n`, nested and linked with their anchors |
| `include` | `src=<document>`, `region=<name>` | The body of a marked region of another document (`<!-- name:begin -->`, any tool's marker, or a surfaces region of that generator), its relative links rewritten for this document and its comment lines dropped |
| `facts` | `group=<name>`, `as=table\|line\|pie` | Numbers counted from the tracked files: each `[[facts.items]]` in `surfaces.toml` counts regex `matches` `in` a glob, tracked `files` matching a glob, or published `docs` under a path; the table shows what was counted, `line` is one sentence of bold numbers, `pie` a Mermaid pie |
| `open-findings` | `as=pie` | What the records say is missing, blocked, handed off, not run or failed, one row per finding with its kind, its evidence (record and line), its follow-up issue and an OBSERVED/DERIVED label, computed by `findings.py` from the sources `[findings]` declares; or a pie by kind, each slice opening the table filtered to that kind ([Open findings](#open-findings)) |

A fact marked `site_only = true` is left out of every Markdown region and shown only on the site's
overview, where it is computed at build time. The document count is one: nearly every pull request
adds a document, so in `README.md` it made every two open pull requests conflict on the same line.

A fact is the one kind of number a committed document can state without going stale: it changes
only when the files it counts change, and `surfaces sync` rewrites it in the same commit. A count
of commits or of today's date is not a fact in this sense, since every commit would change it; the
site's overview computes those at build time instead. The overview also shows every fact as a card
linked to its page, with what was counted as its tooltip.

An `include` reads the other document as it is on disk when `sync` starts. When that region is
itself one this `sync` regenerates, the include catches up on the next `sync`, and `check` says so
until then.

## Charts

A chart is a number drawn, so it is generated like one. Every chart in a document is a Mermaid
`pie` or `xychart-beta` block written by a region (`facts as=pie`, `ai-* as=pie|bar`,
`table-chart`) or by a tool between its own markers: the README scoreboard, every duel page
(`tools/research/mcp-duel --render`, markers `duel-charts`), every AI battle record and
ablation page (`battle.py render`, markers `battle-charts` and `ablation-charts`) and every A/B
record page (`tools/research/mcp-ab --render`, markers `ab-charts`), each drawn from its record's
JSON and declared as a generator below. The battle post-mortem pages (`battle-postmortem render`,
markers `postmortem-charts`) are drawn the same way but need cargo, which CI does not have, so
`tools/gate.sh change` checks them instead of a generator (`research/battle-postmortem.md`).
GitHub and the wiki render Mermaid natively, so a reader of the repository sees the chart without
a build.

The site reads the same block back (`mermaid_chart_spec`) and draws it with Apache ECharts in the
site's palette: a tooltip per value, zoom when there are many categories, a switch between bars and
lines, the data as a table, an image export, and a card that maximizes to the viewport until
Escape. Two Mermaid comments carry what Mermaid cannot say and GitHub ignores: `%% series: a | b`
names the bar and line series, and `%% links: t1 | t2 ...` gives every bar or slice a link target,
in order, so a click opens that mark's page (a milestone, a duel's write-up). A pie of the `.ai/`
records links each slice to the table it summarises: the document that carries that table region
(this one first), at the heading before it, as `?col=Status&filter=proposed#heading`. The site then
shows only the rows whose cell in that column is the value, with a badge that names the filter
and clears it; GitHub ignores the query and opens the document. `check` resolves every chart link
like any other link and reports one that leads nowhere as a `chart` finding. The Mermaid source
stays under the chart. `diagrams-check` fails when a page's chart has no ECharts canvas or its
Mermaid source does not parse, as it fails on a Mermaid diagram that does not render.

`check` enforces both halves:

| Finding | When |
|---|---|
| `chart` | a Mermaid chart (pie, xychart, sankey, quadrant, radar) outside a generated region: a surfaces region, or any tool's `<!-- name:begin -->`/`<!-- name:end -->` markers |
| `library` | a tracked `package.json` depends on, a page loads, or code imports a library `[check] libraries` in `surfaces.toml` lists; the finding names what to use instead |

The libraries are one layer per kind of view: Apache ECharts for charts and graphs (here and in
the control panel, rule `project.panel-charts-go-through-one-layer`), D3 for bespoke layouts and
force physics, three.js for 3D, p5 for generative art, Mermaid for diagrams in Markdown and
Flowbite for the shell. Chart.js, Cytoscape and the like would be a second layer for a view
ECharts already draws, so they are listed.

## Parallel merges

A count in a region changes with every pull request that adds what it counts. Two pull requests
that each add a document both write the same new number, both pass on their own, and the second
to merge leaves `README.md` one short. Waiting for every owner to re-sync turns auto-merge into a
race. What keeps it from blocking:

- **A branch behind master still merges.** Branch protection is `strict: false` and requires only
  "Check and build", so master moving does not by itself block a pull request.
- **The default branch builds from regenerated regions**, so a count that slipped through never
  stops the site and the wiki from deploying, and master's next state is correct.
- **When a stale count turns a pull request red, its owner refreshes it**:
  `tools/surfaces/refresh-branch <branch>` merges `origin/master` into the branch, resolves a
  conflict inside a generated region by taking master's side, and runs `surfaces sync` before it
  commits, so the one merge commit, "Merge origin/master into <branch>", already holds the
  regenerated regions and a local pre-commit hook that runs `surfaces check` accepts it. A branch
  that already holds master but has stale regions gets one "Doc: [Surfaces] Regenerate derived
  regions after merging master" commit. `--push` pushes the result as a fast-forward of what was
  fetched, never a force. A conflict anywhere else aborts the merge and leaves the branch to its
  owner (exit 10). It works in a temporary worktree of its own, detached on `origin/<branch>`
  and removed on every exit, a failure or a signal included, so the checkout it is run from
  keeps its branch, HEAD and files: run from the shared primary checkout it never detaches
  master. The result stays in `refs/refresh/<branch>`, which a refresh without `--push` prints,
  so it can be pushed later (`git push origin refs/refresh/<branch>:refs/heads/<branch>`). The
  generator reuses the caller's `tools/surfaces/.venv` (`SURFACES_VENV` overrides it).
- **No CI job pushes to a pull request's branch.** A `refresh` job used to run `refresh-branch
  --push` as github-actions[bot] on every auto-merge branch for each push to master and dispatch
  Surfaces on it, because a push made with `GITHUB_TOKEN` starts no workflow. A bot-pushed head
  never merged: its `pull_request` workflows end `action_required` with no jobs, and even with the
  dispatched run's "Check and build" green on that head the pull request stayed BLOCKED until a
  human pushed (#70). With every pull request reset on each merge, the job only stranded them and
  filled the runner queue. Running it from CI again needs a push that starts `pull_request`
  workflows (an App or PAT token), which is the owner's call.

`tools/surfaces/test_refresh_branch.py` runs every path of `refresh-branch` against scratch
repositories, including that the calling checkout keeps its branch and that no temporary
worktree survives a failure or a signal, and `tools/surfaces/test_surfaces_workflow.py` keeps the workflow from pushing to a
pull request's branch again; both run in the Surfaces workflow.

## Open findings

A list of what is still missing restates the records it was read from, so it is a derived
surface like any index. `findings.py` computes it, the `open-findings` region renders it (here:
[`research/open-findings.md`](../../research/open-findings.md)), and `check` fails while the
committed copy is stale. The generator names no fact of a repository: `[findings]` in
`surfaces.toml` says where to look and which vocabulary the records use.

| Source | Declared by | A finding when | Kind | Label |
|---|---|---|---|---|
| Issue records | `issues = { in, id, non_scope_of }` | an issue that is not done depends on one that is not done | `blocked` | DERIVED |
| | | a `non_scope` entry of an issue whose status is in `non_scope_of` names no open issue (one that names only done issues was taken over and is dropped) | `out-of-scope` | DERIVED |
| | | an `evidence[].result` holds an open status: the sentence around it | the status's kind | OBSERVED |
| Tables | `tables = { in, columns, exclude }` | a row's cell under a header in `columns` begins with an open status | the status's kind | OBSERVED |
| Front matter | `front_matter = [{ in, field, values, kind }]` | the document's `field` holds one of `values` | `kind` | OBSERVED |
| Markers | `markers = { in, exclude }` | a passage is marked as below | the marker's `kind` | OBSERVED |
| Live | `live = [{ id, kind, command, items, keep, record, text }]` | only under `findings.py --live`: each item of the command's JSON at `items` whose fields match `keep` | `kind` | DERIVED |

`statuses` maps each open status of the records' vocabulary (`NOT RUN`, `PARTIAL`, ...) to a kind,
matched as a whole token, case and all. `kinds` declares every kind, with its meaning and in the
order the page lists them; a finding of an undeclared kind is a check finding. An issue's status
comes from its lifecycle stamps: `completed_at` is done, `verified_at` verified, `started_at`
active, none open; only a done issue satisfies a dependency, as in Majordomus.

A finding's **follow-up** is the open issues that own it: what a blocked issue waits on, the
issue whose evidence says NOT RUN while it is still open, the open issues a row or passage names
(`id` is how an issue id is written), or a marker's `follow-up=`. With none the page says
"none — needs an issue". The **evidence** is the record id or document with the line it is on.

A sentence in prose is not a status cell, so a document marks the passage it means:

```html
... settings. <!-- open-finding kind=failed -->In that round no route was found.<!-- /open-finding --> Then ...

<!-- open-finding kind=handoff follow-up=I0123 -->
A paragraph that is the finding, up to the next blank line.
```

The marker takes `kind=` (required, a declared kind) and `follow-up=` (an issue record that
exists). An inline marker runs to `<!-- /open-finding -->`, or to the end of its paragraph or the
next marker; one on its own line takes the paragraph after it. Markers inside code fences or
backticks are examples, not findings. `check` fails on an unknown attribute, an undeclared kind,
a follow-up that names no issue and a closing marker with no opening one in its paragraph.

Live sources answer for the moment (`majordomus plan drift` reads the forge and the branches), so
committing them would make every check depend on the clock. `findings.py --live` prints them with
the rest, and prints `NOT RUN: <id>: <why>` for a command that is missing or fails.

`findings.py [--json] [--live]` prints the same list from the command line (from the tracked
files only, where `sync` also reads files not yet added). `test_findings.py` runs every line of
it against scratch repositories and a `surfaces sync`/`check` round trip.

## Generators

Some derived text belongs to another tool: a region under that tool's own markers, or a whole
generated file. `surfaces.toml` declares each such tool once, with the command that regenerates its
output and the command that fails while it is stale:

```toml
[[generators.items]]
id = "readme-scoreboard"
outputs = ["README.md"]                                   # where a finding points
sync = ["tools/research/readme-scoreboard"]               # optional: regenerate in place
check = ["tools/research/readme-scoreboard", "--check"]   # exit non-zero while stale
```

`surfaces sync` runs every `sync` before it regenerates the regions, so a region that reads a
generated document sees its new text. `surfaces check` and `surfaces sync --check` run every
`check`, and a failure is a `generate` finding with the tool's last output. Commands run from the
repository root with `SURFACES_GENERATING=1` set, and a nested `surfaces` call skips the
generators, so a generator may itself call `surfaces`. A generator without `sync` is regenerated
by its own workflow, and `check` still proves it current.

Edit the text around a region, never inside it. `surfaces sync` rewrites the inside, and
`surfaces check` fails while it would.

A document's title comes from its front matter `title`, or else its first `# `, `<h1>` or
setext (`====`) heading, or else its file name.
Its summary comes from front matter `headline`, `description` or `statement`, or else its
first paragraph. A better first paragraph therefore improves every index that lists the
document.

Front matter that does not parse as YAML would otherwise be published as text: the page opens
with its raw fields and every table that reads its `id` or `title` gets an empty cell. `check`
fails on it (finding `front`), naming the parser's error. The usual cause is an unquoted value
containing `: `. `[check] front_matter_exclude` takes path globs for documents vendored from
elsewhere, which are fixed where they come from.

## Site

- **Shell.** The site uses the Flowbite admin shell: a navbar with search, a sidebar
  grouped by top-level directory, a breadcrumb, cards and tables. It is built from
  `assets/input.css` with Tailwind 4, Flowbite 4 and `flowbite-typography`.
- **Front-end libraries.** Mermaid 12 renders diagrams and Apache ECharts 6 renders charts.
  Both come from `node_modules` at the versions `package-lock.json` pins, and each loads only on
  pages that need it. This is the same release a CDN would serve, published with the site, so the
  page never depends on a third party at view time and every build renders the same.
- **Diagrams in the site's palette.** `assets/app.js` `mermaidConfig` gives Mermaid the site's
  palette in light and dark, with the Flowbite primary for borders and the grays of the cards,
  rounded nodes and smooth edges. Every diagram sits in a card with a maximize button, and Escape
  restores it.
- **Links.** A relative link to a published document becomes a link to its page, with
  GitHub-compatible heading anchors. A link to any other tracked file goes to GitHub at
  the exact commit the site was built from. Images load from that same commit.
- **Data.** `data/site.json` holds the chart inputs: weekly commits, the documentation map,
  per-document age at `HEAD` and plan progress. `data/search.json` holds the search index.
  Both are computed at build time from git, so the same commit always builds the same site.
- **AI layer page.** `ai.html` opens with charts counted from the `.ai/` records it lists:
  features, decisions and issues by status, rules by class and by origin (project or vendored),
  and issues per milestone. `Site.ai_chart` builds each one from the same column a table shows
  and draws it through `chart_figure`, the card the Markdown charts use. A slice or a bar links to
  `ai.html?col=<column>&filter=<value>#ai-<table>`, so a click filters that table to the rows it
  counted (`assets/app.js` `linkedFilter`, with a badge that clears it). The features table is in
  a collapsed block that opens when a filter targets it.

Every page ends with where it is linked from and what it links to, and the "All documents" page
draws every link between published documents as a force graph (drag, zoom, hover, click; a
`links.min` selector, kept in the URL, hides rarely linked documents). Both come from the links
the documents already contain, resolved the way `check` resolves them, so a new link shows up
on both ends with no index to maintain. The wiki gets the same "Linked from" line on every page.

The shell shows the document's title as the page heading, so an article's first `# ` heading
is dropped when it is that same title. The sidebar lists documents by title, with the path as
the tooltip. Search reads the title, path, summary, headings and the first part of the body,
and is shown at every width. `404.html` carries a `<base href>` set to the Pages path, because
GitHub serves it at whatever URL was missed.

## Declared pages

A page that is not a document, such as a view over a repository's own records, is declared in
`surfaces.toml` instead of written into the generator, which stays generic:

```toml
[[pages.items]]
id = "atlas"                                   # published as atlas.html, listed in the sidebar
title = "Research Atlas"
summary = "One paragraph: the page's intro, its meta description and its search entry."
data = ["tools/research/atlas/atlas", "data"]  # run at build time; stdout is the page's JSON
script = "tools/research/atlas/atlas.js"       # the page's own client script, copied to js/<id>.js
e2e = "tools/research/atlas/atlas.e2e.mjs"     # the scenario pages-check runs in Chrome
```

- **Data.** `surfaces site` runs `data` from the repository root and publishes its stdout as
  `data/<id>.json`. It must be a JSON object that names its `schema`; a failing command or other
  output stops the build. The data is derived at build time, so it cannot drift from the commit.
- **Cards.** When the data carries `cards` (`value`, `label`, `detail`, `view`, `chain` of
  `{label, href}`), the page renders them as plain HTML, so it states what it found with no
  script at all. A `chain` href is a repository path (a document becomes its page, any other file
  the forge at the built commit, a trailing `/` a directory); `view` is a query on the page itself.
- **Script.** The page's script draws into `#page-app` and builds on `window.surfaces`
  (`assets/app.js`): the URL state (`params`, `setParam`), the chart registry (`charts`, so a chart
  resizes and its `.chart-card` maximizes and restores on Escape), `echarts()` (the site's own
  copy), `onTheme`, `href(path)` and `restoreMax`. It sets `data-state="ready"` once every view is
  drawn, or `"error"` with a message when its data does not load. Tailwind scans the script, so its
  classes are in the stylesheet.
- **Checked.** `surfaces check` fails on a declaration without a slug id, title, summary, a data
  command whose first word is tracked, or a tracked `script` and `e2e` (finding `page`): a page no
  browser check drives is not built. `pages-check [SITE_DIR] [--shots DIR]` opens each declared
  page (from the built `data/pages.json`) at 1440 and 390 px in light and dark, waits for its
  ready state rather than a fixed time, and fails on a page error, a console error, sideways scroll
  or a relative link to nothing; then it imports the page's `e2e` module and calls its default
  export with `{browser, base, url, site, root, open, check}`.

## Wiki

GitHub keeps a wiki in a separate repository, `<repo>.wiki.git`, which exists only after
its first page has been saved once in the web interface. Until then the workflow's wiki
job fails and says so. Page names are derived from document paths, `README.md` becomes
`Home`, and `_Sidebar.md` and `_Footer.md` are generated. A page is the document's body: its front matter,
when it has one, is folded into a "Record metadata" block at the end, and a document whose
title only exists in its front matter gets it as a `# ` heading. The sidebar holds one collapsed
group per top-level directory, and titles that two documents share carry the file name. The
workflow copies the pages with `rsync --checksum`, so a page whose size and time happen to match
the old one is still replaced. Each sync replaces the whole
wiki: any edit made in the wiki is overwritten, and the footer of every page says so.

## Diagrams

A Markdown diagram is a ```mermaid block (rule `project.docs-are-derived-and-current`). GitHub
and the wiki render it natively, and the site renders it in its own palette. Two gates keep it
so:

- `surfaces check` (`check_diagrams`) fails on a fenced block drawn in characters: box-drawing or
  arrow glyphs, or ASCII boxes and arrows. Code and data fences are exempt, and so is a directory
  listing, told by its `├──` and `└──` branches. Upstream's documents are exempt through
  `[check] exclude`.
- `diagrams-check [SITE_DIR] [--shots DIR]` serves a built site on loopback, opens every page that
  carries a diagram in Chrome (`playwright-core`, `CHROME` overrides the path), and fails on any
  diagram that does not become an SVG or shows Mermaid's syntax error. `--shots` saves the first
  diagram of each page for a visual review.

`tools/research/test_surfaces_diagrams.py` pins what the detector flags and what it leaves alone.

## Feature maturity

`check` also judges the feature records of the layer's `features` section against the levels
`[features]` in `surfaces.toml` declares. A record's `status` must be a declared level. Its
`## Maturity` section must carry one `- **label**:` line for each label the level `requires`, and
no label the level neither requires nor `allows`. Each line must cite at least one thing the
repository resolves: a tracked path in a code span (`path`, `path/`, `path:line:Symbol`), an issue
record (`I0123`) or `PR #123`. Each failure is a finding of kind `feature`. The meaning of the
levels lives in the document `definition` names (here `.ai/repo/features/README.md`, "Maturity
levels"). The `feature-maturity-refs` generator proves each cited symbol and anchored line with
`tools/knowledge/refs`. Without a `[features]` table nothing is judged.
`tools/surfaces/test_maturity.py` pins the rules and runs the check on a scratch repository.

## In CI

`.github/workflows/surfaces.yml` runs `check`, `site`, `diagrams-check` and `wiki` on every push
and pull request, and `tools/research/e2e-coverage` (every web UI page has a browser check). From the default branch only, it deploys the site to Pages and pushes the wiki.
The checkout is full-history but blobless, because the charts read the commit log.

## Enforcement

`surfaces check` runs in three places:

- the pre-commit hook, declared in `.ai/repo/policy.yaml` under `enforcement` and
  reconciled by `majordomus doctor`;
- `tools/gate.sh change`;
- the workflow.

The project rule `project.published-surfaces-are-derived` states the contract.

## Files

| File | Role |
|---|---|
| `surfaces` | Entry point. Creates the virtual environment, then runs `surfaces.py` |
| `surfaces.py` | The generator: discovery, rendering, regions, check |
| `findings.py` | The open findings: their sources, read through `[findings]` in `surfaces.toml` |
| `maturity.py` | Feature maturity: each feature record's status and its `## Maturity` evidence, read through `[features]` in `surfaces.toml` |
| `assets/input.css`, `assets/app.js` | The site's stylesheet source and client script (Mermaid theme, ECharts builders, maximize) |
| `diagrams-check` | Every Mermaid diagram of a built site renders, in Chrome |
| `pages-check` | Every declared page of a built site works, in Chrome, with its own scenario |
| `package.json`, `package-lock.json` | Pinned front-end dependencies |
| `requirements.txt` | Pinned Python dependencies |
