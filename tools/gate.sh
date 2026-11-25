#!/usr/bin/env bash
# Layered local validation gate for this OpenTTD research fork.
# See AGENTS.md and research/README.md for what each tier means and why.
set -euo pipefail

SELF="$(basename "$0")"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Exit status of a tier whose run was incomplete for a reason outside the code (PARTIAL in the
# research/README.md taxonomy): distinct from a failure (1) and from a usage error (2).
EXIT_PARTIAL=3

BASESET_DOC="research/baseset-prerequisite-macos.md"
RESEARCH_BUILD_DIR="build-gate-research"
# Every guard of research-only code, as cmake/Options.cmake defines it.
GATED_MACROS='OTTD_RESEARCH_INSTRUMENTATION|OTTD_PRISMATIC_MCP_SERVER'
# The words that name research-only code, matched case-insensitively: the fork's subsystem
# (PrismaticMCP, prismatic_mcp, WID_TN_PRISMATIC, WindowClass::PrismaticAsk, STR_PRISMATIC_*), its
# label (RESEARCH-ONLY), its guards (OTTD_RESEARCH_*) and the research console commands and their
# state (research_status, _research_net_compat, ResearchNetCompat). Upstream code names neither.
RESEARCH_WORDS='prismatic|research'
# Extra CMake options for every configure this gate runs, word-split: e.g.
# GATE_CMAKE_ARGS="-DOPTION_DEDICATED=ON" on a headless Linux host without SDL3 or Allegro, where
# the default (GUI) configure stops at CMakeLists.txt "SDL3 or Allegro is required".
read -r -a GATE_CMAKE_EXTRA <<< "${GATE_CMAKE_ARGS:-}"

usage() {
  cat <<EOF
Usage: $SELF <smoke|change|research|research-free|full> [options]

  smoke     Cheapest check, for fast iteration. git diff --check, then an
            incremental build against the existing build/ directory.
            Requires build/ to already be configured (run 'change' or
            'full' first if it isn't).
              -R <ctest-regex>  also run matching tests via 'ctest -R'

  change    Standard check before calling a task done. Configures build/
            if missing, incremental build, full existing ctest suite,
            diff hygiene, and a summary of untracked/modified files to
            review for accidental generated output, plus a check that the
            generated Prismatic MCP contributor docs are not stale
            (tools/check-prismatic-docs.sh), that every AI-battle record is
            reproducible and its leaderboard derived (tools/research/
            ai-battle/battle.py check), that the town-authority model and
            its outputs are what src/ derives (tools/research/town-model/
            derive.py --check), that the AlgoGlyph kernel contracts are what
            the pinned AlgoGlyph source derives (algoglyph-contracts --check;
            NOT RUN, so PARTIAL, without an AlgoGlyph checkout), and
            that every published surface
            agrees with the repository: Markdown regions current, every
            relative link and anchor resolving (tools/surfaces/surfaces check),
            and every file and symbol the .ai/ records cite still exists
            (tools/knowledge/refs check); then lists the records that cite a
            file this change touches (tools/knowledge/refs impact HEAD)
            and how many src/ files no subsystem map tracks
            (tools/knowledge/refs coverage src/, informational).
            Checks the AlgoGlyph submodule's pin first (tools/research/
            algoglyph-pin check; the submodule is checked out at the pin,
            without node_modules, when it is missing: mcp-rhai links its
            Rust crate), and when the change touches the pin or the native
            kernels, runs their tests (cargo test --bin mcp-rhai algoglyph,
            --bin battlebrain native_rust; offline).
            Runs every test_*.py of the repository under line tracing and
            fails on a changed Python line no test ran, by file and line
            (tools/coverage/changed-lines, rule project.new-code-tested);
            changed C++, Rust, JavaScript and shell files are named there
            as not judged yet, never refused.
            Always proves the decision-authority boundaries (cargo test
            --bin battlebrain, offline; rule project.decision-authority-
            boundaries), that every source line naming research is inside
            a research guard, and, after the build, that the default binary
            carries no string naming research (NOT RUN, so PARTIAL, when
            build/ is configured with a research option ON).
            When the change touches tools/research/real-world, also runs its
            cargo tests and replays the Brno snapshot against its recorded
            hashes (offline). When it touches a file of COVERAGE_TABLE (the
            Prismatic client, Battlebrain, the live-decision executor), requires
            every counted line of that file's criterion to be run by a test or
            exempt with its reason (cargo llvm-cov; rule
            project.new-code-is-covered; NOT RUN, so PARTIAL, without it).
            Names the research tier when the change touches research-gated
            code.

  research  The change tier for research-gated code, which a default
            build never compiles. Configures $RESEARCH_BUILD_DIR/ with
            OPTION_RESEARCH_INSTRUMENTATION=ON (and OPTION_PRISMATIC_MCP_SERVER=ON
            once cmake/Options.cmake declares it), builds, runs the full
            ctest suite, proves the research console command is in the
            binary, and runs the [prismatic_mcp] tests when the server is built.

  research-free [build-dir]
            Only the two checks that a default build is free of research
            code: the source guards, then 'strings' on build-dir/openttd
            (default build/; NOT RUN, so PARTIAL, unless it is a default
            build).

  full      Expensive clean-room validation: wipes and recreates
            build-fullgate/, configures + builds + tests from scratch.
            For significant changes or an experimental baseline -- not
            required for small or documentation-only changes.
            Then runs the Rust performance benchmarks of mcp-rhai against
            the newest committed record of this host (tools/research/
            rust-bench check, research/rust-bench.md): a median more than
            25% slower and above the baseline's slowest sample fails; no
            such record, no cargo, or uncached dependencies on a host that
            cannot download them, is NOT RUN, so PARTIAL. Only in this
            tier: a fresh run takes minutes.

  -h, --help   Show this help and exit.

Base graphics set: the regression_* cases need one (OpenGFX or the
original TTD data). Without one they hang on macOS instead of failing,
so every tier that runs ctest checks first; when none is reachable it
runs the other cases, reports regression_* as NOT RUN, and exits
$EXIT_PARTIAL (PARTIAL) instead of claiming PASS. See $BASESET_DOC.
The same holds for a build configured without liblzma: the regression
savegames are LZMA, so regression_* are NOT RUN and the tier is PARTIAL.

Environment: GATE_CMAKE_ARGS adds CMake options to every configure, e.g.
GATE_CMAKE_ARGS="-DOPTION_DEDICATED=ON" on a headless Linux host without
SDL3 or Allegro.

Guarantees: never commits or pushes anything, never writes outside this
repository, fails fast on the first failing step.
EOF
}

nproc_portable() {
  if command -v nproc >/dev/null 2>&1; then
    nproc
  elif command -v sysctl >/dev/null 2>&1; then
    sysctl -n hw.ncpu
  else
    echo 4
  fi
}

JOBS="$(nproc_portable)"

diff_check() {
  echo "==> git diff --check"
  git diff --check
  echo "==> git diff --cached --check"
  git diff --cached --check
}

# The directories OpenTTD searches for base sets that a user or package can populate
# (docs/directory_structure.md): the personal directory per OS, the shared data
# directories, and the build directory's own baseset/.
baseset_search_dirs() {
  local build_dir="$1"
  printf '%s\n' \
    "$ROOT_DIR/$build_dir/baseset" \
    "$ROOT_DIR/baseset" \
    "$HOME/Documents/OpenTTD/baseset" \
    "$HOME/Documents/OpenTTD/content_download/baseset" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/openttd/baseset" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/openttd/content_download/baseset" \
    "$HOME/.openttd/baseset" \
    "/usr/share/games/openttd/baseset" \
    "/usr/local/share/games/openttd/baseset" \
    "/Library/Application Support/OpenTTD/baseset"
}

# True when a usable base graphics set is reachable. An .obg file alone is not enough: the
# build copies the metadata of the original TTD sets into baseset/, and those need the
# original data files. Usable means an OpenGFX-style set (an .obg that is not orig_*, loose
# or inside a .tar) or the original graphics (trg1r.grf, any case).
baseset_available() {
  local build_dir="$1" dir f
  while IFS= read -r dir; do
    [[ -d "$dir" ]] || continue
    while IFS= read -r f; do
      case "$f" in
        *.tar)
          # grep -c reads the whole listing: grep -q would stop at the match, tar would die of
          # SIGPIPE, and pipefail would report a present base set as missing (seen on GNU tar).
          if [[ "$(tar -tf "$f" 2>/dev/null | grep -ciE '\.obg$')" -gt 0 ]]; then return 0; fi ;;
        *.obg|*.OBG)
          case "$(basename "$f" | tr '[:upper:]' '[:lower:]')" in
            orig_*) ;;
            *) return 0 ;;
          esac ;;
        *)
          return 0 ;;
      esac
    done < <(find "$dir" -maxdepth 3 \( -iname '*.tar' -o -iname '*.obg' -o -iname 'trg1r.grf' \) 2>/dev/null)
  done < <(baseset_search_dirs "$build_dir")
  return 1
}

