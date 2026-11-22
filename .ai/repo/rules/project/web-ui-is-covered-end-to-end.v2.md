---
id: project.web-ui-is-covered-end-to-end
version: 2
kind: rule
title: Every control of every web UI page is driven end to end in a real browser, and none fails silently
description: Every page of the panel and the hub, as the one shell declares them, and every control on them, is driven in Chrome against a running stack. The pages come from the shell. The controls are inferred from the live DOM by the generic crawl (tools/research/mcp-ui-crawl-check). Each control must do something visible, and a failure it causes must be shown. Every page script takes $, esc and the API client from the shell's one kit, never a copy. tools/research/e2e-coverage and the panel's node tests fail the gate on an undriven page, a duplicate helper or a page that calls fetch() past the kit.
statement: Drive every page and every control of the web UI in a real browser against a live stack (the generic crawl plus the page checks), let no control be dead or fail silently, and give every page script the shell's one kit ($, esc, api, notify) instead of a copy; a page, control or flow no browser check drives is treated as not built.
status: active
class: blocking
depends_on: [project.panel-items-are-routable-pages@1, project.never-report-unrun-validation-as-pass@1]
tags: [panel, hub, ui, testing, playwright, e2e, derivation]
---

# Rationale

Version 1 (owner, 2026-11-18): "everything in the web UI must from now on always be covered by
Playwright end-to-end tests, and this must be a rule, always, enforced now". It covered **pages**:
each one opened, showed its content and threw no error.

That was not enough. On 2026-11-22 the owner found "spousta prvku zde nefunguje, namatkou treba
approve" (many elements here do not work, Approve for one), and "web ui musi mit 100% test
coverage pres playwright + derived infered generic dynamic playwright tests, toto musi byt
enforced, stavajici debt odstranen a novemu byt zabraneno, zadne duplicitni implementace" (the
web UI must have 100 % coverage through Playwright, with derived, inferred, generic, dynamic tests,
enforced; the existing debt removed and new debt prevented; no duplicate implementations).

Every page check passed while every button of a page opened as `localhost` was dead. The server
refused their POSTs with 403. A second, narrower copy of the origin check caused the refusal, and
each page's own copy of `api()` swallowed it (ADR 0038).

# Required behaviour

- **Pages are derived.** They are every page of every app in `static/shell.js` `APPS`. A new page
  joins the rule when it joins the registry.
- **Controls are inferred, not listed.** `tools/research/mcp-ui-crawl-check` (declared as
  `e2e-covers: *`) takes every visible control from the live DOM of every page, and takes them
  again after every action. A control that only appears after another one (a row, a dialog, a
  tab) is driven too. Repeated controls (one per approval, one per company) share a pattern, and
  one of them is driven for all.
- **Every control does something visible.** It may send a request, change the page, navigate,
  move focus or change ARIA state, or show a toast. Anything else is DEAD. A link to the server
  answers below 400.
- **No silent failure.** A 4xx/5xx answer or a failed request that an action causes must be shown
  to the person: a toast from the shell's `notify`, or the status written into the page.
  Otherwise it is SILENT. A page error always fails.
- **One kit, no copies.** Every page script takes `$`, `esc`, `api` and `notify` from the shell:
  `globalThis.PanelKit` in a classic script, `components.js` in a module. `api()` is the only way
  a page talks to its server. It shows every non-2xx answer, so no control can fail silently by
  construction. A page script that defines its own copy fails `test/one-kit.test.mjs`. So does a
  `fetch(` outside the kit, except a stream the kit cannot carry (named in the test, with its
  reason).
- **Page checks still prove content.** The checks declaring pages (`e2e-covers: panel /x`) keep
  proving each page's own content, its shell, no page error and no sideways scroll at 390 px. A
  flow a page offers is driven through its controls, not only its API.
- **Enforced.**
  - `tools/research/e2e-coverage` fails on a shell page no browser check declares, a declaration
    in a file that drives no browser, or a declared page the shell does not have. It runs in
    `tools/gate.sh change` and in CI (`surfaces.yml`).
  - The panel's node tests (`node --test tools/research/mcp-panel/test/`) fail on a duplicate
    helper or a `fetch(` past the kit. They run in the gate and in CI.
- **Run, not only declared.** A change to a page reruns the crawl and the checks that cover it
  against a live, disposable stack:
  `tools/research/e2e-coverage --run --panel <url> --hub <url>`. The report names each run and
  its result (rule `project.never-report-unrun-validation-as-pass`). A check that could not run
  is NOT RUN, never PASS. The crawl presses everything, so it never runs against a game someone
  is playing.

# Checks today

`tools/research/e2e-coverage` prints the live table of pages and the checks that cover them; the
crawl's `--json` report lists every control pattern and its verdict.

# Failure behaviour

Any of these fails the gate and blocks the change:

- a page without a declared, browser-driving check;
- a helper copied into a page script;
- a `fetch(` past the kit.

A crawl verdict of DEAD, SILENT or FAIL is a defect of the page, to be fixed, not excused. A
report that claims a page or control works without naming the crawl or check run against a live
stack is a validation claim without evidence.

# Related

Rules `project.panel-items-are-routable-pages`, `project.never-report-unrun-validation-as-pass` and
`project.extend-and-integrate-before-adding`; ADR 0038; `research/mcp-control.md`.
