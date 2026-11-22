---
schema: knowledge/v1
id: a-build-from-a-modified-tree-cannot-join-a-clean-build
kind: knowledge
class: constraint
title: "A binary built from a modified work tree carries -m<hash> and is refused by a server of the same commit built clean (-g<hash>)"
description: "Before a multiplayer experiment, or when a client fails the version check against a server built from the same commit."
status: verified
epistemics: observed
date: 2026-11-22
tags: [build-and-packaging, network]
about: cmake/scripts/FindVersion.cmake
route: platform
provenance:
  origin: authored
  derived_from:
    - file:cmake/scripts/FindVersion.cmake
    - file:src/network/core/network_game_info.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/build-and-packaging/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
---

# Statement

The `find_version` target runs `cmake/scripts/FindVersion.cmake` on every build; when
`git diff-index HEAD` prints anything it sets `REV_MODIFIED 2`, and
`src/network/core/network_game_info.cpp:GetNetworkRevisionString` appends `-m<hash>` instead of
`-g<hash>`. For untagged builds `IsNetworkCompatibleVersion` compares the text after the last
`-`, marker letter included, so a client built from a dirty tree and a server built clean from the
same commit refuse each other. Build every participant from the same tree state (all committed,
or all with the same local diff) and record the revision string in the experiment manifest.
Complements `a-research-build-and-a-default-build-join-the-same-game`: CMake options do not
change the string, the tree state does.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-build-from-a-modified-tree-cannot-join-a-clean-build` with evidence, or `majordomus knowledge reject a-build-from-a-modified-tree-cannot-join-a-clean-build --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- cmake/scripts/FindVersion.cmake:31 runs `git diff-index HEAD`; :36-37 set REV_MODIFIED 2 when it prints anything.
- src/network/core/network_game_info.cpp:73 builds the suffix `"-" + "gum"[_openttd_revision_modified] + hash`, so a modified tree gives -m<hash>, a clean one -g<hash>.
- src/network/core/network_game_info.cpp:223-231 IsNetworkCompatibleVersion: tagged builds need a full match; otherwise ExtractNetworkRevisionHash (:96-101) takes the text from the last '-', marker letter included, and compares it.
- Caveat: in a research build (OTTD_RESEARCH_INSTRUMENTATION) :213-216 consult the research_net_compat policy first; its default is Strict (:120, :205), which falls through to the same comparison. A non-Strict policy is the operator's own widening (ADR 0029).
