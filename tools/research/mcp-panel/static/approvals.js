/* Approvals (the home's #approvals): every request the game server's approval core holds for the
   owner, decided fast. Each row has its Yes and No at the same place (the column after the
   checkbox), so the owner clicks down the queue without moving the mouse; a decided row leaves at
   once, the next one takes its place and its focus. Checkboxes pick rows (the header one takes every
   row the filter shows, Shift+click a range, Space the focused row); Approve all / Deny all act on
   the picked rows, else on every row the filter shows. Filters (tool, company, search, expired) live
   in the address (a_*). "Always..." opens a rule editor prefilled from the row: a standing rule of
   the core, as Claude's always allow / always deny, listed below with its hits and a Remove.

   Every decision goes to the panel's owner routes (panel.rs: POST /api/approve, /api/deny,
   /api/approve/many, /api/deny/many, /api/rules, /api/rules/remove), which need the owner's sign-in
   (the shell's api() asks for it); the core decides, the page only asks and shows (ADR 0038). An
   owner-only tool (openttd.ai.set_settings) is decided at the game's console alone: its row says so
   and offers no Yes and no allow rule. Pure parts first (test/approvals.test.mjs), the page after. */

import { UI, TONE, $, esc, api, notify, patchAddress } from './components.js';

/* ---- pure ----------------------------------------------------------------------------------- */

/** The address keys of the filter. */
const KEYS = { tool: 'a_tool', company: 'a_company', q: 'a_q', expired: 'a_expired' };

/** The filter an address holds: tool, company, search text, and whether expired rows show. */
export function filterOf(search = '') {
  const p = new URLSearchParams(search);
  return { tool: p.get(KEYS.tool) || '', company: p.get(KEYS.company) || '', q: p.get(KEYS.q) || '', expired: p.get(KEYS.expired) === 'show' };
}

/** The address patch of a filter (empty values drop their key). */
export const filterPatch = (f) => ({ tool: f.tool, company: f.company, q: f.q, expired: f.expired ? 'show' : '' });

/** The company an approval's arguments name, as text ('' when none). */
export function companyOf(args) {
  const c = args && typeof args === 'object' ? args.company : undefined;
  return typeof c === 'number' || typeof c === 'string' ? String(c) : '';
}

/** The rows the list shows: the approvals still waiting (expired ones only when asked for), that
 *  the filter takes, in the server's order. */
export function rowsOf(approvals, f) {
  const q = f.q.trim().toLowerCase();
  return (approvals || []).filter((a) => a.state === 'pending')
    .filter((a) => f.expired || !a.expired)
    .filter((a) => !f.tool || a.tool === f.tool)
    .filter((a) => !f.company || companyOf(a.args) === f.company)
    .filter((a) => !q || `#${a.id} ${a.tool} ${JSON.stringify(a.args)}`.toLowerCase().includes(q));
}

/** Whether the panel may decide a row: waiting, not expired, not an owner-only tool. */
export const decidable = (a) => a.state === 'pending' && !a.expired && !a.owner_only;

/** Every value of a facet among the waiting approvals, with its count, most frequent first. */
export function facet(approvals, of) {
  const n = new Map();
  for (const a of approvals || []) if (a.state === 'pending') { const v = of(a); if (v !== '') n.set(v, (n.get(v) || 0) + 1); }
  return [...n].sort((x, y) => y[1] - x[1] || String(x[0]).localeCompare(String(y[0])));
}

/** The ids a bulk decision takes: the picked rows the filter shows, else every row it shows; only
 *  the ones the panel may decide. */
export function bulkIds(rows, picked) {
  const ok = rows.filter(decidable);
  const chosen = ok.filter((a) => picked.has(a.id));
  return (picked.size ? chosen : ok).map((a) => a.id);
}

/** The ids from `from` to `to` (inclusive, either order) in the shown order: a Shift+click range. */
export function rangeIds(ids, from, to) {
  const [i, j] = [ids.indexOf(from), ids.indexOf(to)];
  if (i < 0 || j < 0) return [to];
  return ids.slice(Math.min(i, j), Math.max(i, j) + 1);
}

/** The header checkbox's state over the decidable rows shown: all, some (indeterminate) or none. */
export function headerState(rows, picked) {
  const ids = rows.filter(decidable).map((a) => a.id);
  const n = ids.filter((id) => picked.has(id)).length;
  return { checked: n > 0 && n === ids.length, indeterminate: n > 0 && n < ids.length, disabled: !ids.length };
}

/** The labels of the bulk buttons: "selected" while rows are picked, "all" over the filter else. */
export function bulkLabels(n, picked) {
  const what = picked ? 'selected' : 'all';
  return { approve: `Approve ${what} (${n})`, deny: `Deny ${what} (${n})` };
}

/** The matcher a rule editor starts from: the row's own arguments (an exact match; widen a value
 *  to "*" or a glob), or none for a call without arguments. */
