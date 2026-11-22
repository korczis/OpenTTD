---
id: project.research-code-is-gated-and-labelled
version: 1
kind: rule
title: Research-only code is compile-gated, default-off, labelled at the point of change and removable
description: Access to internals is allowed for scoped research, and the cost is containment. Research code compiles only under a default-OFF CMake option, carries a RESEARCH-ONLY, UNSTABLE or DEBUG-ONLY label, and never bleeds into a default or release build.
statement: Put research-only code behind a default-OFF CMake option, label it RESEARCH-ONLY, UNSTABLE or DEBUG-ONLY where it is written, keep it to what the experiment needs, and keep default and release builds free of it.
status: active
class: blocking
depends_on: []
tags: [research, instrumentation, containment]
---

# Rationale

ADR 0004 opens the internals to research. What keeps that from becoming permanent debug
output is the compile gate: an absent symbol can be proven absent (`strings` on a
baseline-release binary), and a runtime flag cannot.

# Required behaviour

- Gate: `OPTION_RESEARCH_INSTRUMENTATION` (defines `OTTD_RESEARCH_INSTRUMENTATION`), or an
  option that requires it, such as `OPTION_PRISMATIC_MCP_SERVER`.
- Label: the doc comment says what the code exposes, whether it can affect behaviour, and its
  observer effects.
- A diagnostic interface binds to loopback only unless there is an explicit reason and
  protection for anything else.
- Verify: a change to gated code is built with the gate ON, because a default build does not
  compile it: `tools/gate.sh research` (I0022). `tools/gate.sh change` names that tier when the
  change touches a file carrying a research guard.
- Guard every member, too: an enum value, a window class, a widget id or a table entry that
  only research code uses sits inside the guard, appended last so no upstream value moves. A
  research member between `#ifdef <guard>` and `#endif` in an enum the script API exports
  (`WindowClass`, the `*Widgets` enums) is left out of `script_window.hpp` by
  `cmake/scripts/GenerateWidget.cmake`, so scripts see the same constants in every build.
- Prove absence, in the sources and in the binary (`tools/gate.sh change`; both alone:
  `tools/gate.sh research-free [build-dir]`):
  - `research_code_is_guarded`: every line of code in `src/` (comments excepted) that names
    `prismatic` or `research`, in any case, lies inside an `#if`/`#ifdef` on
    `OTTD_RESEARCH_INSTRUMENTATION` or `OTTD_PRISMATIC_MCP_SERVER`; `src/prismatic_mcp/` is
    exempt only because every `add_files` there carries `CONDITION OPTION_PRISMATIC_MCP_SERVER`,
    which the step checks.
  - `default_build_free_of_research`: `strings` on the default binary it built finds no string
    naming `prismatic` or `research`, in any case: no symbol, debug-info name, script-API
    constant, console command or label. Left out: the checkout and build paths and the version
    string, which carry the branch name, and a string that is only a StringID's name
    (`^STR_...$`; see below). NOT RUN, so PARTIAL, when `build/` is
    configured with a research option ON.
  - `cargo test --bin battlebrain` checks that both options default OFF in
    `cmake/Options.cmake` (rule `project.decision-authority-boundaries`, B8).

# What a default build still carries, and why

Each item is unavoidable and none is reachable from a default build; `research_code_is_guarded`
keeps it so.

- `STR_PRISMATIC_MCP_TOOLBAR_TOOLTIP` and `STR_PRISMATIC_MCP_ASK_CAPTION` in
  `src/lang/english.txt`. strgen has no conditional strings: upstream, too, compiles every
  string of every build, including those whose only user is compiled out on some platform
  (`STR_GAME_OPTIONS_VIDEO_VSYNC_TOOLTIP`, named only under `#ifndef __APPLE__` in
  `src/settings_gui.cpp`). So
  the two strings are in the default build's `lang/english.lng` and its generated
  `table/strings.h`, appended last so no upstream StringID moves. Only guarded code names them,
  so the binary references neither. A RelWithDebInfo binary built by GCC still carries both
  names, in debug info that names every constant of `table/strings.h` whether it is used or not
  (4811 `STR_` names on lundra, 2026-11-22), so the strings step leaves out a string that is only
  a StringID's name; the source step is what proves the two are never referenced.
- `add_subdirectory(prismatic_mcp)` in `src/CMakeLists.txt` and `test_prismatic_mcp.cpp`,
  `test_network_research_compat.cpp` in `src/tests/CMakeLists.txt`. The first adds no file to a
  default build (`CONDITION OPTION_PRISMATIC_MCP_SERVER`); the tests compile to empty units,
  their whole body being inside the guard. They are build-system entries, not code.
- The two options themselves in `cmake/Options.cmake`, which default OFF: they are the gate.

Guarded on 2026-11-22 (red-team finding 9), so a default build no longer has them:
`WID_TN_PRISMATIC` (and with it `WID_TN_END` is upstream's again), `WindowClass::PrismaticAsk`
and `WindowClass::PrismaticControl` (the latter landed unguarded while this was in review, and
`research_code_is_guarded` refused it),
the `MenuClickPrismatic`/`ToolbarPrismaticClick` stubs and their table entries in
`src/toolbar_gui.cpp`, and the `WID_TN_PRISMATIC`/`PrismaticAsk` script-API constants that put
both names into every default binary. Neither enum is saved or sent over the network: windows
are not in a savegame, `windows.cfg` stores a window by its `ini_key`, and a widget id is local
to its window.

# Failure behaviour

Ungated research code, or a research symbol in a baseline-release binary, blocks the change.

# Related

ADR 0004, ADR 0008, `cmake/Options.cmake`, `cmake/scripts/GenerateWidget.cmake`,
`src/console_cmds.cpp`, `tools/gate.sh`.
