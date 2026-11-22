---
schema: knowledge/v1
id: a-language-pack-must-match-the-binarys-english-txt
kind: knowledge
class: fact
title: "A .lng file built from a different english.txt is skipped without an error; with none valid the game refuses to start"
description: "When running a binary away from its build directory, mixing lang/ directories, or diagnosing \"No available language packs\"."
status: verified
epistemics: observed
date: 2026-11-22
tags: [strings-and-languages, platform-and-files]
about: src/strings.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/strgen/strgen_base.cpp
    - file:src/strgen/strgen.cpp
    - file:src/strings.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/strings-and-languages/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/platform-and-files/README.md
---

# Statement

strgen writes `LANGUAGE_PACK_VERSION` into `table/strings.h` and every `.lng` header; it is a hash
over each string's ID, name and counted commands (`src/strgen/strgen_base.cpp:StringData::Version`),
so adding, renaming or re-parameterising any English string changes it.
`src/strings.cpp:LanguagePackHeader::IsValid` rejects a header with another version, and
`src/strings.cpp:InitializeLanguagePacks` logs the rejected file only at `-d misc=3` and stops with
"No available language packs (invalid versions?)" when nothing valid is left on any search path.
Ship or point a binary at the `lang/` directory built with it.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-language-pack-must-match-the-binarys-english-txt` with evidence, or `majordomus knowledge reject a-language-pack-must-match-the-binarys-english-txt --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/strgen/strgen_base.cpp:129-145 StringData::Version hashes each string's index, name and counted commands; :729 writes it into the language header; src/strgen/strgen.cpp:258 emits LANGUAGE_PACK_VERSION.
- src/strings.cpp:2018-2021 LanguagePackHeader::IsValid requires version == LANGUAGE_PACK_VERSION.
- src/strings.cpp:2234 (FillLanguageList, :2221) logs "is not a valid language file" at Severity::Notice, i.e. -d misc=3 (src/debug_type.h:18).
- src/strings.cpp:2255 (InitializeLanguagePacks) `UserError("No available language packs (invalid versions?)")` when nothing is left.