# Run the ctest suite of one build directory. Returns 0 on PASS, 1 when a case failed, and
# $EXIT_PARTIAL when the regression cases had to be left out for want of a base graphics set
# and everything else passed. Callers run it as `run_ctest dir || status=$?`, which disables
# set -e inside it, so every ctest status is taken explicitly here.
# The regression cases load LZMA-compressed savegames (regression/*/test.sav start with "OTTX"), so
# a build configured without liblzma (link_package LIBLZMA is only ENCOURAGED) runs them to empty
# output and reports four bogus diffs; such a build leaves them out as NOT RUN instead (lundra,
# 2026-11-22, a Linux host without liblzma-dev).
lzma_built_in() {
  local cache="$1/CMakeCache.txt"
  [[ -f "$cache" ]] || return 0
  # FindLibLZMA probes the library's functions only once it found headers and library.
  grep -qE '^LIBLZMA_INCLUDE_DIR:[A-Z]+=' "$cache" && ! grep -qE '^LIBLZMA_INCLUDE_DIR:[A-Z]+=.*NOTFOUND' "$cache" \
    && grep -qE '^LIBLZMA_HAS_AUTO_DECODER:INTERNAL=1' "$cache"
}

run_ctest() {
  local build_dir="$1" rc=0
  if ! lzma_built_in "$build_dir"; then
    echo "==> $build_dir was configured without liblzma: regression_* cannot load their savegames"
    echo "==> ctest --test-dir $build_dir -j$JOBS --timeout 120 -E '^regression_'"
    CTEST_OUTPUT_ON_FAILURE=1 ctest --test-dir "$build_dir" -j"$JOBS" --timeout 120 \
      -E '^regression_' || rc=$?
    echo "regression_*: NOT RUN -- configure with liblzma (its headers and library), then reconfigure $build_dir"
    [[ "$rc" -eq 0 ]] || return 1
    return "$EXIT_PARTIAL"
  fi
  if baseset_available "$build_dir"; then
    echo "==> ctest --test-dir $build_dir -j$JOBS --timeout 120"
    CTEST_OUTPUT_ON_FAILURE=1 ctest --test-dir "$build_dir" -j"$JOBS" --timeout 120 || rc=$?
    [[ "$rc" -eq 0 ]] || return 1
    return 0
  fi
  echo "==> no base graphics set on any search path: regression_* would hang, not fail"
  echo "==> ctest --test-dir $build_dir -j$JOBS --timeout 120 -E '^regression_'"
  CTEST_OUTPUT_ON_FAILURE=1 ctest --test-dir "$build_dir" -j"$JOBS" --timeout 120 \
    -E '^regression_' || rc=$?
  echo "regression_*: NOT RUN -- install a base graphics set, see $BASESET_DOC"
  [[ "$rc" -eq 0 ]] || return 1
  return "$EXIT_PARTIAL"
}

