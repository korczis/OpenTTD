---
schema: knowledge/v1
id: the-owners-answers-of-2026-11-25-on-majordomus-and-the-live-game
kind: knowledge
class: fact
title: "The owner's answers of 2026-11-25: Majordomus learns this session's four lessons generically, and the live game on :8800 is left as it is"
description: "When a session asks whether the live game's stack may be redeployed with merged PrismaticAI fixes, or which of this repository's lessons were handed to prismatic-majordomus and on whose word."
status: verified
epistemics: decided
date: 2026-11-25
tags: [decision, owner-answer, majordomus, live-game]
route: project
provenance:
  origin: authored
  derived_from:
    - file:research/live-arena.md
relations:
  - type: relates_to
    target: file:.ai/repo/knowledge/curated/auto-approve-is-the-owners-switch.md
---

# Statement

Session 7709ab47-7786-494d-bbcc-679b293569a7 (openttd-36, Claude Code) received four answers
from the owner on 2026-11-25:

1. The owner selected all four generic lessons, plus a free-text instruction to consult the
   other models over several passes. The four lessons are build slots and locks, owner answers
   in the repository, plan against merged pull requests, and baseline-relative gates.
   `prismatic-majordomus` is to implement them generically, each as a design for review and
   then an implementation with tests.
2. The live game on :8800 (seed 21647, then 1999-09, which the owner co-plays) is left as it
   is. It is not rebuilt with #264, and the merged PrismaticAI fixes (#271/#272/#276/#279/
   #280/#285) are not loaded into it. Those fixes arrive with the next fresh game.
3. A chat turn asked that everything learned in this session be used to improve the generic
   Majordomus.
4. Later the same day the owner decided how five panel pull requests land: #283, #311, #305,
   #263 and #319. openttd-51 may review, but may not approve. Each of the five merges once
   openttd-51's review finds no defect, the gate passes and CI is green. Other pull requests
   are not covered.

The records below are the answers, verbatim. They are not a paraphrase. Each block is the
record that `majordomus question record` writes, the owner-answer/v1 kind that
prismatic-majordomus adds in I4669, generated from the session's own provider transcript
with `--transcript` and `--locator` (`text_source: transcript-checked`). The question, the
options, the asking session and both times were read from the transcript, not typed. The
selected labels and the free text were checked byte for byte against the tool result.
The digests are sha256 values of the stored text. Once this repository's Majordomus carries
`question record`, these records move unchanged to `.ai/repo/answers/<id>.yaml`, and other
records cite them as `owner-answer:<id>`.

The relay of these answers to the workers was not verbatim in two places. It called the
first two questions "translated from Czech", but the transcript shows that both were asked
in English. It also wrote the chat turn's "sesion,  z" with one space instead of two. Both
facts are why a record is written from the transcript rather than from a relayed copy.

# Evidence

## A-20261125-which-majordomus-lessons