export function matcherOf(args) {
  return args && typeof args === 'object' && !Array.isArray(args) && Object.keys(args).length ? args : null;
}

/** A rule as a person reads it: "always allow openttd.cmd.* where {...}". */
export function ruleText(r) {
  const m = r.match && Object.keys(r.match).length ? ` where ${JSON.stringify(r.match)}` : '';
  return `always ${r.effect} ${r.tool}${m}`;
}

/** The body of POST /api/rules from the editor's fields; Err text for a matcher that is not a JSON
 *  object. */
export function ruleBody({ effect, tool, matcher, useMatcher, persist }) {
  const body = { effect, tool: tool.trim(), persist: !!persist };
  if (useMatcher && matcher.trim()) {
    let m;
    try { m = JSON.parse(matcher); } catch (e) { return { error: `the matcher is not JSON: ${e.message}` }; }
    if (!m || typeof m !== 'object' || Array.isArray(m)) return { error: 'the matcher is a JSON object: {"name": "max_*"}' };
    body.match = m;
  }
  return { body };
}

/** The approval core's glob (prismatic_mcp_rules.h Glob): `*` any run, `?` one character, a
 *  backslash makes the next character itself. */
export function coreGlob(pattern, text) {
  const p = [...pattern]; const t = [...text];
  let pi = 0; let ti = 0; let star = -1; let mark = 0;
  while (ti < t.length) {
    if (p[pi] === '*') { star = pi++; mark = ti; continue; }
    if (p[pi] === '?') { pi++; ti++; continue; }
    if (pi < p.length) {
      const lit = p[pi] === '\\' && pi + 1 < p.length ? pi + 1 : pi;
      if (p[lit] === t[ti]) { pi = lit + 1; ti++; continue; }
    }
    if (star < 0) return false;
    pi = star + 1; mark++; ti = mark;
  }
  while (p[pi] === '*') pi++;
  return pi === p.length;
}

/** Whether a glob names more than one tool (an unescaped `*` or `?`). */
export const hasWildcard = (g) => /(^|[^\\])(\\\\)*[*?]/.test(g);

