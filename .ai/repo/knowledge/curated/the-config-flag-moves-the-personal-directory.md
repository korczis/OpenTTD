---
schema: knowledge/v1
id: the-config-flag-moves-the-personal-directory
kind: knowledge
class: fact
title: "-c <file> makes the file's folder the working-dir search path, the config directory and the personal directory"
description: "When starting OpenTTD with -c for an experiment, a test or a second instance, and looking for its saves, logs, crash files or downloaded content."
status: verified
epistemics: observed
date: 2026-11-22
tags: [platform-and-files, research]
about: src/fileio.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/fileio.cpp
    - file:src/crashlog.cpp
    - file:tools/research/research
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/platform-and-files/README.md
---

# Statement

With `-c <file>`, `src/fileio.cpp:DetermineBasePaths` sets the working-dir search path to the
file's parent directory, and `src/fileio.cpp:DeterminePaths` uses it as the config directory and
(without XDG) as `_personal_dir`: `save/`, `autosave/`, `scenario/`, `content_download/`,
`private.cfg`, `secrets.cfg` and `openttd.log` are created or written there, and
`src/crashlog.cpp:CrashLog::CreateFileName` puts crash logs, dumps, saves and screenshots there
too. The same folder is searched first for `baseset/` and NewGRFs. A per-run config therefore
isolates a run's whole file footprint; `tools/research/research run` relies on it with
`-c <experiment>/runtime/openttd-research.cfg`, and `-x` keeps the run from rewriting that file.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote the-config-flag-moves-the-personal-directory` with evidence, or `majordomus knowledge reject the-config-flag-moves-the-personal-directory --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/fileio.cpp:821-839 DetermineBasePaths: with _config_file set, Searchpath::WorkingDir is the config file's parent directory.
- src/fileio.cpp:959-960 DeterminePaths uses it as config_dir; :996 private.cfg and :998 secrets.cfg there; :1017 _personal_dir = config_dir (under USE_XDG only when config_dir is not the XDG config home); :1029-1033 creates save, autosave, scenario and the other subdirectories in it; :1037 content_download; :1049 openttd.log.
- src/crashlog.cpp:83 CreateFileName prefixes _personal_dir.
- src/fileio_type.h:114 WorkingDir is the first search path.
- tools/research/research:375, :381 runs `-D -x -g -G <seed> -c <experiment>/runtime/openttd-research.cfg`.