```yaml
schema: "owner-answer/v1"
id: "A-20261125-which-majordomus-lessons"
channel: "question-tool"
provider: "claude-code"
asked:
  session: "7709ab47-7786-494d-bbcc-679b293569a7"
  at: "2026-11-25T05:06:23.177Z"
  session_source: "transcript"
answered_at: "2026-11-25T05:53:09.117Z"
text_source: "transcript-checked"
source:
  transcript: "7709ab47-7786-494d-bbcc-679b293569a7.jsonl"
  locator: "toolu_01WppK7NfiRXAVW2qWPYgS5s#0"
  sha256: "e5eb338b5c0f57bf8b11f6f97b2044f2bf9287757bb3da9a6a2990499a1fffd0"
question: "Yes. From this session I've distilled these generic lessons for prismatic-majordomus (nothing OpenTTD-specific). Which should I implement there? (Each one first as a design for review, then implementation with tests in that repo's own workflow.)"
multi_select: true
options:
  - label: "Build slots and locks"
    description: "`majordomus slot take/release/status`: the owner file carries pid+session, a stale lock (dead pid, gone session) is reclaimed automatically, an ownerless slot is refused. Here dead/ownerless slots blocked builds 4 times."
  - label: "Owner answers in the repo"
    description: "`majordomus decision` today writes only to .ai/local (uncommitted), which contradicts 'record owner answers in the repo'. Add a tracked owner-answer record (verbatim question+answer, session, time) and link it to PRs."
  - label: "Plan ↔ merged PRs"
    description: "A reconciliation command: maps merged PRs to issues and milestones and proposes transitions with evidence. Here milestones showed 0/13 even though the work was merged."
  - label: "Baseline-relative gates"
    description: "A gate verdict relative to the base (branch vs master over repeated runs and neutral perturbations) for nondeterministic checks. It generalises the owner's 'compare to master' rule. Plus resumable agents: WIP commit+push and handover on interruption."
selected:
  - "Build slots and locks"
  - "Owner answers in the repo"
  - "Plan ↔ merged PRs"
  - "Baseline-relative gates"
free_text: "porad se s ostatnimi ai/llm, na vice pruchodu a vymslete nejlepsi reseni "
digest:
  question: "c64f2e1db1abd7d716010e29b0c0fea60bedbd7f9e1667e3a6aabe98f428bfb2"
  answer: "ad3aaeca6b8300d15e2fd3119fe1a12a52b732d8becf4d70bdcc0508912e905c"
translation:
  language: "en"
  answer: "consult the other AIs/LLMs, in several passes, and work out the best solution"
settles:
  - "other:prismatic-majordomus I4669"
recorded:
  session: ""
  at: "2026-11-25T07:57:28Z"
  by: "Tomas Korcak <korczis@gmail.com>"
```

## A-20261125-leave-the-live-game-for-now

```yaml
schema: "owner-answer/v1"
id: "A-20261125-leave-the-live-game-for-now"
channel: "question-tool"
provider: "claude-code"
asked:
  session: "7709ab47-7786-494d-bbcc-679b293569a7"
  at: "2026-11-25T06:08:52.218Z"
  session_source: "transcript"
answered_at: "2026-11-25T06:28:19.042Z"
text_source: "transcript-checked"
source:
  transcript: "7709ab47-7786-494d-bbcc-679b293569a7.jsonl"
  locator: "toolu_0144SkG9Nph3LcPvwKMkuean#0"
  sha256: "e2b1b3a02dc93a2a72e076a41c8908f87d21ff1e28c332d0b6caf754d24225ef"
question: "openttd-51 asks me to take over the live game's stack on :8800 and redeploy it. The game is seed 21647, now 1999-09; you co-play in it. The redeploy would mean rebuilding the C++ server with #264 (only after `openttd_test [prismatic_mcp]` passes) and loading the merged PrismaticAI fixes (#271/#272/#276/#279/#280/#285) into the running game via rescan_ai + save + load. That changes the game you're playing. What should I do?"
options:
  - label: "Leave it for now (Recommended)"
    description: "Don't touch the game. The fixes arrive with the next fresh game; first I'll wait for #291 and the road-town fix and prove them with a replay."
  - label: "Load only the PrismaticAI fixes"
    description: "rescan_ai + save + load of the current master PrismaticAI into the running game, without rebuilding the server. A short pause in the game."
  - label: "Full redeploy"
    description: "Rebuild the server with #264 after the test passes, then save, restart the stack and load the save with the new PrismaticAI. A longer outage."
selected:
  - "Leave it for now (Recommended)"
digest:
  question: "2515201cf4ef71308199b426d6a69d05940c127a85ce93251474d842f87ca885"
  answer: "a5f568a3412a8997c0e1026ab85ddc8022d362abae68c578f995c01a2e49c618"
settles:
  - "other:the live game on :8800 is not redeployed"
recorded:
  session: ""
  at: "2026-11-25T07:57:30Z"
  by: "Tomas Korcak <korczis@gmail.com>"
```

## A-20261125-evolve-majordomus-from-lessons

