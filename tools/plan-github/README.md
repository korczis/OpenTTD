# plan-github

Mirrors the canonical plan in `.ai/repo/project` to GitHub milestones and issues. The records stay
the source of truth; GitHub is a projection (decision recorded 2026-11-22 by the owner).

```bash
tools/plan-github/plan_github.py --milestone records-and-status-tell-the-truth --dry-run
tools/plan-github/plan_github.py --milestone records-and-status-tell-the-truth
tools/plan-github/plan_github.py --all
python3 tools/plan-github/test_plan_github.py --coverage   # tests + every line of plan_github.py ran
```

- Data comes from `majordomus plan roadmap --json`, `plan list --json`, `plan show` and `plan body`;
  writes go through `gh api` against the `repository:` named in `.ai/repo/project/project.yaml`.
- Each milestone is titled `M<n>: <title>` with `n` its position in plan order (the same numbering
  as the README table); each issue is titled `<id>: <title>` and its body is `majordomus plan body`
  with canonical paths linked to the default branch.
- A hidden marker `<!-- plan-github: <canonical path> -->` in every projected milestone and issue lets
  a rerun find and edit what an earlier run created. If someone edits a body or description by hand
  and loses the marker, the issue is still found by its `<id>: ` title prefix and the milestone by its
  derived title, and the next sync restores the marker instead of creating a duplicate. Nothing on
  GitHub is deleted.
- Labels are derived: `plan`, `priority/p0..p3`, and one `plan/<status>` from the derived status
  (ready, blocked, active, verify, done, cancelled). DONE and CANCELLED close the issue.
- Run it after the records land on the default branch, so the canonical links resolve.

Repository-agnostic on purpose: nothing here names this fork, so it can move into Majordomus.