/** The values a JSON path ($, .name, ['name'], [n], [*], .*, ..name) selects; null when it does not parse. */
export function pathSelect(root, path) {
  if (!path.startsWith('$')) return null;
  const b = path.slice(1);
  const kids = (n) => (Array.isArray(n) ? n : n && typeof n === 'object' ? Object.values(n) : []);
  const descend = (n, key, out) => {
    if (Array.isArray(n)) n.forEach((v) => descend(v, key, out));
    else if (n && typeof n === 'object') for (const [k, v] of Object.entries(n)) { if (k === key) out.push(v); descend(v, key, out); }
  };
  const end = (from) => { const m = b.slice(from).search(/[.[]/); return m < 0 ? b.length : from + m; };
  let at = [root]; let i = 0;
  while (i < b.length) {
    const next = [];
    if (b.startsWith('..', i)) {
      const e = end(i + 2); if (e === i + 2) return null;
      at.forEach((n) => descend(n, b.slice(i + 2, e), next)); i = e;
    } else if (b[i] === '.') {
      const e = end(i + 1); if (e === i + 1) return null;
      const name = b.slice(i + 1, e);
      at.forEach((n) => { if (name === '*') next.push(...kids(n)); else if (n && typeof n === 'object' && !Array.isArray(n) && name in n) next.push(n[name]); });
      i = e;
    } else if (b[i] === '[') {
      const close = b.indexOf(']', i); if (close < 0) return null;
      const inner = b.slice(i + 1, close);
      const quoted = inner.length >= 2 && (inner[0] === "'" || inner[0] === '"') && inner.at(-1) === inner[0];
      if (!quoted && inner !== '*' && !/^\d{1,9}$/.test(inner)) return null;
      at.forEach((n) => {
        if (inner === '*') next.push(...kids(n));
        else if (quoted) { const k = inner.slice(1, -1); if (n && typeof n === 'object' && !Array.isArray(n) && k in n) next.push(n[k]); }
        else if (Array.isArray(n) && Number(inner) < n.length) next.push(n[Number(inner)]);
      });
      i = close + 1;
    } else return null;
    at = next;
  }
  return at;
}

/** Whether `value` matches the JSON matcher `pattern`, as the approval core decides it
 *  (prismatic_mcp_rules.h Match; panel.rs core_match is the same). */
export function coreMatch(pattern, value) {
  if (pattern && typeof pattern === 'object' && !Array.isArray(pattern)) {
    return Object.entries(pattern).every(([k, p]) => {
      if (k === '$in') return Array.isArray(p) && p.some((e) => coreMatch(e, value));
      if (k === '$ne') return !coreMatch(p, value);
      if (['$lt', '$lte', '$gt', '$gte'].includes(k)) {
        if (typeof p !== 'number' || typeof value !== 'number') return false;
        return k === '$lt' ? value < p : k === '$lte' ? value <= p : k === '$gt' ? value > p : value >= p;
      }
      if (k.startsWith('$')) { const f = pathSelect(value, k); return !!f && f.length > 0 && f.every((v) => coreMatch(p, v)); }
      return !!value && typeof value === 'object' && !Array.isArray(value) && k in value && coreMatch(p, value[k]);
    });
  }
  if (typeof pattern === 'string') return pattern === '*' || coreGlob(pattern, typeof value === 'string' ? value : JSON.stringify(value));
  if (Array.isArray(pattern)) return Array.isArray(value) && value.length === pattern.length && pattern.every((p, i) => coreMatch(p, value[i]));
  return typeof value === 'string' ? JSON.stringify(pattern) === value : JSON.stringify(pattern) === JSON.stringify(value);
}

/** Whether a matcher constrains no value (panel.rs broad_matcher, the core's IsBroadMatcher): null,
 *  {}, or an object whose every entry, under any key or path, is a non-empty string of `*` only, an
 *  operator object whose only operator is $ne, or a nested broad object. Key presence never narrows. */
export function broad(m) {
  if (m === null || m === undefined) return true;
  if (typeof m !== 'object' || Array.isArray(m)) return false;
  return Object.entries(m).every(([k, v]) => {
    if (k === '$ne') return true;
    if (['$in', '$lt', '$lte', '$gt', '$gte'].includes(k)) return false;
    if (typeof v === 'string') return /^\*+$/.test(v);
    return !!v && typeof v === 'object' && !Array.isArray(v) && broad(v);
  });
}
/** An MCP tool's action type, the other name a glob matches (openttd.cmd.X -> command.X). */
export const actionOf = (tool) => String(tool).replace(/^openttd\.cmd\./, 'command.').replace(/^openttd\./, '');

/** What a rule would do now: the ids of the waiting, unexpired approvals it would decide (an allow
 *  rule never an owner-only one), or why it cannot be made (a matcher that is not JSON, a broad
 *  allow rule over several tools, which the core refuses). */
export function dryRun(approvals, { effect, tool, matcher, useMatcher }) {
  const glob = tool.trim();
  let m = null;
  if (useMatcher && matcher.trim()) {
    try { m = JSON.parse(matcher); } catch (e) { return { error: 'the matcher is not JSON yet' }; }
  }
  if (!glob) return { error: 'name a tool or a glob' };
  if (effect === 'allow' && hasWildcard(glob) && broad(m)) return { error: `too broad: an allow rule for every tool of ${glob} with any arguments is refused; narrow the matcher or name one tool` };
  const ids = (approvals || []).filter((a) => a.state === 'pending' && !a.expired && !(effect === 'allow' && a.owner_only))
    .filter((a) => (coreGlob(glob, a.tool) || coreGlob(glob, actionOf(a.tool))) && (m === null || coreMatch(m, a.args)))
    .map((a) => a.id);
  return { ids };
}

/** The dry run as a line. */
export const dryRunText = (d) => (d.error ? d.error : `Would decide ${d.ids.length} waiting approval${d.ids.length === 1 ? '' : 's'} now${d.ids.length ? `: ${d.ids.map((id) => `#${id}`).join(', ')}` : ''}`);

/** The keys that decide (or pick); a held one never repeats them. */
export const DECISIVE = new Set(['y', 'a', 'n', 'd', 'Enter', ' ']);
/** How long after a decision, or after the rows moved under the pointer, a key or click is not taken. */
export const GUARD_MS = 400;
/** How long a row a refresh moved (re-ordered, or a row inserted above it) takes no click. */
export const ROW_GUARD_MS = 600;

/** The rows a refresh moved under a still pointer: each row that is new, or that has above it a row
 *  that was not above it before (a re-order, a row inserted above). A row only moving up because a
 *  row above it left is not moved. None on the first drawing. */
export function movedRows(prev, ids) {
  if (!prev) return [];
  const pos = new Map(prev.map((id, i) => [id, i]));
  const moved = [];
  ids.forEach((id, i) => {
    if (!pos.has(id)) { moved.push(id); return; }
    const before = new Set(prev.slice(0, pos.get(id)));
    if (ids.slice(0, i).some((a) => !before.has(a))) moved.push(id);
  });
  return moved;
}

/** One failed id of a bulk answer as a person reads it. */
export const failureText = (r) => `#${r.id}: ${r.code} ${(r.answer && (r.answer.error || r.answer.status)) || ''}`.trim();

const DECIDE = 'w-40 px-2 py-2';
const BIG_YES = UI.primary.replace('px-4 py-2', 'w-16 px-3 py-2.5');
const BIG_NO = UI.secondary.replace('px-4 py-2', 'w-16 px-3 py-2.5');
const SMALL = UI.secondary.replace('px-4 py-2', 'px-3 py-1.5');
const badge = (text, tone) => `<span class="${UI.badge} ${TONE[tone]} whitespace-nowrap">${esc(text)}</span>`;

/** One row: checkbox, Yes and No (or why there are none), id, tool and the rule that matched it,
 *  the exact arguments, the company, and Always... */
export function rowHtml(a, { picked = false, current = false, rules = true } = {}) {
  const ok = decidable(a);
  const decide = a.owner_only
    ? `<span class="flex flex-col gap-1">${badge('decide at the game\'s console', 'purple')}<code class="text-xs">prismatic_mcp approve ${a.id}</code></span>`
    : a.expired ? badge('expired', 'gray')
      : `<span class="flex gap-2"><button type="button" data-approve="${a.id}" data-id="${a.id}" data-role="yes" class="${BIG_YES}" aria-label="Approve #${a.id}" title="Approve #${a.id} (y, a, Enter)">Yes</button><button type="button" data-deny="${a.id}" data-id="${a.id}" data-role="no" class="${BIG_NO}" aria-label="Deny #${a.id}" title="Deny #${a.id} (n, d)">No</button></span>`;
  const m = a.matched_rule;
  // A rule that applies but does not decide (an owner-only tool, an unattended rule with no
  // registered approver for the company) holds the approval for the owner, as the game's window says.
  const rule = m ? ` ${badge(`rule #${m.id} ${m.effect}${m.decides === false ? ', held: you decide it' : ''}`, m.decides === false ? 'yellow' : m.effect === 'deny' ? 'red' : 'green')}` : '';
  return `<tr data-row="${a.id}" tabindex="${current ? 0 : -1}" aria-selected="${picked}" class="align-top bg-white border-b dark:bg-gray-800 dark:border-gray-700 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-primary-500${current ? ' bg-primary-50 dark:bg-gray-700' : ''}${a.expired ? ' opacity-60' : ''}">
      <td class="w-10 px-3 py-2"><input type="checkbox" data-select="${a.id}" data-id="${a.id}" data-role="pick" ${picked ? 'checked' : ''} ${ok ? '' : 'disabled'} class="${UI.check}" aria-label="Select #${a.id}"></td>
      <td class="${DECIDE}">${decide}</td>
      <td class="px-3 py-2 font-mono text-gray-900 dark:text-white">#${a.id}</td>
      <td class="px-3 py-2"><span class="font-mono text-gray-900 whitespace-nowrap dark:text-white">${esc(a.tool)}</span>${rule}</td>
      <td class="px-3 py-2 font-mono text-xs break-all min-w-56">${esc(JSON.stringify(a.args))}</td>
      <td class="px-3 py-2">${esc(companyOf(a.args))}</td>
      <td class="px-3 py-2"><button type="button" data-always="${a.id}" data-id="${a.id}" data-role="always" class="${SMALL}" ${rules ? '' : 'disabled title="the game\'s approval core keeps no standing rules"'}>Always…</button></td>
    </tr>`;
}

/** The standing rules: effect, tool glob, matcher, who made it, hits, kept across restarts, Remove. */
export function rulesHtml(rules) {
  if (!rules.length) return '<tr><td class="px-3 py-3" colspan="8">no standing rule yet: "Always…" on a row makes one</td></tr>';
  return rules.map((r) => `<tr class="bg-white border-b dark:bg-gray-800 dark:border-gray-700">
      <td class="px-3 py-2 font-mono">#${esc(r.id)}</td>
      <td class="px-3 py-2">${badge(`always ${r.effect}`, r.effect === 'deny' ? 'red' : 'green')}</td>
      <td class="px-3 py-2 font-mono text-gray-900 dark:text-white">${esc(r.tool)}</td>
      <td class="px-3 py-2 font-mono text-xs break-all">${r.match ? esc(JSON.stringify(r.match)) : 'any arguments'}</td>
      <td class="px-3 py-2">${esc(r.created_via ?? '')}</td>
      <td class="px-3 py-2 tabular-nums">${esc(r.hits ?? 0)}</td>
      <td class="px-3 py-2">${r.persisted ? badge('kept', 'blue') : badge('this run', 'gray')}</td>
      <td class="px-3 py-2"><button type="button" data-rule-remove="${esc(r.id)}" class="${SMALL}" aria-label="Remove rule #${esc(r.id)}">Remove</button></td>
    </tr>`).join('');
}

/** The options of a facet select: "every <what>" then each value with its count. */
export const optionsHtml = (values, every) => `<option value="">${esc(every)}</option>`
  + values.map(([v, n]) => `<option value="${esc(v)}">${esc(v)} (${n})</option>`).join('');

/** The rule editor, a Flowbite modal prefilled from one row. */
export function editorHtml(a, { ownerOnly = false } = {}) {
  const m = matcherOf(a.args);
  const radio = (v, label, extra = '') => `<label class="inline-flex items-center gap-2 text-sm font-medium text-gray-900 dark:text-white"><input type="radio" name="rule-effect" value="${v}" ${extra} class="${UI.check} rounded-full">${label}</label>`;
  return `<div id="rule-editor" role="dialog" aria-modal="true" aria-labelledby="rule-editor-title" tabindex="-1" class="fixed inset-0 z-50 flex items-center justify-center w-full h-full p-4 bg-gray-900/50 dark:bg-gray-900/80">
  <div class="relative w-full max-w-xl bg-white rounded-lg shadow-sm dark:bg-gray-700">
    <div class="flex items-center justify-between p-4 border-b border-gray-200 rounded-t dark:border-gray-600">
      <h3 id="rule-editor-title" class="text-lg font-semibold text-gray-900 dark:text-white">Standing rule from #${a.id}</h3>
      <button type="button" data-rule-cancel aria-label="Close" class="inline-flex items-center justify-center w-8 h-8 text-sm text-gray-400 bg-transparent rounded-lg hover:bg-gray-200 hover:text-gray-900 dark:hover:bg-gray-600 dark:hover:text-white">&times;</button>
    </div>
    <form id="rule-form" class="p-4 space-y-4">
      <div class="flex gap-6">${radio('allow', 'Always allow', ownerOnly ? 'disabled' : 'checked')}${radio('deny', 'Always deny', ownerOnly ? 'checked' : '')}</div>
      ${ownerOnly ? '<p class="text-sm text-gray-500 dark:text-gray-400">This tool is decided at the game\'s console alone: no allow rule of the panel decides it.</p>' : ''}
      <div><label for="rule-tool" class="block mb-2 text-sm font-medium text-gray-900 dark:text-white">Tool (glob: * any run, ? one character)</label>
        <input id="rule-tool" required value="${esc(a.tool)}" class="block w-full font-mono ${UI.input}"></div>
      <div><label class="inline-flex items-center gap-2 mb-2 text-sm font-medium text-gray-900 dark:text-white"><input id="rule-use-match" type="checkbox" ${m ? 'checked' : ''} class="${UI.check}">Only for these arguments (JSON matcher)</label>
        <textarea id="rule-match" rows="4" spellcheck="false" class="block w-full font-mono text-xs ${UI.input}">${esc(m ? JSON.stringify(m, null, 2) : '{}')}</textarea>
        <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">Plain keys match a subset; "$.a.b", "$.list[*]" are JSON paths; a string is a glob ("max_*"); "*" is any present value.</p></div>
      <label class="inline-flex items-center gap-2 text-sm font-medium text-gray-900 dark:text-white"><input id="rule-persist" type="checkbox" class="${UI.check}">Keep it across restarts</label>
      <p id="rule-dryrun" aria-live="polite" class="text-sm font-medium text-gray-900 dark:text-white"></p>
      <p id="rule-error" role="alert" class="hidden text-sm font-medium text-red-600 dark:text-red-500"></p>
      <div class="flex justify-end gap-2"><button type="button" data-rule-cancel class="${UI.secondary}">Cancel</button><button type="submit" class="${UI.primary}">Add rule</button></div>
    </form>
  </div>
</div>`;
}

/** A confirmation, a Flowbite modal: the question and the button that answers yes. */
export function askHtml(question, yes) {
  return `<div id="approve-confirm" role="dialog" aria-modal="true" aria-labelledby="approve-confirm-title" tabindex="-1" class="fixed inset-0 z-50 flex items-center justify-center w-full h-full p-4 bg-gray-900/50 dark:bg-gray-900/80">
  <div class="relative w-full max-w-md p-4 text-center bg-white rounded-lg shadow-sm dark:bg-gray-700">
    <h3 id="approve-confirm-title" class="mb-4 text-lg font-normal text-gray-700 dark:text-gray-300">${esc(question)}</h3>
    <div class="flex justify-center gap-2"><button type="button" data-confirm="no" class="${UI.secondary}">Cancel</button><button type="button" data-confirm="yes" class="${UI.primary}">${esc(yes)}</button></div>
  </div>
</div>`;
}

/** The confirmation of a bulk approval, naming the count. */
export const confirmHtml = (n, picked) => askHtml(`Approve ${n} ${picked ? 'selected ' : ''}approval${n === 1 ? '' : 's'}? The game carries each out at once.`, `Approve ${n}`);

/* ---- the page ------------------------------------------------------------------------------- */

/** The list's state: the server's last answer, the filter, the picked rows, the rows decided here
 *  that the server still lists, the focused row and the anchor of a Shift+click range. */
const S = { view: null, filter: filterOf(globalThis.location?.search), picked: new Set(), gone: new Set(), current: null, anchor: null,
  order: '', movedAt: 0, pointerAt: 0, lockUntil: 0, prev: null, rowMovedAt: new Map(), hover: false, queued: null };

/** Whether row `id` moved under the pointer (a refresh re-ordered it or put a row above it) too
 *  recently to take a click. */
const rowMoved = (id) => Date.now() - (S.rowMovedAt.get(id) || 0) < ROW_GUARD_MS;

/** Whether a pointer click comes too soon after the rows moved under a pointer that has not moved:
 *  a double click, or a poll that took a row away above, would otherwise decide another row. */
const tooSoon = (e) => e.detail > 1 || (e.detail >= 1 && Date.now() - S.movedAt < GUARD_MS && S.pointerAt < S.movedAt);

/** The rows on the page now (the decided ones left out). */
const shown = () => rowsOf(S.view ? S.view.approvals : [], S.filter).filter((a) => !S.gone.has(a.id));

function render(fromRefresh = false) {
  const root = $('approvals');
  if (!root || !S.view) return;
  const rows = shown();
  const ids = rows.map((a) => a.id);
  if (fromRefresh) for (const id of movedRows(S.prev, ids)) S.rowMovedAt.set(id, Date.now());
  S.prev = ids;
  if (ids.join() !== S.order) { S.order = ids.join(); S.movedAt = Date.now(); }
  for (const id of [...S.picked]) if (!rows.some((a) => a.id === id && decidable(a))) S.picked.delete(id);
  if (!ids.includes(S.current)) S.current = ids[0] ?? null;
  // Focus survives the redraw: the same control of the same row.
  const act = document.activeElement;
  const keep = act && root.contains(act) && act.closest('#pending') ? { id: act.dataset.id || act.dataset.row, role: act.dataset.role || 'row' } : null;
  // Redrawn only when it changed: a poll that redraws under the pointer would swallow a click.
  draw($('pending'), rows.length ? rows.map((a) => rowHtml(a, { picked: S.picked.has(a.id), current: a.id === S.current, rules: S.view.rules_supported })).join('')
    : '<tr><td class="px-3 py-3" colspan="7">none waiting</td></tr>');
  if (keep) focusRow(Number(keep.id) && ids.includes(Number(keep.id)) ? Number(keep.id) : S.current, keep.role);
  const head = headerState(rows, S.picked);
  Object.assign($('approvals-pick-all'), head);
  const n = bulkIds(rows, S.picked).length;
  const labels = bulkLabels(n, S.picked.size > 0);
  $('approvals-approve-bulk').textContent = labels.approve;
  $('approvals-deny-bulk').textContent = labels.deny;
  $('approvals-approve-bulk').disabled = $('approvals-deny-bulk').disabled = !n;
  const all = S.view.approvals;
  $('approvals-count').textContent = `${rows.length} shown of ${all.filter((a) => a.state === 'pending').length} waiting`;
  const sel = (id, values, every) => {
    const el = $(id);
    draw(el, optionsHtml(values, every));
    el.value = S.filter[id === 'approvals-tool' ? 'tool' : 'company'];
  };
  sel('approvals-tool', facet(all, (a) => a.tool), 'Every tool');
  const companies = facet(all, (a) => companyOf(a.args));
  sel('approvals-company', companies, 'Every company');
  $('approvals-company').hidden = !companies.length && !S.filter.company;
  draw($('approvals-rules'), rulesHtml(S.view.rules || []));
  $('approvals-rules-note').hidden = !!S.view.rules_supported;
}

/** Put `html` into `el` unless it already holds it. */
function draw(el, html) {
  if (el.dataset.html !== html) { el.innerHTML = html; el.dataset.html = html; }
}

/** Focus row `id` (its `role` control when it has one: yes, no, pick, always); a key that moves
 *  (`scroll`) brings it into view, a decision never scrolls: the next row stays under the pointer. */
function focusRow(id, role = 'row', scroll = false) {
  const tr = document.querySelector(`#pending tr[data-row="${id}"]`);
  if (!tr) return;
  S.current = id;
  for (const r of document.querySelectorAll('#pending tr[data-row]')) r.tabIndex = r === tr ? 0 : -1;
  const el = (role !== 'row' && tr.querySelector(`[data-role="${role}"]:not([disabled])`)) || tr;
  el.focus({ preventScroll: true });
  if (scroll) tr.scrollIntoView({ block: 'nearest' });
}

async function refresh() {
  const v = await api('/api/approvals', undefined, { shownIn: 'approvals-msg' });
  if (!v.ok) return;
  // The rows never move under a pointer over the list: the answer waits until it leaves.
  if (S.hover) { S.queued = v; return; }
  apply(v);
}

/** Take the server's answer and draw it (a refresh). */
function apply(v) {
  S.view = v;
  // A row decided here leaves the set once the server no longer lists it as waiting.
  for (const id of [...S.gone]) if (!v.approvals.some((a) => a.id === id && a.state === 'pending')) S.gone.delete(id);
  render(true);
}

/** Decide one row: it leaves at once and the next row takes its place and the focus; a failure
 *  brings it back (the API client shows why). */
async function decide(id, approve) {
  const ids = shown().map((a) => a.id);
  const next = ids[ids.indexOf(id) + 1] ?? ids[ids.indexOf(id) - 1];
  // The next row's same control takes the focus (its Yes after a Yes), or the row itself after a key.
  const act = document.activeElement;
  const role = act && act.closest && act.closest('#pending') ? act.dataset.role || 'row' : 'row';
  S.lockUntil = Date.now() + GUARD_MS;
  S.gone.add(id);
  S.picked.delete(id);
  render();
  if (next !== undefined) focusRow(next, role);
  const r = await api(approve ? '/api/approve' : '/api/deny', { id });
  if (r.ok) notify(approve ? `#${id} approved: ${r.status} ${r.command_result ?? ''}`.trim() : `#${id} denied`, 'green');
  else { S.gone.delete(id); render(); }
  refresh();
}

/** Approve or deny the picked rows (else every row the filter shows) through one bulk route. */
async function decideBulk(approve) {
  const rows = shown();
  const ids = bulkIds(rows, S.picked);
  if (!ids.length) return;
  if (approve && !(await ask(confirmHtml(ids.length, S.picked.size > 0)))) return;
  S.lockUntil = Date.now() + GUARD_MS;
  ids.forEach((id) => S.gone.add(id));
  S.picked.clear();
  render();
  const r = await api(approve ? '/api/approve/many' : '/api/deny/many', { ids });
  if (r.ok) {
    const failed = r.results.filter((x) => x.code !== 200);
    failed.forEach((x) => S.gone.delete(x.id));
    notify(`${approve ? 'Approved' : 'Denied'} ${r.done} of ${ids.length}`, 'green');
    if (failed.length) notify(`${failed.length} not ${approve ? 'approved' : 'denied'}: ${failed.map(failureText).join('; ')}`, 'red');
  } else ids.forEach((id) => S.gone.delete(id));
  refresh();
}

/** Ask first (a bulk approval, removing a rule): true on yes, false on Cancel or Escape. */
function ask(html) {
  return new Promise((resolve) => {
    document.body.insertAdjacentHTML('beforeend', html);
    const box = $('approve-confirm');
    const done = (ok) => { box.remove(); resolve(ok); };
    box.querySelector('[data-confirm="yes"]').onclick = () => done(true);
    box.querySelector('[data-confirm="no"]').onclick = () => done(false);
    box.onkeydown = (e) => { if (e.key === 'Escape') done(false); };
    box.querySelector('[data-confirm="yes"]').focus();
  });
}

/** The rule editor for row `id`; Add rule sends it, the answer is shown, the list redrawn. */
function openEditor(id) {
  const a = S.view.approvals.find((x) => x.id === id);
  if (!a) return;
  document.body.insertAdjacentHTML('beforeend', editorHtml(a, { ownerOnly: a.owner_only }));
  const box = $('rule-editor');
  const close = () => { box.remove(); focusRow(id, 'always'); };
  box.querySelectorAll('[data-rule-cancel]').forEach((b) => { b.onclick = close; });
  box.onkeydown = (e) => { if (e.key === 'Escape') close(); };
  const fields = () => ({ effect: box.querySelector('input[name="rule-effect"]:checked').value, tool: $('rule-tool').value,
    matcher: $('rule-match').value, useMatcher: $('rule-use-match').checked, persist: $('rule-persist').checked });
  // What the rule would decide now, before it is made (a dry run over the rows the page holds).
  const preview = () => { $('rule-dryrun').textContent = dryRunText(dryRun(S.view.approvals, fields())); };
  $('rule-form').oninput = preview;
  $('rule-form').onchange = preview;
  preview();
  $('rule-form').onsubmit = async (e) => {
    e.preventDefault();
    const { body, error } = ruleBody(fields());
    const err = $('rule-error');
    if (error) { err.textContent = error; err.classList.remove('hidden'); return; }
    const r = await api('/api/rules', body, { shownIn: err });
    if (!r.ok) { err.textContent = err.dataset.failure || 'refused'; err.classList.remove('hidden'); return; }
    notify(`Rule #${r.rule.id}: ${ruleText(r.rule)}`, 'green');
    close();
    refresh();
  };
  $('rule-tool').focus();
}

/** Pick or drop row `id`; with Shift, every row from the anchor to it alike. The focus stays on the
 *  row's `role` control (its checkbox after a click, the row after Space). */
function pick(id, on, range, role = 'pick') {
  const ids = shown().filter(decidable).map((a) => a.id);
  const span = range && S.anchor !== null ? rangeIds(ids, S.anchor, id) : [id];
  for (const x of span) { if (on) S.picked.add(x); else S.picked.delete(x); }
  if (!range) S.anchor = id;
  render();
  focusRow(id, role);
}

/** The keys of the list, while the focus is in it (never while typing, never in a dialog). */
function onKey(e) {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  // A held key decides one row, never the queue: no auto-repeat, and none just after a decision.
  if (DECISIVE.has(e.key) && (e.repeat || (e.key !== ' ' && Date.now() < S.lockUntil))) { e.preventDefault(); return; }
  const t = e.target;
  if (/^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) && t.type !== 'checkbox') return;
  const tr = t.closest('tr[data-row]');
  const id = tr ? Number(tr.dataset.row) : S.current;
  const ids = shown().map((a) => a.id);
  const a = shown().find((x) => x.id === id);
  const onRow = t === tr;
  const key = e.key;
  if (key === 'j' || key === 'ArrowDown' || key === 'k' || key === 'ArrowUp') {
    const step = key === 'j' || key === 'ArrowDown' ? 1 : -1;
    const to = ids[Math.max(0, Math.min(ids.length - 1, ids.indexOf(id) + step))];
    if (to !== undefined) { e.preventDefault(); focusRow(to, 'row', true); }
    return;
  }
  if (!a) return;
  if ((key === 'y' || key === 'a' || (key === 'Enter' && onRow)) && decidable(a)) { e.preventDefault(); decide(id, true); }
  else if ((key === 'n' || key === 'd') && decidable(a)) { e.preventDefault(); decide(id, false); }
  else if (key === ' ' && onRow && decidable(a)) { e.preventDefault(); pick(id, !S.picked.has(id), e.shiftKey, 'row'); }
}

