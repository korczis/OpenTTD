---
schema: knowledge/v1
id: a-renamed-table-key-loses-its-value-silently
kind: knowledge
class: lesson
title: "A table chunk field whose key is renamed without a version gate is skipped on load with only an sl warning; the value is lost"
description: "Before renaming a member that a table-based chunk saves."
status: verified
epistemics: observed
date: 2026-11-22
tags: [saveload]
about: src/saveload/saveload.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/saveload/saveload.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/saveload/README.md
---

# Statement

`src/saveload/saveload.cpp:SlTableHeader` matches saved fields by name and logs "Field ... not found,
skipping" at `Facility::Sl` warning level, whereas a type mismatch is a corrupt-save error. Keep the old key
behind a version gate (rule `project.savegame-fields-are-version-gated`).

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote a-renamed-table-key-loses-its-value-silently` with evidence, or `majordomus knowledge reject a-renamed-table-key-loses-its-value-silently --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/saveload/saveload.cpp:1956 SlTableHeader matches by key; :1987 logs "Field '{}' of type 0x{:02x} not found, skipping" at Facility::Sl, Severity::Warning on load.
- src/saveload/saveload.cpp:2019 a type mismatch is SlErrorCorrupt("Field type is different than expected").
