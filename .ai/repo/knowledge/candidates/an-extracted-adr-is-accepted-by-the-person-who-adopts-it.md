---
schema: knowledge/v1
id: an-extracted-adr-is-accepted-by-the-person-who-adopts-it
kind: knowledge
class: convention
title: "Majordomus refuses an accepted ADR whose origin is extracted; the person adopting it makes it origin authored with an acceptance note"
description: "Before marking an ADR that Majordomus extracted as accepted."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [majordomus, adr, provenance, governance]
about: .ai/repo/adrs/0001-this-fork-is-a-private-research-target-and-never-interacts-w.md
route: platform
provenance:
  origin: authored
  derived_from:
    - commit:ff4b687c5393f3b598123ee03bfc4fcbc9366c88
    - file:.ai/repo/adrs/0001-this-fork-is-a-private-research-target-and-never-interacts-w.md
---

# Fact

Majordomus's ADR check refuses a record with `provenance.origin: extracted` and
`status: accepted` ("a record the tool derived is proposed until a person accepts it";
prismatic-majordomus `lib/adr.sh:225-232` at `origin/HEAD` 636acc8c845, Majordomus 1.2.0). Every
other status is allowed for an extracted record, and an extracted record must name evidence.

So acceptance is written by the person who adopts the record, not by flipping its status. In
ff4b687c53 (PR #22) each of the 27 extracted ADRs 0001-0030 the owner accepted became
`origin: authored`, kept its `derived_from` as the evidence it was extracted from, and gained a
note in its Decision section naming who accepted it, when and in which words
(`.ai/repo/adrs/0001-this-fork-is-a-private-research-target-and-never-interacts-w.md`, lines 6, 18 and 40).

# How to apply

To accept an extracted ADR: set `status: accepted` and `origin: authored`, keep `derived_from`,
add the acceptance note with the owner's words and date, then `majordomus adr check`.

# Review

Authored 2026-11-22 from prismatic-majordomus at 636acc8c845 and ff4b687c53 on origin/master;
awaiting the owner's review. `majordomus knowledge promote an-extracted-adr-is-accepted-by-the-person-who-adopts-it`, or
`majordomus knowledge reject an-extracted-adr-is-accepted-by-the-person-who-adopts-it --reason ...`.