# The worse of two tier statuses, FAIL (any other non-zero) over PARTIAL over PASS: a later step's
# PARTIAL never hides an earlier step's FAIL.
worst() {
  if [[ "$1" -ne 0 && "$1" -ne "$EXIT_PARTIAL" ]]; then echo "$1"; elif [[ "$2" -ne 0 ]]; then echo "$2"; else echo "$1"; fi
}

# Finish a tier: PASS, PARTIAL ($EXIT_PARTIAL) or FAIL (anything else), with its exit status.
finish_tier() {
  local tier="$1" status="$2"
  if [[ "$status" -eq 0 ]]; then
    echo "$tier: PASS"
  elif [[ "$status" -eq "$EXIT_PARTIAL" ]]; then
    echo "$tier: PARTIAL (exit $status)"
    exit "$status"
  else
    echo "$tier: FAIL (exit $status)"
    exit "$status"
  fi
}

# The source and build files of this change, staged or not, relative to HEAD, that carry a
# research guard: a preprocessor conditional on one of the macros, or the CMake option that
# defines it. Documents and this script only name the macros, so they are not code to build.
gated_files_changed() {
  { git diff --name-only HEAD 2>/dev/null; git ls-files --others --exclude-standard; } \
    | sort -u \
    | while IFS= read -r f; do
        [[ -f "$f" ]] || continue
        case "$f" in
          src/*|cmake/*|CMakeLists.txt|*/CMakeLists.txt) ;;
          *) continue ;;
        esac
        if grep -qE "^[[:space:]]*#[[:space:]]*(if|ifdef|ifndef|elif).*($GATED_MACROS)" "$f" \
          || grep -qE 'OPTION_(RESEARCH_INSTRUMENTATION|PRISMATIC_MCP_SERVER)' "$f"; then
          echo "$f"
        fi
      done
  return 0
}

# Whether this change, staged or not, touches the panel's pages, charts or stylesheet.
panel_changed() {
  local files
  files="$(changed_files)"
  grep -qE '^tools/research/mcp-panel/(static|test)/|^tools/research/mcp-panel/input\.css$' <<< "$files"
}

# PrismaticAI or the battle tooling changed: run the working tree's bot in one short headless match
# (rule project.battle-records-are-reproducible; tools/research/ai-battle/battle.py fitness). It fails
# on a Squirrel error, an unfinished match or a collapsing fleet; it says NOT RUN without the arena.
# The changed-file list is captured before matching: `grep -q` exits at its first match, the writer
# then dies of SIGPIPE, and under pipefail the whole pipe reported "no change", so a step was skipped.
changed_files() {
  { git diff --name-only HEAD 2>/dev/null; git ls-files --others --exclude-standard; } || true
}

bot_changed() {
  local files
  files="$(changed_files)"
  grep -qE '^tools/research/(prismatic-ai|ai-battle)/' <<< "$files"
}

