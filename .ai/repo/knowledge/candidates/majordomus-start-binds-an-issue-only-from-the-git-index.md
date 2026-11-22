---
schema: knowledge/v1
id: majordomus-start-binds-an-issue-only-from-the-git-index
kind: knowledge
class: fact
title: "majordomus start finds an issue only once its file is staged, and refuses one whose YAML has a multi-line plain scalar with '- ' lines"
description: "Before running majordomus start --issue for an issue file just written."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [majordomus, lifecycle, issues, yaml]
about: .ai/repo/project/issues
route: platform
provenance:
  origin: authored
  derived_from:
    - file:.ai/repo/project/issues/I0116.yaml
    - file:.ai/repo/workflows/task-lifecycle.md
---

# Fact

`majordomus start --issue <id>` asks the executable's `intents.binding` capability
(prismatic-majordomus `lib/intent_binding.sh:33-70`), which discovers issue files from the git
index. Recorded 2026-11-22 with Majordomus 1.2.0 in a worktree at 194552a5fa, by copying
`.ai/repo/project/issues/I0116.yaml` to a scratch `I9999.yaml` (removed afterwards) and asking
`majordomus-cli run intents.binding --input '{"issue":"I9999","paths":".ai/repo/knowledge"}'`:

- untracked: `standing: refused`; after `git add`: `standing: bound`;
- staged, with `why:` turned into a plain scalar continued by `  - ...` lines: `standing: refused`,
  logged as `ERROR list item without a parent key on line 10 code=malformed_yaml`.

# How to apply

Stage a new issue file before `majordomus start --issue`, and write every value as a single-line
(quoted) string; a list belongs under its own key.

# Review

Authored 2026-11-22 from the recorded run above; awaiting the owner's review.
`majordomus knowledge promote majordomus-start-binds-an-issue-only-from-the-git-index`, or
`majordomus knowledge reject majordomus-start-binds-an-issue-only-from-the-git-index --reason ...`.