export function mount() {
  const root = $('approvals');
  if (!root) return;
  $('approvals-q').value = S.filter.q;
  $('approvals-expired').checked = S.filter.expired;
  const setFilter = (patch) => {
    Object.assign(S.filter, patch);
    patchAddress(filterPatch(S.filter), (k) => KEYS[k]);
    render();
  };
  $('approvals-tool').onchange = (e) => setFilter({ tool: e.target.value });
  $('approvals-company').onchange = (e) => setFilter({ company: e.target.value });
  $('approvals-q').oninput = (e) => setFilter({ q: e.target.value });
  $('approvals-expired').onchange = (e) => setFilter({ expired: e.target.checked });
  $('approvals-pick-all').onchange = (e) => {
    for (const a of shown().filter(decidable)) { if (e.target.checked) S.picked.add(a.id); else S.picked.delete(a.id); }
    render();
  };
  $('approvals-approve-bulk').onclick = (e) => (tooSoon(e) ? undefined : decideBulk(true));
  $('approvals-deny-bulk').onclick = (e) => (tooSoon(e) ? undefined : decideBulk(false));
  document.addEventListener('pointermove', (e) => { S.pointerAt = Date.now(); S.hover = !!e.target.closest?.('#approvals-list'); }, { passive: true });
  // Leaving the list applies the answer that waited while the pointer was over it.
  $('approvals-list').addEventListener('pointerleave', () => {
    S.hover = false;
    if (S.queued) { const v = S.queued; S.queued = null; apply(v); }
  });
  $('pending').addEventListener('click', (e) => {
    const t = e.target.closest('[data-approve],[data-deny],[data-select],[data-always],tr[data-row]');
    if (!t) return;
    if ((t.dataset.approve || t.dataset.deny) && tooSoon(e)) return undefined;
    // A row that a refresh just moved under the pointer takes no Approve, Deny or Always.
    if ((t.dataset.approve || t.dataset.deny || t.dataset.always) && e.detail >= 1 && rowMoved(Number(t.closest('tr[data-row]').dataset.row))) return undefined;
    if (t.dataset.approve) return decide(Number(t.dataset.approve), true);
    if (t.dataset.deny) return decide(Number(t.dataset.deny), false);
    if (t.dataset.select) return pick(Number(t.dataset.select), t.checked, e.shiftKey);
    if (t.dataset.always) return openEditor(Number(t.dataset.always));
    return focusRow(Number(t.dataset.row));
  });
  $('pending').addEventListener('keydown', onKey);
  $('approvals-rules').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-rule-remove]');
    if (!b) return;
    const rule = (S.view.rules || []).find((r) => String(r.id) === b.dataset.ruleRemove) || { id: b.dataset.ruleRemove, effect: '', tool: '' };
    if (!(await ask(askHtml(`Remove rule #${rule.id} (${ruleText(rule)})? Requests it would have decided wait for you again.`, 'Remove')))) return;
    b.disabled = true;
    const r = await api('/api/rules/remove', { id: Number(b.dataset.ruleRemove) });
    if (r.ok) notify(`Rule #${r.removed} removed`, 'green');
    b.disabled = false;
    refresh();
  });
  refresh();
  setInterval(refresh, 1500);
}

if (globalThis.document) mount();