# The panel's own tests (rule project.panel-charts-go-through-one-layer): chart options rendered by
# ECharts, the one-door scan, the chart registry, and app.css against a fresh build of its sources.
panel_tests() {
  local dir=tools/research/mcp-panel css=""
  [[ -d "$dir/node_modules/echarts" ]] || (cd "$dir" && npm ci --no-audit --no-fund --silent)
  command -v tailwindcss >/dev/null && css=1
  echo "==> node --test $dir/test/*.test.mjs${css:+ (with the app.css freshness check)}"
  PANEL_CSS_CHECK="$css" node --test "$dir"/test/*.test.mjs
}

# The files the change tier keeps covered line by line by their own tests (rule
# project.new-code-is-covered; src/prismatic.rs since I0092). One row per file of tools/research/mcp-rhai:
#   <file>|<the test binaries whose tests run it, comma-separated>|<criterion>
# Criterion `all`: every line llvm-cov counts in the file, its tests included.
# Criterion `items:<start>;<start>;...`: only the lines of each named item, for a file the rule
# covers in part. An item runs from the first line containing <start> (e.g. `fn apply_live`) to the
# line where its braces balance again. A <start> the file no longer contains fails the step.
COVERAGE_TABLE=(
  'src/prismatic.rs|mcp-rhai|all'
  'src/battlebrain_contract.rs|battlebrain|all'
  'src/battlebrain_policy.rs|battlebrain|all'
  'src/bin/battlebrain.rs|battlebrain|all'
  'src/service_eval.rs|mcp-panel|all'
  'src/panel_decision.rs|mcp-panel|items:fn is_freight_slot_setting;fn live_expiry;fn decision_gate;fn live_world;fn apply_live;impl LiveGame for Arc<Game>'
)
# The only way a counted line may stay unrun: <file>|<item start>|<reason>. The item is found as a
# criterion item is (its first line containing <item start>, through its balanced braces, or that
# line alone when it opens none), by text and not line number, so an edit above it cannot move an
# exemption onto other lines; one that names nothing fails the step. Each is printed with its
# reason on every run of the change tier.
COVERAGE_EXEMPT=(
  'src/bin/battlebrain.rs|fn main() {|the process entry: it hands the real argv and stdin to cli() and exits with its code; every subcommand is cli(), which the tests run'
  'src/battlebrain_contract.rs|_ => unreachable!("types are closed"),|validate_record has already restricted the type to TYPES with one_of, and every type has its arm; a type added without one panics here by design'
)

# The rows of COVERAGE_TABLE whose file this change, staged or not, touches.
coverage_rows_changed() {
  local files row
  files="$(changed_files)"
  for row in "${COVERAGE_TABLE[@]}"; do
    if grep -qxF "tools/research/mcp-rhai/${row%%|*}" <<< "$files"; then printf '%s\n' "$row"; fi
  done
}

# One row's verdict from an lcov report: every counted line of the criterion run at least once,
# or inside an exempt item. Prints "<file>: <n> of <m> lines covered" and each missed line with its
# number and text. An item (of a criterion or an exemption) whose start the file lacks fails it.
coverage_check() {
  local lcov="$1" dir="$2" file="$3" criterion="$4" items="" exempt="" row
  [[ "$criterion" == items:* ]] && items="${criterion#items:}"
  for row in ${COVERAGE_EXEMPT[@]+"${COVERAGE_EXEMPT[@]}"}; do
    [[ "${row%%|*}" == "$file" ]] && { row="${row#*|}"; exempt+="${row%%|*}"$'\x1f'; }
  done
  awk -v want="/$file" -v file="$file" -v items="$items" -v exempt="$exempt" '
    FNR == NR { src[FNR] = $0; nsrc = FNR; next }
    # A file a binary includes by #[path = "../x.rs"] is reported as src/bin/../x.rs: normalised first.
    /^SF:/ { p = substr($0, 4); while (sub(/\/[^\/.][^\/]*\/\.\.\//, "/", p)) {}
             f = (length(p) >= length(want) && substr(p, length(p) - length(want) + 1) == want); next }
    /^end_of_record/ { f = 0; next }
    f && /^DA:/ { split(substr($0, 4), a, ","); seen[a[1] + 0] = 1; hits[a[1] + 0] += a[2] }
    function trim(s) { sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s); return s }
    function count(s, c,   n, i) { n = 0; for (i = 1; i <= length(s); i++) if (substr(s, i, 1) == c) n++; return n }
    # Mark the item that starts at the first line containing `start`: through the line where its
    # braces balance, or that line alone when it opens no brace. 0 when the file has no such line.
    function mark(start, kind,   s, l, depth) {
      s = 0
      for (l = 1; l <= nsrc; l++) if (index(src[l], start) > 0) { s = l; break }
      if (!s) return 0
      if (count(src[s], "{") == 0) { marked[kind, s] = 1; return 1 }
      depth = 0
      for (l = s; l <= nsrc; l++) {
        marked[kind, l] = 1; depth += count(src[l], "{") - count(src[l], "}")
        if (depth <= 0) break
      }
      return 1
    }
    END {
      bad = 0
      if (items == "") { for (l = 1; l <= nsrc; l++) marked["in", l] = 1 }
      k = split(items, it, ";")
      for (i = 1; i <= k; i++) if (!mark(it[i], "in")) { printf "%s: the criterion names \"%s\", which the file does not contain\n", file, it[i]; bad = 1 }
      k = split(exempt, ex, "\037")
      for (i = 1; i <= k; i++) if (ex[i] != "" && !mark(ex[i], "ex")) { printf "%s: the exemption \"%s\" names nothing in the file: remove it\n", file, ex[i]; bad = 1 }
      total = 0; missed = 0; excused = 0
      for (l = 1; l <= nsrc; l++) {
        if (!(l in seen) || !(("in", l) in marked)) continue
        total++
        if (hits[l] > 0) continue
        if (("ex", l) in marked) { excused++; continue }
        missed++; lines = lines sprintf("\n    %s:%d  %s", file, l, trim(src[l]))
      }
      printf "%s: %d of %d lines covered%s%s\n", file, total - missed - excused, total,
             (excused ? sprintf(", %d exempt", excused) : ""), (missed ? sprintf(", %d run by no test:%s", missed, lines) : "")
      if (total == 0) printf "%s: no counted line: its test binaries do not build it, or the criterion selects nothing\n", file
      exit (bad || total == 0 || missed > 0)
    }' "$dir/$file" "$lcov"
}

# New code is covered (rule project.new-code-is-covered): for every COVERAGE_TABLE file this change
# touches, cargo llvm-cov runs the tests of the binaries that build it, and each counted line of its
# criterion must be run by a test or be exempt. Built without the `store` feature: no file of the
# table has a line it gates, and the step then needs no system libduckdb. Without cargo-llvm-cov the
# step is NOT RUN and the tier PARTIAL, never PASS.
new_code_coverage() {
  local dir=tools/research/mcp-rhai rows row bins="" b lcov status=0 rest file criterion
  local -a binargs=()
  echo "==> new code is covered (rule project.new-code-is-covered): ${#COVERAGE_TABLE[@]} file(s) in the table, ${#COVERAGE_EXEMPT[@]} exemption(s)"
  for row in ${COVERAGE_EXEMPT[@]+"${COVERAGE_EXEMPT[@]}"}; do
    rest="${row#*|}"
    printf '    exempt %s  "%s"\n      because %s\n' "${row%%|*}" "${rest%%|*}" "${rest#*|}"
  done
  rows="$(coverage_rows_changed)"
  if [[ -z "$rows" ]]; then
    echo "    no file of the table changed: nothing to measure"
    return 0
  fi
  if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "coverage: NOT RUN -- cargo-llvm-cov is not installed (cargo install cargo-llvm-cov)"
    return "$EXIT_PARTIAL"
  fi
  while IFS='|' read -r _ b _; do bins+="${b},"; done <<< "$rows"
  for b in $(tr ',' '\n' <<< "$bins" | sort -u); do binargs+=(--bin "$b"); done
  lcov="$(mktemp)"
  echo "==> cargo llvm-cov --release --no-default-features ${binargs[*]} ($dir)"
  if ! (cd "$dir" && cargo llvm-cov --release --no-default-features "${binargs[@]}" --lcov --output-path "$lcov" --quiet); then
    echo "coverage: FAIL -- the tests did not pass, so no line counts as covered"
    rm -f "$lcov"
    return 1
  fi
  while IFS='|' read -r file b criterion; do
    coverage_check "$lcov" "$dir" "$file" "$criterion" || status=1
  done <<< "$rows"
  rm -f "$lcov"
  return "$status"
}

# Whether this change, staged or not, touches the real-world scenario pipeline.
real_world_changed() {
  local files
  files="$(changed_files)"
  grep -qE '^tools/research/real-world/|^tools/research/schema/real-world-scenario\.' <<< "$files"
}

# The pipeline's own proof (research/real-world-scenario.md): its offline tests (determinism,
# replay, licensing, provenance, evidence rules, the schema) and a replay of the committed snapshot
# against the hashes recorded with it. Offline: nothing here uses the network.
real_world_tests() {
  local dir=tools/research/real-world
  echo "==> cargo test --offline ($dir)"
  (cd "$dir" && CARGO_TARGET_DIR="$PWD/target" cargo test --offline --quiet)
  echo "==> $dir/rw-scenario replay --expect $dir/fixtures/brno/SHA256SUMS"
  "$dir/rw-scenario" replay --snapshot "$dir/fixtures/brno/snapshot.json" --expect "$dir/fixtures/brno/SHA256SUMS" >/dev/null
}

# The AlgoGlyph submodule (I0227, ADR 0039): mcp-rhai links its crate algoglyph-kernels by path, so
# every cargo step below needs the submodule's work tree. A checkout without it gets it at the pin
# (git submodule update --init, no node_modules; the network once per worktree), then the pin record
# must be the gitlink's and list every kernel the stack needs.
algoglyph_pin() {
  local pin=tools/research/algoglyph-pin
  if [[ ! -f tools/research/vendor/algoglyph/crates/algoglyph-kernels/Cargo.toml ]]; then
    echo "==> $pin install --no-npm (the submodule is not checked out)"
    "$pin" install --no-npm
  fi
  echo "==> $pin check (the pin record is the gitlink's and lists every kernel the stack needs)"
  "$pin" check
}

# Whether this change, staged or not, touches the AlgoGlyph pin or the code that runs its kernels.
algoglyph_changed() {
  local files
  files="$(changed_files)"
  grep -qE '^\.gitmodules$|^tools/research/vendor/|^tools/research/algoglyph-pin$|^tools/research/mcp-rhai/(Cargo\.(toml|lock)|src/(algoglyph_native|hub_algoglyph|hub)\.rs)$' <<< "$files"
}

# The native kernels and the hub's routing (src/algoglyph_native.rs, src/hub_algoglyph.rs), and a
# native-rust entry in Battlebrain's catalogue; built like the boundaries below (no store feature).
algoglyph_tests() {
  echo "==> cargo test --bin mcp-rhai --bin mcp-hub algoglyph, --bin battlebrain native_rust (I0227: the pinned crate in-process)"
  RUSTFLAGS="${RUSTFLAGS:--C target-cpu=native}" cargo test --quiet --release --offline --no-default-features \
    --manifest-path tools/research/mcp-rhai/Cargo.toml --bin mcp-rhai --bin mcp-hub algoglyph
  RUSTFLAGS="${RUSTFLAGS:--C target-cpu=native}" cargo test --quiet --release --offline --no-default-features \
    --manifest-path tools/research/mcp-rhai/Cargo.toml --bin battlebrain native_rust
}

# The architecture boundaries around the one decision authority (rule
# project.decision-authority-boundaries, ADR 0037): B1-B8 as source scans and contract tests,
# compiled into battlebrain's tests (tools/research/mcp-rhai/src/authority_boundaries.rs). Offline,
# and built like the post-mortem check (release, no store feature, the same RUSTFLAGS), so the two
# share one set of dependencies.
authority_boundaries() {
  echo "==> cargo test --bin battlebrain (rule project.decision-authority-boundaries: one decision writer, B1-B8)"
  RUSTFLAGS="${RUSTFLAGS:--C target-cpu=native}" cargo test --quiet --release --offline \
    --no-default-features \
    --manifest-path tools/research/mcp-rhai/Cargo.toml --bin battlebrain
}

# B8 of that rule in the sources a default build compiles: every line of code that names research
# (RESEARCH_WORDS) sits inside a research guard (GATED_MACROS), so no default-build path reaches a
# research symbol or string, a STR_PRISMATIC_* among them (strgen cannot leave a string out of
# english.txt, so it is the reference that must be guarded). Comments may name research freely.
# src/prismatic_mcp/ is left out because CMake compiles none of it unless
# OPTION_PRISMATIC_MCP_SERVER is ON, which is checked here as well.
research_code_is_guarded() {
  local list=src/prismatic_mcp/CMakeLists.txt hits
  echo "==> every source line naming research is inside a research guard (rule project.research-code-is-gated-and-labelled)"
  if [[ "$(grep -c 'add_files(' "$list")" -ne "$(grep -c 'CONDITION OPTION_PRISMATIC_MCP_SERVER' "$list")" ]]; then
    echo "error: $list adds files without 'CONDITION OPTION_PRISMATIC_MCP_SERVER'" >&2
    return 1
  fi
  hits="$(git ls-files -z -- 'src/*.cpp' 'src/*.h' 'src/*.hpp' ':!src/prismatic_mcp/' ':!src/3rdparty/' \
    | xargs -0 awk -v words="$RESEARCH_WORDS" -v macros="$GATED_MACROS" '
      # The code of a line: comments removed, a block comment carried over from the line before.
      function code_of(line,    out, i, j) {
        out = ""
        while (line != "") {
          if (in_comment) {
            i = index(line, "*/")
            if (i == 0) return out
            line = substr(line, i + 2); in_comment = 0
          } else {
            i = index(line, "/*"); j = index(line, "//")
            if (j > 0 && (i == 0 || j < i)) return out substr(line, 1, j - 1)
            if (i == 0) return out line
            out = out substr(line, 1, i - 1); line = substr(line, i + 2); in_comment = 1
          }
        }
        return out
      }
      # 1 when the condition holds only with a research macro defined, 0 otherwise.
      function positive(cond) { return cond ~ macros && cond !~ "!" }
      FNR == 1 { depth = 0; in_comment = 0 }
      {
        code = code_of($0)
        if (code ~ /^[ \t]*#[ \t]*if(def)?[ \t(!]/) {
          depth++; gated[depth] = positive(code); other[depth] = (code ~ macros && !gated[depth])
          next
        }
        if (code ~ /^[ \t]*#[ \t]*ifndef[ \t]/) {
          depth++; gated[depth] = 0; other[depth] = (code ~ macros)
          next
        }
        if (code ~ /^[ \t]*#[ \t]*elif/) { gated[depth] = positive(code); other[depth] = 0; next }
        if (code ~ /^[ \t]*#[ \t]*else/) { gated[depth] = other[depth]; other[depth] = 0; next }
        if (code ~ /^[ \t]*#[ \t]*endif/) { if (depth > 0) depth--; next }
        if (tolower(code) !~ words) next
        for (k = 1; k <= depth; k++) if (gated[k]) next
        printf "%s:%d: %s\n", FILENAME, FNR, $0
      }')"
  if [[ -n "$hits" ]]; then
    echo "error: research-only code outside a research guard ($GATED_MACROS):" >&2
    printf '%s\n' "$hits" >&2
    return 1
  fi
}

# B8 of that rule on the binary itself: a default build carries no research code. Checked on the
# build this tier just made when its cache says both research options are OFF; otherwise NOT RUN,
# so PARTIAL, never PASS. Every string of the binary naming research (RESEARCH_WORDS) fails it: a
# symbol, a debug-info name, a script-API constant, a console command or a label. Left out: the
# paths of this checkout and of the build directory and the version string, which carry the branch
# name (a worktree or branch may be called research/...), and a string that is only a StringID's
# name (^STR_...$): GCC's debug info names every constant of the generated table/strings.h, used
# or not (seen on lundra, 2026-11-22), and strgen cannot leave a string of english.txt out of a
# build. That a STR_PRISMATIC_* is referenced only under a guard is research_code_is_guarded's job.
default_build_free_of_research() {
  local dir="$1" bin="$1/openttd" cache="$1/CMakeCache.txt" hits rev
  local -a known
  if [[ ! -f "$cache" ]]; then
    echo "default build free of research code: NOT RUN -- $dir is not configured"
    return "$EXIT_PARTIAL"
  fi
  if ! grep -qE '^OPTION_RESEARCH_INSTRUMENTATION:BOOL=OFF$' "$cache" \
    || ! grep -qE '^OPTION_PRISMATIC_MCP_SERVER:BOOL=OFF$' "$cache"; then
    echo "default build free of research code: NOT RUN -- $dir is configured with a research option ON"
    return "$EXIT_PARTIAL"
  fi
  [[ -f "$bin" ]] || bin="$dir/openttd.exe"
  if [[ ! -f "$bin" ]]; then
    echo "default build free of research code: NOT RUN -- no $dir/openttd binary"
    return "$EXIT_PARTIAL"
  fi
  known=(-e "$ROOT_DIR" -e "$(cd "$ROOT_DIR" && pwd -P)" -e "$(cd "$dir" && pwd -P)")
  rev="$(sed -n 's/^const std::string _openttd_revision = "\(.*\)";$/\1/p' "$dir/generated/rev.cpp" 2>/dev/null || true)"
  [[ -z "$rev" ]] || known+=(-e "$rev")
  echo "==> strings $bin: a default build must carry no string naming research (rule project.decision-authority-boundaries, B8)"
  # Each grep reads all of its input: grep -q would stop early and strings would die of SIGPIPE.
  hits="$(strings "$bin" | grep -iE "$RESEARCH_WORDS" | grep -vF "${known[@]}" \
    | grep -vE '^STR_[A-Z0-9_]+$' | sort | uniq -c || true)"
  if [[ -n "$hits" ]]; then
    echo "error: $bin is a default build and carries research string(s):" >&2
    printf '%s\n' "$hits" | head -n 40 >&2
    return 1
  fi
}

# Both B8 checks, on their own: the sources, then the default binary of a build directory.
cmd_research_free() {
  local dir="${1:-build}" status=0
  research_code_is_guarded
  default_build_free_of_research "$dir" || status=$?
  finish_tier research-free "$status"
}

cmd_smoke() {
  local ctest_regex=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -R) ctest_regex="${2:?-R requires a ctest regex}"; shift 2 ;;
      *) echo "smoke: unknown option: $1" >&2; usage; exit 2 ;;
    esac
  done

  diff_check

  if [[ ! -f build/CMakeCache.txt ]]; then
    echo "error: build/ is not configured yet. Run '$SELF change' or '$SELF full' first." >&2
    exit 1
  fi

  echo "==> cmake --build build -j$JOBS (incremental)"
  cmake --build build -j"$JOBS"

  if [[ -n "$ctest_regex" ]]; then
    echo "==> ctest --test-dir build -R '$ctest_regex' --output-on-failure"
    ctest --test-dir build -R "$ctest_regex" --output-on-failure
  fi

  echo "smoke: PASS"
}