```yaml
schema: "owner-answer/v1"
id: "A-20261125-evolve-majordomus-from-lessons"
channel: "chat"
provider: "claude-code"
asked:
  session: "7709ab47-7786-494d-bbcc-679b293569a7"
  at: "2026-11-25T06:06:37.664Z"
  session_source: "transcript"
answered_at: "2026-11-25T06:06:37.664Z"
text_source: "transcript-checked"
source:
  transcript: "7709ab47-7786-494d-bbcc-679b293569a7.jsonl"
  locator: "2568d1e3-20f6-45e8-9d51-2542f7bf4082"
  sha256: "8424a8b8469b031d03b962c16070e1ad7707cb4385efb83cae7e1b7232b1cdd0"
free_text: "muzes na zaklade vseho z teto sesion,  z lessons learned, obecne / dynamicky zlepsi ~/dev/prismatic-majordomus /evolve it? proste pouzit to co jsme se naucili zde, v tomto projektu pouzit ke zlepseni generickeho majordomus?"
digest:
  question: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  answer: "233ff00b384ea882434a639df42229691bf15bb5073a5a6a068a55008aed94d3"
translation:
  language: "en"
  answer: "Based on everything from this session and its lessons learned, can you generically and dynamically improve ~/dev/prismatic-majordomus, evolve it? Simply use what we learned here, in this project, to improve the generic Majordomus."
settles:
  - "other:prismatic-majordomus I4669"
recorded:
  session: ""
  at: "2026-11-25T07:57:33Z"
  by: "Tomas Korcak <korczis@gmail.com>"
```

## A-20261125-panel-prs-land-on-a-clean-review

```yaml
schema: "owner-answer/v1"
id: "A-20261125-panel-prs-land-on-a-clean-review"
channel: "question-tool"
provider: "claude-code"
asked:
  session: "7709ab47-7786-494d-bbcc-679b293569a7"
  at: "2026-11-25T11:04:36.649Z"
  session_source: "transcript"
answered_at: "2026-11-25T17:04:21.158Z"
text_source: "transcript-checked"
source:
  transcript: "7709ab47-7786-494d-bbcc-679b293569a7.jsonl"
  locator: "toolu_01XxQDbTq5hCbQtC72jZGYt1#0"
  sha256: "309e2fe7b69e2c2ed3f251fd984169f5ac6b4d54bc4396992fc78358ef6b7687"
question: "openttd-51 isn't allowed to give approvals: the permission system says an approval from this GitHub account is yours. It can do reviews with findings, without a verdict. The merge decision is therefore yours. How should my panel PRs land? #283 (FunnelRoll before MaintenanceRound): review with no defect, gate PASS, fitness 7/7, CI all green. #311 (replan realised profit): review with no remaining defect, gate PASS, CI running. #305 and #263: gate PASS, openttd-51's review still coming. Plus the #261 fix-up (messages and proposals only for your company)."
options:
  - label: "Review clean + CI green (Recommended)"
    description: "Each of these 5 PRs merges once openttd-51's review finds no defect, the gate passes and CI is green. I record your answer in the repo, verbatim. Other PRs aren't covered."
  - label: "Now #283 only"
    description: "Merge only #283 (everything ready). Ask about the others separately."
  - label: "I decide each one"
    description: "Merge nothing; I'll ask you for each PR when it's ready."
selected:
  - "Review clean + CI green (Recommended)"
digest:
  question: "721c7ce27f99132ab827d6950cdfdb74e2a9775d476d36dda14ee7374a2c76ec"
  answer: "4a5ecfe723d6f718d2fabadf523751cbeb5a12d5a81821a1f5a1a42eff5d360f"
settles:
  - "other:#283"
  - "other:#311"
  - "other:#305"
  - "other:#263"
  - "other:#319"
recorded:
  session: ""
  at: "2026-11-25T17:13:04Z"
  by: "Tomas Korcak <korczis@gmail.com>"
```

The transcript, `~/.claude/projects/-Users-korczis-dev-OpenTTD/<session>.jsonl`, stays on the
owner's machine. A reader who holds it can recompute each `source.sha256`. The value is the
sha256 of the raw lines that the locator names, each followed by a newline: the tool_use line,
then the tool_result line, or the single message line.
