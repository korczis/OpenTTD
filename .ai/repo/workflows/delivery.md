# Delivering to master: a pull request that merges itself when it passes

`korczis/OpenTTD` `master` is branch-protected (2026-11-22): a commit reaches it only after the
required check "Check and build" (workflow Surfaces) is green on it, for admins too. Several
sessions land the same day, so a direct `git push origin HEAD:master` loses the race whenever
master moves during the check. The owner's instruction of 2026-11-22: "use gh cli a vzdy enabluj
automerge pokud vse projde, at to odsypa, vse zdokumentuj, informuj ostatni a zacni pouzivat"
(use the gh CLI and always enable auto-merge so it merges when everything passes). The repository
allows auto-merge since then (`allow_auto_merge: true`).

| Step | Run |
|---|---|
| work on a branch in its worktree | `majordomus worktree create <branch>`, then `majordomus start ...` |
| bring master in and test the merged tree | `git fetch && git merge origin/master`, then the gate the change needs (`tools/gate.sh change`, the crate's `cargo test --release`) |
| close the task | `majordomus handover`, `majordomus finish --outcome completed --verify-command "<cmd>"` |
| push the branch | `git push origin HEAD:<branch>` |
| open the pull request | `gh pr create -R korczis/OpenTTD --base master --head <branch> --title "<commit-style title>" --body-file <file>` |
| let it merge itself | `gh pr merge <n> -R korczis/OpenTTD --auto --merge` |
| tell the peers | the board (`majordomus_announce`) and a message to the sessions working on the same paths |

## The checks a pull request runs

| Check (job name) | Workflow | What it proves |
|---|---|---|
| Check and build | `.github/workflows/surfaces.yml` | derived regions current, links and anchors resolve, the site and wiki build, the web UI's own tests |
| Research Rust tests | `.github/workflows/research-rust.yml` | `cargo test --release --locked` for `battlebrain`, `mcp-plan`, `mcp-intent` and `mcp-panel` with default features: the boundaries B1-B8 of rule `project.decision-authority-boundaries`, the decision contract and executor, the knapsack checks |

Research Rust runs on every pull request, every push to master and on dispatch. Its one job
always reports: its first step diffs the change (a pull request's merge commit against its first
parent, a push against the commit before it, a dispatch against its merge base with master) and,
when no path the four test binaries compile or read changed, succeeds without building. That
keeps it safe to require without blocking unrelated pull requests. It links the system libduckdb
of the `store` feature (`tools/research/mcp-rhai/build.rs`) from DuckDB's pinned release
archive, verified by SHA-256, and refuses to run when the locked `libduckdb-sys` encodes another
DuckDB version. Every action is pinned by commit SHA. It runs on the pull request's own pushes;
no CI job pushes to a pull request's branch (below), so nothing dispatches it there.

Only "Check and build" is required until the owner adds the second check:

```bash
gh api -X POST repos/korczis/OpenTTD/branches/master/protection/required_status_checks/contexts \
  -f 'contexts[]=Research Rust tests'
```

Until then a red Research Rust run is visible on the pull request but does not stop auto-merge,
so look at it before enabling auto-merge.

GitHub merges the pull request as soon as the required checks are green; nothing waits on the
session. When master moves first and the branch conflicts, the pull request stays open: merge
master into the branch again, re-run the gate and push; auto-merge stays enabled.

No CI job pushes to a pull request's branch. A branch behind master still merges (`strict:
false`); master regenerates its derived regions (counts, indexes, charts) on every push, so a
count that two pull requests both wrote is corrected there. When a stale count turns the pull
request's own "Check and build" red, its owner runs `tools/surfaces/refresh-branch <branch>`
(merges master, resolves conflicts inside generated regions, runs `surfaces sync`) and pushes
(`tools/surfaces/README.md`, "Parallel merges"). A conflict outside the derived regions is the
owner's as before. A Surfaces `refresh` job used to do this as github-actions[bot] on every push to
master, for every auto-merge branch; a bot-pushed head never merged (its `pull_request` workflows
end `action_required`, and the pull request stayed BLOCKED until a human pushed, #70), so it was
removed. Bringing it back needs a push that starts `pull_request` workflows, an App or PAT token:
the owner's call.

- The title follows the fork's commit-message form (`Add: [Component] ...`, CODINGSTYLE.md); the
  merge commit is a plain merge, so every commit of the branch is kept.
- The body says what changed and how it was validated, each check as PASS, FAIL or NOT RUN, and
  ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)` when an agent wrote it.
- The "Commit checker" (OpenTTD-git-hooks) judges a pull request's own commits against the
  `<keyword>( #<issue>)?: [<component>] <Details>` form and skips the merge commits that bring master
  in (a4a295be0f), with the whole history fetched so every commit of the range is present
  (eb8a984273). Before those two fixes it failed on every pull request that merged master (#3, #5)
  and was treated as advisory; now it is one of the checks that must pass, and a failure names a
  commit of the branch to fix (a new commit, never an amend of a pushed one).
- A pull request is opened only against `korczis/OpenTTD`. Never against upstream OpenTTD: `gh`
  defaults to the parent repository of a fork, so `-R korczis/OpenTTD` is always given.
- Direct pushes to `master` stay possible for a commit already green on Surfaces with master
  unchanged since, but a pull request with auto-merge is the default.
