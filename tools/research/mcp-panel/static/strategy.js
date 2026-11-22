/* The strategist and the blackboard in the panel (research/mechanisms/strategist-and-blackboard.md,
   src/panel_strategy.rs): the background strategist's state and its last cycle (GET /api/strategy),
   the current strategy it keeps on the blackboard (portfolio, investment, rail lines priced on the
   map, town warnings, BattleBrain's tuning proposals, a bot settings suggestion that is never
   applied), what the peers on the Majordomus mesh plan, and the whole blackboard with every key's
   writer, revision and history (GET /api/blackboard). The prefix filter and the open key live in
   the address. Read-only but for "Run a cycle now" (POST /api/strategy/run). */

import { UI, CLS, $, esc, api, whole, badge, patchAddress } from './components.js';

/* ---- address state ------------------------------------------------------------------------- */
export function getState(href = globalThis.location?.href) {
  const q = new URL(href).searchParams;
  return { prefix: q.get('prefix') || '', key: q.get('key') || undefined };
}

/* ---- pure helpers: answers in, rows out ---------------------------------------------------- */

/** A state's badge tone. */
export function stateTone(state) {
  return { ok: 'green', running: 'blue', waiting: 'yellow', off: 'gray', error: 'red' }[state] || 'gray';
}

/** A number as the panel writes it, or a dash. */
const num = (v) => (typeof v === 'number' ? whole.format(Math.round(v)) : '–');

/** Seconds since a unix time, short ("12 s", "3 min", "2 h"). */
export function ago(t, now = Date.now() / 1000) {
  if (typeof t !== 'number') return '';
  const s = Math.max(0, Math.round(now - t));
  return s < 90 ? `${s} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${Math.round(s / 3600)} h`;
}