cmd_change() {
  local status=0 gated f
  diff_check

  echo "==> tools/check-prismatic-docs.sh"
  tools/check-prismatic-docs.sh
  echo "==> tools/surfaces/surfaces check"
  tools/surfaces/surfaces check
  echo "==> tools/research/readme-scoreboard --check (README scoreboard derived from research/results)"
  tools/research/readme-scoreboard --check
  echo "==> tools/research/ai-battle/battle.py check (rule project.battle-records-are-reproducible)"
  python3 -B tools/research/ai-battle/battle.py check
  echo "==> tools/research/town-model/derive.py --check (the town-authority model is what src/ derives)"
  python3 -B tools/research/town-model/derive.py --check
  algoglyph_pin
  # I0228: the AlgoGlyph kernel contracts are what the pinned AlgoGlyph source derives. Without an
  # AlgoGlyph checkout ($ALGOGLYPH_ROOT, its submodule) the tool prints NOT RUN and exits 3: PARTIAL.
  echo "==> algoglyph-contracts --check (research/battlebrain/README.md#derived-kernel-contracts-i0228)"
  rc=0
  RUSTFLAGS="${RUSTFLAGS:--C target-cpu=native}" cargo run --quiet --release --offline \
    --no-default-features \
    --manifest-path tools/research/mcp-rhai/Cargo.toml --bin algoglyph-contracts -- --check || rc=$?
  case "$rc" in
    0) ;;
    "$EXIT_PARTIAL") status="$EXIT_PARTIAL" ;;
    *) exit "$rc" ;;
  esac
  if compgen -G 'research/results/postmortem-*.json' > /dev/null; then
    echo "==> battle-postmortem check (research/battle-postmortem.md: every post-mortem .md is what its .json derives)"
    # RUSTFLAGS replaces a target's rustflags from a global cargo config; a `-C lto` there breaks
    # build scripts (research/live-arena.md). An RUSTFLAGS already set wins.
    # `check` reads only the tracked .json/.md, never DuckDB: built without the `store` feature, so
    # the gate needs no system libduckdb (a Linux host without Homebrew's failed here, 2026-11-22).
    RUSTFLAGS="${RUSTFLAGS:--C target-cpu=native}" cargo run --quiet --release --offline \
      --no-default-features \
      --manifest-path tools/research/mcp-rhai/Cargo.toml --bin battle-postmortem -- check
  fi
  authority_boundaries
  if algoglyph_changed; then algoglyph_tests; fi
  research_code_is_guarded
  echo "==> tools/knowledge/refs check"
  tools/knowledge/refs check
  # The check surfaces.toml runs over research/capability-matrix.md rests on these (anchored lines).
  echo "==> python3 -B -m unittest tools/knowledge/test_refs.py (refs: paths, symbols, anchored lines)"
  python3 -B -m unittest tools/knowledge/test_refs.py
  echo "==> tools/research/e2e-coverage (rule project.web-ui-is-covered-end-to-end: every web UI page has a browser check)"
  tools/research/e2e-coverage
  echo "==> tools/knowledge/refs impact HEAD (records citing what this change touches: re-read them)"
  tools/knowledge/refs impact HEAD
  echo "==> tools/knowledge/refs coverage src/ (tracked files no subsystem map attaches to; informational)"
  tools/knowledge/refs coverage src/ | tail -n 1
  # Every test_*.py of the repository, found by name, run under line tracing; a changed Python
  # line no test ran fails here (rule project.new-code-tested). C++/Rust/JS are named, not judged.
  echo "==> tools/coverage/changed-lines (rule project.new-code-tested: every changed Python line runs under a test)"
  python3 -B tools/coverage/changed-lines
  if panel_changed; then panel_tests; fi
  if bot_changed; then
    echo "==> tools/research/ai-battle/battle.py fitness (PrismaticAI in one short headless match)"
    python3 -B tools/research/ai-battle/battle.py fitness
  fi
  if real_world_changed; then real_world_tests; fi
  new_code_coverage || status=$(worst "$status" $?)

  if [[ ! -f build/CMakeCache.txt ]]; then
    echo "==> cmake -S . -B build -DCMAKE_BUILD_TYPE=RelWithDebInfo ${GATE_CMAKE_EXTRA[*]} (initial configure)"
    cmake -S . -B build -DCMAKE_BUILD_TYPE=RelWithDebInfo ${GATE_CMAKE_EXTRA[@]+"${GATE_CMAKE_EXTRA[@]}"}
  fi

  echo "==> cmake --build build -j$JOBS"
  cmake --build build -j"$JOBS"

  run_ctest build || status=$(worst "$status" $?)
  # A failure outranks a PARTIAL: neither may hide the other's worse result (`worst`).
  default_build_free_of_research build || status=$(worst "$status" $?)

  echo "==> git status --porcelain (review below for unwanted generated files)"
  git status --porcelain

  gated="$(gated_files_changed)"
  if [[ -n "$gated" ]]; then
    echo "==> this change touches research-gated code, which this build does not compile:"
    while IFS= read -r f; do printf '      %s\n' "$f"; done <<< "$gated"
    echo "    run '$SELF research' before calling it done"
  fi

  finish_tier change "$status"
}

