---
schema: knowledge/v1
id: a-script-command-outside-a-suspendable-context-is-fatal
kind: knowledge
class: constraint
title: "A script that issues a command from its constructor, Save, Load or a valuator dies with a fatal error"
description: "When generating or bridging an AI or Game Script (PrismaticAI and similar)."
status: verified
epistemics: observed
date: 2026-11-22
tags: [scripting]
about: src/script/api/script_object.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/script/api/script_object.cpp
    - file:src/script/script_instance.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/scripting/README.md
---

# Statement

`src/script/api/script_object.cpp:ScriptObject::DoCommandPrep` throws `Script_FatalError` when the
instance cannot suspend, and the instance wraps the constructor and load in a scope that disables
commands (`src/script/script_instance.cpp`). Issue commands only from `Start()` and what it calls.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote a-script-command-outside-a-suspendable-context-is-fatal` with evidence, or `majordomus knowledge reject a-script-command-outside-a-suspendable-context-is-fatal --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/script/api/script_object.cpp:228-231 CanSuspend needs allow_do_command and a suspendable VM.
- src/script/api/script_object.cpp:260-263 DoCommandPrep throws Script_FatalError("You are not allowed to execute any DoCommand (even indirect) in your constructor, Save(), Load(), and any valuator.").
- src/script/script_instance.cpp:73, :227, :536 open ScriptObject::DisableDoCommandScope.