/** A value's first characters as one line of JSON. */
export function preview(v, max = 120) {
  const t = JSON.stringify(v) ?? 'null';
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

/** strategy.summary as one sentence. */
export function headline(s) {
  if (!s || s.cycle == null) return 'No strategy yet: the strategist runs its first cycle once a game is up.';
  return `Cycle ${s.cycle} at ${s.game_date ?? '?'} (${s.trigger}): ${s.lines ?? 0} lines, score ${num(s.total_score)}, `
    + `by ${s.solver ?? 'no solver'}; ${s.investment_lines ?? 0} investment line(s), ${s.town_warnings ?? 0} town warning(s), `
    + `${s.tuning_proposals ?? 0} tuning proposal(s); ${num(s.duration_ms)} ms.`;
}

/** The parts of the last cycle: [{name, ok, ms, note}]. */
export function parts(last) {
  return Object.entries(last?.parts || {}).map(([name, p]) => ({ name, ok: p.ok, ms: p.ms, note: p.error || p.skipped || '' }));
}

/** The chosen lines of a portfolio as the board keeps it (strategy.plan, .investment, .track). */
export const chosen = (pf) => pf?.portfolio?.chosen || [];

/** Every town warning of strategy.towns, flattened with its company. */
export function warnings(towns) {
  return (towns?.companies || []).flatMap((c) => (c.warnings || []).map((w) => ({ company: c.company, ...w })));
}

/** BattleBrain's proposals of strategy.tuning. */
export const proposals = (t) => t?.proposals || [];

/** The peers' records and intents of mesh.peers. */
export function peers(p) {
  return { records: p?.records || [], intents: p?.intents || [], source: p?.source || 'none', reason: p?.reason || '' };
}

/* ---- renderers: data in, HTML out ----------------------------------------------------------- */

const head = (title, sub, actions = '') => `<div class="${UI.cardHead} mb-4"><div class="${UI.cardHeadTitle}"><h2 class="${UI.cardTitle}">${esc(title)}</h2>${sub ? `<p class="mt-2 ${UI.cardSub}">${sub}</p>` : ''}</div>${actions ? `<div class="flex flex-wrap items-center gap-2">${actions}</div>` : ''}</div>`;
const apiLink = (op) => `<a href="/docs#/default/${op}" class="text-sm text-primary-600 hover:underline dark:text-primary-500">API</a>`;
const table = (cols, rows, empty = 'nothing') => `<div class="${UI.tableWrap}"><table class="${UI.table}"><thead class="${UI.thead}"><tr>${cols.map((c) => `<th class="${CLS.cell}">${esc(c)}</th>`).join('')}</tr></thead>
  <tbody>${rows.length ? rows.join('') : `<tr><td class="${CLS.cell}" colspan="${cols.length}">${esc(empty)}</td></tr>`}</tbody></table></div>`;
const tr = (cells, attrs = '') => `<tr class="border-b dark:border-gray-700"${attrs}>${cells.map((c) => `<td class="${CLS.cell} align-top">${c}</td>`).join('')}</tr>`;
const card = (inner) => `<div class="${CLS.card}">${inner}</div>`;
const strong = (t) => `<span class="font-medium text-gray-900 dark:text-white">${esc(t)}</span>`;
const note = (t) => (t ? `<p class="mt-2 ${CLS.muted}">${esc(t)}</p>` : '');

/** The strategist card: state, reason, schedule, AlgoGlyph's runner and the last cycle's parts. */
export function statusHtml(s) {
  const st = s.status || {};
  const c = s.config || {};
  const badges = [badge(st.state || 'unknown', stateTone(st.state)), badge(s.enabled ? 'background on' : 'background off', s.enabled ? 'green' : 'gray'),
    badge(`${s.cycles ?? 0} cycle(s)`, 'blue')].join(' ');
  const next = s.next_run_in_s == null ? 'only on demand' : `at the latest in ${s.next_run_in_s} s`;
  const algo = s.summary?.algoglyph || {};
  const sub = `${badges} ${esc(st.reason || '')}. Next run ${esc(next)}; every ${esc(c.every_s)} s and on each new game month (no sooner than ${esc(c.min_gap_s)} s apart), ${esc(c.workers)} workers. AlgoGlyph: ${esc(algo.runner || 'not checked yet')}${algo.dir ? ` (${esc(algo.dir)})` : ''}.`;
  const rows = parts(s.last).map((p) => tr([strong(p.name), badge(p.ok ? 'ok' : 'error', p.ok ? 'green' : 'red'), esc(num(p.ms)), esc(p.note)]));
  const run = `<button id="s-run" type="button" class="${UI.primary}">Run a cycle now</button>`;
  return card(`${head('Strategist', sub, `${run}${apiLink('strategy')}`)}
    ${table(['Part', 'Outcome', 'ms', 'Note'], rows, 'no cycle yet')}
    ${(st.notes || []).map((n) => note(n)).join('')}<p id="s-run-msg" class="mt-2 ${CLS.muted}"></p>`);
}

const lineRows = (pf) => chosen(pf).map((l) => tr([esc(l.cargo || ''), esc(l.mode), `${strong(l.from)} → ${strong(l.to)}`, esc(num(l.score)),
  esc(num(l.investment)), esc(num(l.monthly_profit)), esc(l.track || '')]));
const LINE_COLS = ['Cargo', 'Mode', 'Line', 'Score', 'Investment', 'Profit / month', 'Track'];

/** A portfolio part: its solver and lines, or why it was skipped or failed. */
export function portfolioHtml(title, pf, op) {
  if (!pf) return card(head(title, 'Not computed yet.', apiLink(op)));
  if (pf.skipped || pf.error) return card(head(title, esc(pf.skipped || `Failed: ${pf.error}`), apiLink(op)));
  const p = pf.portfolio || {};
  const sub = `Solver ${badge(p.solver || 'none', /^algoglyph/.test(p.solver || '') ? 'green' : 'yellow')}, total score ${esc(num(p.total_score))}, ${esc(num(p.investment_k))}k of ${esc(num(p.budget_k))}k.`;
  return card(`${head(title, sub, apiLink(op))}${table(LINE_COLS, lineRows(pf), 'no line chosen')}`);
}

/** The town warnings of every rated company. */
export function townsHtml(towns) {
  if (!towns) return card(head('Town warnings', 'Not computed yet.', apiLink('strategy')));
  if (towns.error) return card(head('Town warnings', `Failed: ${esc(towns.error)}`, apiLink('strategy')));
  const rows = warnings(towns).map((w) => tr([esc(w.company), strong(w.town), w.rating == null ? 'unknown' : esc(w.rating),
    w.band ? badge(String(w.band).replace(/^TOWN_RATING_/, ''), 'red') : '', w.refuses_stations ? badge('refuses stations', 'red') : '',
    `<ul class="space-y-1">${(w.advice || []).map((a) => `<li title="${esc(`${a.why || ''} (${a.source || ''})`)}">${esc(a.what)}</li>`).join('')}</ul>`]));
  const sub = `${esc(towns.towns ?? 0)} towns, rated companies ${esc((towns.rated_companies || []).join(', ') || 'none')}, tolerance ${badge(towns.tolerance || 'lenient', 'blue')} (${esc(towns.tolerance_source || '')}); advice from town_lib.rhai (see <a class="text-primary-600 hover:underline dark:text-primary-500" href="/towns">Towns</a>).`;
  return card(`${head('Town warnings', sub, apiLink('strategy'))}${table(['Company', 'Town', 'Rating', 'Band', 'Gate', 'Advice'], rows, 'no town warns')}`);
}

/** BattleBrain's tuning proposals. */
export function tuningHtml(t) {
  const title = 'Tuning proposals (BattleBrain)';
  if (!t) return card(head(title, 'Not read yet.', apiLink('strategy')));
  if (t.skipped || t.error) return card(head(title, esc(t.skipped || `Failed: ${t.error}`), apiLink('strategy')));
  const rows = proposals(t).map((p) => tr([badge(p.kind, p.kind === 'safe' ? 'green' : 'purple'), esc(p.predicted_share), esc(p.spread),
    `<code class="text-xs break-all">${esc(p.settings)}</code>`]));
  const sub = `${esc(t.rows)} rows, ${esc(t.distinct_settings)} distinct settings, from ${esc(t.file)} written ${esc(ago(t.mtime))} ago. Read-only: the strategist never writes it nor calls the GA.`;
  return card(`${head(title, sub, apiLink('strategy'))}${table(['Kind', 'Predicted share', 'Spread', 'Settings'], rows, 'no proposal')}`);
}

/** The bot settings suggestion (data only). */
export function suggestionHtml(sg) {
  if (!sg) return '';
  return card(`${head('Bot settings suggestion', esc(sg.data_only || ''), apiLink('strategy'))}<code class="text-xs break-all">${esc(sg.text || '')}</code>`);
}

/** What the peers on the mesh plan, and how the strategist cooperated this cycle. */
export function peersHtml(p, mesh) {
  const x = peers(p);
  const rows = x.records.map((r) => tr([badge(r.kind, 'blue'), esc(r.author), strong(r.title), esc(r.task), esc(r.revision)]))
    .concat(x.intents.map((i) => tr([badge('intent', 'gray'), esc(i.session), strong(i.intent), esc(i.via), esc(ago(i.at))])));
  const m = mesh || {};
  const out = [m.skipped && `skipped: ${m.skipped}`, (m.records || []).length && `records: ${(m.records || []).map((r) => r.written || (r.unchanged ? 'unchanged' : '')).join(', ')}`,
    m.records_skipped && `no record: ${m.records_skipped}`, m.announce && `announce ${m.announce.claim}: ${m.announce.announced ? 'announced' : m.announce.unchanged ? 'unchanged' : m.announce.skipped}`]
    .filter(Boolean).join('; ');
  const sub = `From ${esc(x.source)}${x.reason ? ` (${esc(x.reason)})` : ''}. Ours this cycle: ${esc(out || 'nothing yet')}.`;
  return card(`${head('Mesh peers', sub, apiLink('strategy'))}${table(['Kind', 'Author / session', 'Title / intent', 'Task / via', 'Rev / age'], rows, 'no peer plans')}`);
}

/** The blackboard's rows: key, writer, revision, written, game date, value preview. */
export function boardRows(entries, open) {
  return (entries || []).map((e) => tr([`<button type="button" class="text-primary-600 hover:underline dark:text-primary-500 font-mono" data-key="${esc(e.key)}">${esc(e.key)}</button>`,
    esc(e.by), esc(e.revision), esc(ago(e.written_at)), esc(e.game_date ?? ''), `<code class="text-xs break-all">${esc(preview(e.value))}</code>`],
  e.key === open ? ' data-open="1"' : ''));
}

/** The blackboard card with its prefix filter. */
export function boardHtml(b, state) {
  const filter = `<label class="text-sm font-medium text-gray-900 dark:text-white">Prefix
    <input id="b-prefix" class="${UI.input} ms-2" value="${esc(state.prefix)}" placeholder="strategy."></label>`;
  const sub = `${esc(b.count ?? 0)} key(s)${state.prefix ? ` under ${esc(state.prefix)}` : ''}; ${esc((b.reserved || []).join('*, '))}* are the strategist's, every other key is for scripts (POST /api/blackboard); the last ${esc(b.history_kept)} values of each key are kept.`;
  return card(`${head('Blackboard', sub, `${filter}${apiLink('blackboard')}`)}
    ${table(['Key', 'By', 'Rev', 'Written', 'Game date', 'Value'], boardRows(b.entries, state.key), 'the board is empty')}`);
}

/** One key's value and history. */
export function detailHtml(e) {
  if (!e) return '';
  const hist = (e.history || []).map((h) => tr([esc(h.revision), esc(h.by), esc(ago(h.written_at)), esc(h.game_date ?? ''), `<code class="text-xs break-all">${esc(preview(h.value, 200))}</code>`]));
  return card(`${head(e.key, `Revision ${esc(e.revision)} by ${esc(e.by)}, ${esc(ago(e.written_at))} ago, game date ${esc(e.game_date ?? 'none')}.`, apiLink('blackboard'))}
    <pre class="${CLS.pre} p-3 text-xs text-gray-200 whitespace-pre-wrap break-all max-h-96 overflow-auto">${esc(JSON.stringify(e.value, null, 2))}</pre>
    <h3 class="mt-4 ${CLS.label}">History</h3>${table(['Rev', 'By', 'Written', 'Game date', 'Value'], hist, 'no earlier value')}`);
}

/* ---- the page ------------------------------------------------------------------------------- */
async function loadStrategy() {
  const s = await api('/api/strategy');
  $('status').innerHTML = statusHtml(s);
  $('summary').textContent = headline(s.summary);
  $('plan').innerHTML = portfolioHtml('Portfolio', s.plan, 'strategy');
  $('investment').innerHTML = portfolioHtml('Investment (AlgoGlyph transport-investment)', s.investment, 'strategy');
  $('track').innerHTML = portfolioHtml('Rail lines priced on the map (AlgoGlyph track-planning)', s.track, 'strategy');
  $('towns').innerHTML = townsHtml(s.towns);
  $('tuning').innerHTML = tuningHtml(s.tuning);
  $('suggestion').innerHTML = suggestionHtml(s.bot_settings_suggestion);
  $('peers').innerHTML = peersHtml(s.peers, s.status?.mesh || s.last?.mesh);
  $('s-run').onclick = async () => {
    $('s-run').disabled = true;
    $('s-run-msg').textContent = 'running a cycle…';
    // A refusal (409 without a game) is written into the message, not a toast.
    const out = await api('/api/strategy/run', { wait: true }, { shownIn: 's-run-msg' });
    await loadStrategy();
    await loadBoard(getState());
    $('s-run-msg').textContent = out.ok ? `cycle ${out.cycle} took ${num(out.duration_ms)} ms` : `refused: ${out.error}`;
  };
  return s;
}

async function loadBoard(state) {
  const q = state.prefix ? `?prefix=${encodeURIComponent(state.prefix)}&history=0` : '?history=0';
  const b = await api('/api/blackboard' + q);
  $('board').innerHTML = boardHtml(b, state);
  $('b-prefix').onchange = (ev) => {
    const prefix = ev.target.value.trim();
    patchAddress({ prefix: prefix || undefined });
    loadBoard({ ...getState(), prefix });
  };
  for (const btn of document.querySelectorAll('#board button[data-key]')) {
    btn.onclick = () => openKey(btn.getAttribute('data-key'));
  }
  if (state.key) await openKey(state.key, false);
  return b;
}

async function openKey(key, write = true) {
  if (write) patchAddress({ key });
  // A key removed since the list was read answers 404: said in the detail card, not a toast.
  const e = await api(`/api/blackboard?key=${encodeURIComponent(key)}`, undefined, { shownIn: 'detail' });
  $('detail').innerHTML = e.ok ? detailHtml(e) : card(head(key, 'No such key (any more).'));
}

/** The whole page; exported so a test mounts it in the fake browser (test/strategy.test.mjs). */
export async function mount() {
  const state = getState();
  await loadStrategy();
  await loadBoard(state);
  setInterval(async () => { await loadStrategy(); await loadBoard(getState()); }, 10000);
}

if (globalThis.document && $('status') && $('board')) mount();