cmd_research() {
  local status=0 mcp=OFF bin
  local -a opts=(-DCMAKE_BUILD_TYPE=RelWithDebInfo -DOPTION_RESEARCH_INSTRUMENTATION=ON)

  diff_check

  if grep -q 'OPTION_PRISMATIC_MCP_SERVER' cmake/Options.cmake; then
    mcp=ON
    opts+=(-DOPTION_PRISMATIC_MCP_SERVER=ON)
  fi

  # Always (re)configure: cmake is cheap when nothing changed, and an options set that drifted
  # in the cache must not pass for the research configuration.
  opts+=(${GATE_CMAKE_EXTRA[@]+"${GATE_CMAKE_EXTRA[@]}"})
  echo "==> cmake -S . -B $RESEARCH_BUILD_DIR ${opts[*]}"
  cmake -S . -B "$RESEARCH_BUILD_DIR" "${opts[@]}"

  echo "==> cmake --build $RESEARCH_BUILD_DIR -j$JOBS"
  cmake --build "$RESEARCH_BUILD_DIR" -j"$JOBS"

  bin="$RESEARCH_BUILD_DIR/openttd"
  echo "==> research_status is compiled into $bin"
  # grep -c reads all of its input: grep -q would stop early, strings would die of SIGPIPE, and
  # pipefail would turn a match into a failure.
  if [[ "$(strings "$bin" | grep -c '^research_status')" -eq 0 ]]; then
    echo "error: $bin carries no research_status: the research option did not take effect" >&2
    exit 1
  fi

  run_ctest "$RESEARCH_BUILD_DIR" || status=$?

  if [[ "$mcp" == ON ]]; then
    # From the build directory, as ctest runs it: a case that finds its fixture next to its source
    # (__FILE__, which the build names relative to the build directory) finds it from there only.
    echo "==> (cd $RESEARCH_BUILD_DIR && ./openttd_test '[prismatic_mcp]')"
    (cd "$RESEARCH_BUILD_DIR" && ./openttd_test '[prismatic_mcp]') || status=$(worst "$status" 1)
    window_check "$bin" || status=$(worst "$status" $?)
  fi

  finish_tier research "$status"
}

