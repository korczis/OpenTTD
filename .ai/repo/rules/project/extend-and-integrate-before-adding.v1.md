---
id: project.extend-and-integrate-before-adding
version: 1
kind: rule
title: Extend and integrate what exists before adding a parallel piece
description: A change first looks for the module, route, view, script, kernel or record that already does part of the job, and extends it cleanly. A second implementation of something that exists is folded into the first or removes it in the same change. Each change opens the next one up (a generic shape, data other surfaces can read) and pays down debt rather than adding to it.
statement: Extend the existing piece before writing a parallel one, integrate with what exists (the store and its views, the Rhai library, AlgoGlyph through the hub, the MCP catalogue, the blackboard and the mesh), and leave no second implementation behind.
status: active
class: blocking
depends_on: [project.mcp-surfaces-are-derived@1, project.docs-are-derived-and-current@1]
tags: [design, debt, integration, derivation]
---

# Rationale

The owner's order (2026-11-22, Czech, in the session that built co-play and the replan service):
"pokud se da neco snadno a ciste rozsirit, aby se dotahla implementace, co nejvice funkcionality,
otevrelo co nejvice dvirek, veci byly co nejvice feature proof, ale zaroven integrovali existujici,
spis uklizeli debt nez jej pridavali, tak to vzdy delej a toto chovani enforce". In English: when
something can be extended easily and cleanly, so that the implementation is finished, as much
functionality as possible works, as many doors as possible open and things are future-proof,
while what exists is integrated and debt is cleared rather than added, always do so, and enforce
this behaviour.

The same session shows the cost of the opposite. The owner's company was played by `mcp-human`, a
second strategy beside PrismaticAI's: buses only, two depots with no exit, no route that earned.
The arena's lineup was a hand copy of the battle's contestants and had fallen behind. Monthly
replanning would have repeated SQL that the store can hold once.

# Required behaviour

- **Look first.** Before writing a module, a route, a script, a query or a record, search for the
  one that already does part of it: `majordomus search`, the Rhai library
  (`tools/research/mcp-panel/rhai/`), the store's views (`GET /api/db/tables`), the hub's
  catalogue (`hub.catalogue`, `algoglyph.kernels`), the MCP `tools/list`. Name in the commit what
  was reused.
- **Extend it cleanly.** Add the field, the view, the function or the option to the existing piece
  in its own shape, so every surface that reads it gains it (derived surfaces,
  `project.mcp-surfaces-are-derived`). Prefer the generic form when it costs no more: one table
  that drives creation, docs and tests, rather than one entry per case.
- **One implementation.** A change that would leave two implementations of one thing folds the
  newer into the older, or retires the older in the same change, with its docs. Where that cannot
  happen in one change, the commit names the remaining duplicate and the next step that removes it.
- **Open doors.** Data a change produces goes where others can read it: the bus and the DuckDB
  store, a view when it is an aggregate, the shared blackboard or the mesh when other agents act
  on it. Data that only the producing script can see is not finished.
- **Debt goes down.** A change touching debt it can clear cheaply clears it: a stale hand copy, a
  silent failure path, an undocumented fallback.

# Failure behaviour

A violation is a review finding against the change. The fix is to fold the duplicate, extend the
existing piece instead, or record the remaining duplicate with its next step. No tool decides this
rule. A reviewer owns it, so it has no `x-majordomus` block.

# Verification

A reviewer reads the change with this rule's five points. The derived-surface tests
(`project.mcp-surfaces-are-derived`) and the docs check (`project.docs-are-derived-and-current`)
catch the mechanical part: a hand-listed surface or a stale doc.
