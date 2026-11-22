---
schema: knowledge/v1
id: a-news-type-binds-its-display-setting-by-name
kind: knowledge
class: constraint
title: "Each news type finds its display setting by a string name at the time an item is shown; a mismatch with the ini var is an assert or a crash, not a build error"
description: "When adding a news type or renaming a news_display setting."
status: verified
epistemics: observed
date: 2026-11-22
tags: [goals-story-news-and-history, settings]
about: src/news_gui.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/news_gui.cpp
    - file:src/table/settings/news_display_settings.ini
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/goals-story-news-and-history/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/settings/README.md
---

# Statement

`_news_type_data` in `src/news_gui.cpp` names each `NewsType`'s setting as a string
(`"news_display.arrival_player"`, ...). `src/news_gui.cpp:NewsTypeData::GetDisplay` resolves it with
`GetSettingFromName` and `assert(sd != nullptr ...)`, then dereferences it. A row whose name has no
matching `var` in `src/table/settings/news_display_settings.ini` compiles, and fails only when an
item of that type is ticked or shown: an assert in an assert build, a null dereference without.
Add or rename the row and the ini entry together and show one such item before calling it done.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-news-type-binds-its-display-setting-by-name` with evidence, or `majordomus knowledge reject a-news-type-binds-its-display-setting-by-name --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/news_gui.cpp:329-346 _news_type_data names each setting as a string ("news_display.arrival_player", ...).
- src/news_gui.cpp:353-357 NewsTypeData::GetDisplay: GetSettingFromName(this->name), assert(sd != nullptr && sd->IsIntSetting()), then sd->AsIntSetting()->Read(nullptr).
- Called at display time: src/news_gui.cpp:781, :819, :1185.