# The Prismatic control window end to end (ADR 0041), in a game of its own in the null video
# driver: 0 PASS, 1 a check failed, NOT RUN (PARTIAL) without node or without a base set.
window_check() {
  local bin="$1" rc=0
  if ! command -v node >/dev/null 2>&1; then
    echo "NOT RUN tools/research/mcp-control-window-check: no node"
    return "$EXIT_PARTIAL"
  fi
  echo "==> tools/research/mcp-control-window-check --bin $bin"
  node tools/research/mcp-control-window-check --bin "$bin" || rc=$?
  if [[ "$rc" -eq 3 ]]; then return "$EXIT_PARTIAL"; fi
  return "$rc"
}

# The Rust performance benchmarks against the newest committed record of this host
# (research/rust-bench.md). 0 PASS, 1 a regression or a failed run, 3 NOT RUN (no record of this
# host, no cargo, dependencies neither cached nor downloadable), which is PARTIAL here. Only the full tier runs it: change must stay fast.
rust_bench_check() {
  local rc=0
  echo "==> tools/research/rust-bench check (median regression above 25% and the baseline's spread, against this host's newest record)"
  python3 -B tools/research/rust-bench check || rc=$?
  case "$rc" in
    0) return 0 ;;
    3) return "$EXIT_PARTIAL" ;;
    *) return 1 ;;
  esac
}

cmd_full() {
  local build_dir="build-fullgate" status=0 rc

  diff_check

  echo "==> rm -rf $build_dir (clean room)"
  rm -rf "$build_dir"

  echo "==> cmake -S . -B $build_dir -DCMAKE_BUILD_TYPE=RelWithDebInfo ${GATE_CMAKE_EXTRA[*]}"
  cmake -S . -B "$build_dir" -DCMAKE_BUILD_TYPE=RelWithDebInfo ${GATE_CMAKE_EXTRA[@]+"${GATE_CMAKE_EXTRA[@]}"}

  echo "==> cmake --build $build_dir -j$JOBS"
  cmake --build "$build_dir" -j"$JOBS"

  run_ctest "$build_dir" || status=$?
  # A failure outranks a PARTIAL: neither may hide the other's worse result.
  rust_bench_check || { rc=$?; [[ "$status" -eq 1 ]] || status=$rc; }

  finish_tier full "$status"
}

[[ $# -ge 1 ]] || { usage; exit 2; }

case "$1" in
  -h|--help) usage; exit 0 ;;
  smoke) shift; cmd_smoke "$@" ;;
  change) shift; cmd_change "$@" ;;
  research) shift; cmd_research "$@" ;;
  full) shift; cmd_full "$@" ;;
  research-free) shift; cmd_research_free "$@" ;;
  *) echo "unknown command: $1" >&2; usage; exit 2 ;;
esac
