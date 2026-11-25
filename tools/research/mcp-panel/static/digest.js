/* The strategy digest in the panel (research/mechanisms/strategy-digest.md, src/panel_digest.rs,
   rhai/digest.rhai): GET /api/digest answers one company's latest digest, its week/month/quarter
   histories, every company with a digest, every active alert and the service's heartbeat. This page
   shows the level and alerts, the standing, the share of the leader by month (ECharts through the
   one door, maximizable), profit per vehicle type against the leader's, what failed or was skipped,
   what the system proposed and the approvals waiting with their countdown and default verdict, and
   the history. The company, the history's period and a maximized chart live in the address.
   Read-only. */

import { UI, CLS, $, esc, api, whole, badge, patchAddress, ecInit, maximizable, PALETTE, INK, AXIS } from './components.js';

// Read at draw time: the page script may load before the chart library (and a test's fake).
const charts = () => globalThis.echarts;

/* ---- address state ------------------------------------------------------------------------- */
export const PERIODS = ['week', 'month', 'quarter'];
export function getState(href = globalThis.location?.href) {
  const q = new URL(href).searchParams;
  const c = q.get('company');
  return {
    company: c !== null && /^\d+$/.test(c) ? Number(c) : undefined,
    period: PERIODS.includes(q.get('period')) ? q.get('period') : 'month',
    max: q.get('max') || undefined,
  };
}
export function setState(patch) { patchAddress(patch); }

/* ---- pure helpers -------------------------------------------------------------------------- */

/** A digest level's badge tone. */
export const levelTone = (level) => ({ ok: 'green', unknown: 'gray', watch: 'yellow', alert: 'red' }[level] || 'gray');
/** A share 0..1 as whole percent, or a dash. */
export const pct = (x) => (typeof x === 'number' ? `${Math.round(x * 100)}%` : '–');
/** A number as the panel writes it, or a dash. */
const num = (v) => (typeof v === 'number' ? whole.format(Math.round(v)) : '–');
/** A change in percentage points, signed. */
export const pts = (x) => (typeof x === 'number' ? `${x >= 0 ? '+' : ''}${Math.round(x * 100)} pt` : '');

/** The charts of this page, each with what drives it and where a click goes (the chart registry). */
export const CHARTS = [
  { id: 'share-chart', params: ['company'], drill: 'a month opens its row in the month history' },
  { id: 'types-chart', params: ['company'], drill: 'a type opens replan.<company>, the comparison it is read from' },
];

/** The share of the leader's value by month, with the thresholds the digest escalates at. Pure. */
export function shareOption(series = [], { dark = true } = {}) {
  const pal = dark ? PALETTE.dark : PALETTE.light;
  return {
    color: pal,
    tooltip: { trigger: 'axis', formatter: (ps) => {
      const p = ps[0]?.data;
      return p ? `${esc(p.month)}<br>${pct(p.value / 100)} of ${esc(p.leader ?? '?')} · rank ${p.rank ?? '?'} of ${p.of ?? '?'}` : '';
    } },
    grid: { left: 48, right: 24, top: 24, bottom: 56 },
    xAxis: { type: 'category', data: series.map((s) => s.month), ...AXIS },
    yAxis: { type: 'value', min: 0, max: (v) => Math.max(100, Math.ceil(v.max / 10) * 10), name: '% of the leader', axisLabel: { color: INK.muted, formatter: '{value}%' }, splitLine: AXIS.splitLine },
    dataZoom: [{ type: 'inside' }, { type: 'slider', height: 18, bottom: 8 }],
    series: [{
      type: 'line', name: 'share of the leader', showSymbol: true, symbolSize: 6,
      data: series.map((s) => ({ value: typeof s.share === 'number' ? Math.round(s.share * 1000) / 10 : null, month: s.month, leader: s.leader, rank: s.rank, of: s.of })),
      markLine: { symbol: 'none', silent: true, label: { color: INK.muted, position: 'insideEndTop' }, lineStyle: { type: 'dashed' },
        data: [{ yAxis: 50, name: 'behind', label: { formatter: 'behind 50%' } }, { yAxis: 25, name: 'far behind', label: { formatter: 'far behind 25%' } }] },
    }],
  };
}

/** Profit per vehicle by transport type, ours against the leader's (replan's comparison). Pure. */
export function typesOption(byType = [], { dark = true } = {}) {
  const pal = dark ? PALETTE.dark : PALETTE.light;
  return {
    color: pal,
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
    legend: { textStyle: { color: dark ? INK.text : '#374151' } },
    grid: { left: 64, right: 24, top: 36, bottom: 32 },
    xAxis: { type: 'category', data: byType.map((t) => t.type), ...AXIS },
    yAxis: { type: 'value', name: 'profit per vehicle', ...AXIS },
    series: [
      { type: 'bar', name: 'ours', data: byType.map((t) => ({ value: t.ours_per_vehicle ?? null, type: t.type })) },
      { type: 'bar', name: 'the leader of the type', data: byType.map((t) => ({ value: t.leader_per_vehicle ?? null, type: t.type, leader: t.leader })) },
    ],
  };
}

/* ---- renderers: data in, HTML out ----------------------------------------------------------- */

const head = (title, sub, actions = '') => `<div class="${UI.cardHead} mb-4"><div class="${UI.cardHeadTitle}"><h2 class="${UI.cardTitle}">${esc(title)}</h2>${sub ? `<p class="mt-2 ${UI.cardSub}">${sub}</p>` : ''}</div>${actions ? `<div data-toolbar class="flex flex-wrap items-center gap-2">${actions}</div>` : ''}</div>`;
const card = (inner, attrs = '') => `<div class="${CLS.card}"${attrs}>${inner}</div>`;
const table = (cols, rows, empty) => `<div class="${UI.tableWrap}"><table class="${UI.table}"><thead class="${UI.thead}"><tr>${cols.map((c) => `<th class="${CLS.cell}">${esc(c)}</th>`).join('')}</tr></thead>
  <tbody>${rows.length ? rows.join('') : `<tr><td class="${CLS.cell}" colspan="${cols.length}">${esc(empty)}</td></tr>`}</tbody></table></div>`;
const tr = (cells, attrs = '') => `<tr class="border-b dark:border-gray-700"${attrs}>${cells.map((c) => `<td class="${CLS.cell} align-top">${c}</td>`).join('')}</tr>`;
const link = (href, text) => `<a href="${esc(href)}" class="font-medium text-primary-600 hover:underline dark:text-primary-500">${esc(text)}</a>`;
const list = (items, empty) => (items.length ? `<ul class="space-y-1 text-sm text-gray-700 dark:text-gray-300 list-disc ps-5">${items.map((x) => `<li>${esc(x)}</li>`).join('')}</ul>` : `<p class="${CLS.muted}">${esc(empty)}</p>`);

/** The head card: the company switch, the level, the heartbeat and every active alert. */
export function headHtml(a) {
  const d = a.digest;
  const hb = a.heartbeat || {};
  const live = hb.alive ? badge(`service alive (${hb.age_s} s ago)`, 'green')
    : badge(hb.age_s == null ? 'service not running' : `service stale (${hb.age_s} s)`, 'red');
  const tabs = (a.companies || []).map((c) => `<a href="/digest?company=${c.company}" data-company="${c.company}" class="${c.company === a.company ? UI.primary : UI.secondary}">${esc(c.name ?? `company ${c.company}`)} ${badge(c.level || '?', levelTone(c.level))}</a>`).join('');
  const why = hb.why ? `<p class="mt-2 ${CLS.muted}">${esc(hb.why)}</p>` : '';
  if (!d) return card(`${head('Strategy digest', 'No digest yet.', live)}${why}${tabs ? `<div class="flex flex-wrap gap-2">${tabs}</div>` : ''}`);
  const alerts = (d.alerts || []).map((x) => `<div role="${x.level === 'alert' ? 'alert' : 'status'}" data-alert="${esc(x.key)}" class="flex items-start gap-3 p-3 mb-2 text-sm rounded-lg ${x.level === 'alert' ? 'text-red-800 bg-red-50 dark:bg-gray-800 dark:text-red-400 border border-red-300 dark:border-red-800' : 'text-yellow-800 bg-yellow-50 dark:bg-gray-800 dark:text-yellow-300 border border-yellow-300 dark:border-yellow-800'}">${badge(x.level, levelTone(x.level))}<span><span class="font-semibold">${esc(x.key)}</span>: ${esc(x.text)}${(d.new_alerts || []).includes(x.key) ? ` ${badge('new', 'blue')}` : ''}</span></div>`).join('');
  const missing = a.requested_missing != null ? `<p class="mb-2 ${CLS.muted}">Company ${esc(a.requested_missing)} has no digest; showing ${esc(d.name)}.</p>` : '';
  const unknown = (d.unknown || []).length ? `<p class="mt-2 ${CLS.muted}">Unknown (source missing or stale): ${esc(d.unknown.join(', '))}</p>` : '';
  return card(`${head(`${d.name}: ${d.label}`, `${esc(d.period)} digest of ${esc(d.date)} ${badge(d.level, levelTone(d.level))}${d.owner ? ` ${badge('your company', 'blue')}` : ''}`, live)}
    ${missing}<div class="flex flex-wrap gap-2 mb-4">${tabs}</div>${alerts || `<p class="${CLS.muted}">No alert.</p>`}${(d.cleared || []).length ? `<p class="mt-2 ${CLS.muted}">Cleared: ${esc(d.cleared.join(', '))}</p>` : ''}${unknown}${why}`);
}

/** The standing as stat tiles: rank, share, fleet, profit, money, approvals. */
export function standingHtml(d) {
  if (!d) return '';
  const st = d.standing;
  const tile = (label, value, sub = '') => `<div class="${UI.card}"><p class="${CLS.muted}">${esc(label)}</p><p data-tile="${esc(label)}" class="text-2xl font-bold text-gray-900 dark:text-white">${esc(value)}</p><p class="mt-1 ${CLS.muted}">${sub}</p></div>`;
  const m = d.money || {};
  const p = d.profit || {};
  const ap = d.approvals || {};
  const tiles = st ? [
    tile('Rank', `${st.rank} of ${st.of}`, st.we_lead ? `leading ${esc(st.runner_up ?? '')}` : `leader ${esc(st.leader)}${st.leader_changed ? ' (new)' : ''}`),
    tile('Share of the leader', pct(st.share), esc(pts(st.share_change))),
    tile('Vehicles', `${st.vehicles} vs ${st.largest_fleet}`, `largest fleet: ${esc(st.largest_fleet_name || '–')}`),
    tile('Operating profit (quarter)', num(p.quarter), typeof p.change === 'number' ? `${p.change >= 0 ? '+' : ''}${num(p.change)} on the quarter before` : 'from replan'),
    tile('Cash / loan', `${num(m.cash)} / ${num(m.loan)}`, [m.idle_cash != null ? `idle cash ${num(m.idle_cash)} (gap watch)` : `max loan ${num(m.max_loan)}`,
      m.reinvestment != null ? `reinvested ${pct(m.reinvestment)} of cash flow (capital watch)` : ''].filter(Boolean).join('; ')),
    tile('Approvals waiting', String(ap.pending ?? 0), ap.soonest_days != null ? `first default in ${ap.soonest_days} game day(s)` : ''),
  ] : [tile('Rank', '–', 'the company is not among the companies')];
  return `<div class="grid gap-4 mb-4 sm:grid-cols-2 xl:grid-cols-3">${tiles.join('')}</div>`;
}

/** A chart card: title, note, the maximize button's place, the chart box. */
export function chartCard(id, title, note, label) {
  return `<div id="${id}-card" data-maximize="${id}" data-chart="${id}" class="${CLS.card} flex flex-col">
    ${head(title, '', ' ')}
    <p id="${id}-note" class="mb-3 ${UI.cardSub}">${esc(note)}</p>
    <div id="${id}" role="img" aria-label="${esc(label)}" class="w-full h-72 grow"></div></div>`;
}

/** One governance change as a line: setting, from, to, via, and who (credential, Origin, UA, peer). */
export function governanceLine(g) {
  const who = [['credential', 'credential'], ['origin', 'Origin'], ['user_agent', 'UA'], ['peer', 'peer']]
    .filter(([k]) => g[k] != null && g[k] !== '').map(([k, label]) => `${label} ${g[k]}`);
  return `${g.setting ?? g.kind ?? 'governance'} ${g.from ?? '?'} -> ${g.to ?? '?'} via ${g.via ?? '?'}${g.game_date ? ` on ${g.game_date}` : ''}${who.length ? ` (${who.join(', ')})` : ''}`;
}

/** What failed or was skipped, and who changed governance in the period. */
export function failedHtml(d) {
  if (!d) return '';
  const gov = d.governance || [];
  return card(`${head('Failed or skipped', 'From the watchdog, replan, the panel services, the approval deadline and the signal keys.')}${list(d.failed || [], 'Nothing failed or was skipped.')}
    <h3 data-governance class="mt-4 mb-2 text-base font-semibold text-gray-900 dark:text-white">Who changed governance</h3>${list(gov.map(governanceLine), 'Nobody changed governance (auto-approve) in this period.')}
    ${evaluationHtml(d.evaluation)}`);
}

/** What the service evaluation found (eval.poor, eval.coverage), with its caveat. */
export function evaluationHtml(ev) {
  const h = '<h3 data-evaluation class="mt-4 mb-2 text-base font-semibold text-gray-900 dark:text-white">Service evaluation</h3>';
  if (!ev) return `${h}<p class="${CLS.muted}">No service evaluation (eval.poor, eval.coverage): service_eval has not run.</p>`;
  const poor = (ev.poor || []).map((x) => `${x.entry} is persistently ${x.verdict}${x.streak != null ? ` (streak ${x.streak})` : ''}`);
  const cov = (ev.coverage || []).map((c) => `${c.service} reacted to ${c.reacted} of ${c.episodes} deterioration episodes${c.share != null ? ` (${(c.share * 100).toFixed(1)}%)` : ''}`);
  return `${h}<p class="mb-2 ${CLS.muted}">${esc(ev.caveat || 'associative, not causal')}</p>${list([...poor, ...cov], 'Nothing poor, no coverage measured.')}`;
}

/** What the system proposed or changed and why, and every approval waiting with its countdown. */
export function changesHtml(d) {
  if (!d) return '';
  const c = d.changes || {};
  const ap = d.approvals || {};
  const since = ap.since || {};
  const rows = (ap.items || []).map((x) => tr([`#${esc(x.id)}`, esc(x.what), `${esc(x.age_days)} d`,
    x.countdown_days == null ? 'no deadline' : `${esc(x.countdown_days)} d`, x.verdict ? badge(x.verdict, x.verdict === 'approve' ? 'green' : 'gray') : '–'], ` data-approval="${esc(x.id)}"`));
  const proposed = (c.proposed || []).length ? `${c.proposed.length} setting(s): ${(c.proposed || []).slice(0, 12).join(', ')}${c.proposed.length > 12 ? ', …' : ''}` : 'nothing';
  return card(`${head('Changes and approvals', `What replan proposed (${esc(c.date || 'no replan yet')}) and what waits for you. ${link('/', 'Approve in the control window')}`)}
    <dl class="mb-4 text-sm text-gray-700 dark:text-gray-300 grid gap-1">
      <div><dt class="inline font-semibold">Verdict:</dt> <dd class="inline">${c.adopt ? 'switch' : 'hold'}${c.why ? `: ${esc(c.why)}` : ''}</dd></div>
      <div><dt class="inline font-semibold">Proposed:</dt> <dd class="inline">${esc(proposed)}</dd></div>
      <div><dt class="inline font-semibold">Portfolio:</dt> <dd class="inline">${esc((c.portfolio || []).join('; ') || '–')}</dd></div>
      <div><dt class="inline font-semibold">This period:</dt> <dd class="inline">${esc(since.approved ?? 0)} approved, ${esc(since.denied ?? 0)} denied, ${esc(since.by_deadline ?? 0)} settled by the deadline</dd></div>
    </dl>
    ${table(['Approval', 'What', 'Waits', 'Countdown', 'Then'], rows, 'Nothing waits for your approval.')}`);
}

/** The history of one cadence, newest first; the row of `focus` (a label) marked. */
export function historyHtml(history, period, focus) {
  const rows = [...(history?.[period] || [])].reverse().map((r) => tr([esc(r.label), badge(r.level, levelTone(r.level)),
    r.rank == null ? '–' : `${esc(r.rank)}/${esc(r.of)}`, pct(r.share), esc(r.leader ?? '–'), `${num(r.vehicles)} / ${num(r.largest_fleet)}`,
    num(r.profit), num(r.cash), num(r.loan), esc(r.pending ?? 0), esc((r.alerts || []).join(', '))],
  ` data-label="${esc(r.label)}"${r.label === focus ? ' aria-current="true" class="bg-primary-50 dark:bg-gray-700"' : ''}`));
  const select = `<label for="h-period" class="sr-only">Period</label><select id="h-period" class="${UI.select}">${PERIODS.map((p) => `<option value="${p}"${p === period ? ' selected' : ''}>${p}</option>`).join('')}</select>`;
  return card(`${head('History', 'One row per digest of the cadence.', select)}${table(['Period', 'Level', 'Rank', 'Share', 'Leader', 'Vehicles / largest', 'Op. profit', 'Cash', 'Loan', 'Waiting', 'Alerts'], rows, `No ${period} digest yet.`)}`);
}

/* ---- the page ------------------------------------------------------------------------------- */

let shareChart = null;
let typesChart = null;
let last = null;
// Whether the chart cards are in the page (drawn once per mount; a reload only redraws the charts).
let carded = false;

function drawCharts(d) {
  if (!d) return;
  shareChart = shareChart || ecInit($('share-chart'), charts());
  if (shareChart) {
    shareChart.setOption(shareOption(d.series || []), true);
    if (!shareChart._wired) {
      shareChart._wired = true;
      shareChart.on('click', (p) => { if (p.data?.month) focusMonth(p.data.month); });
    }
  }
  typesChart = typesChart || ecInit($('types-chart'), charts());
  if (typesChart) {
    typesChart.setOption(typesOption(d.by_type || []), true);
    if (!typesChart._wired) {
      typesChart._wired = true;
      typesChart.on('click', () => { location.href = `/api/rhai/shared?key=replan.${last?.company ?? 0}`; });
    }
  }
}

/** A month clicked on the share chart: the month history, its row marked and in view. */
export function focusMonth(month) {
  setState({ period: 'month' });
  $('history').innerHTML = historyHtml(last?.history, 'month', month);
  wireHistory();
  document.querySelector(`#history tr[data-label="${month}"]`)?.scrollIntoView?.({ block: 'center' });
}

function wireHistory() {
  $('h-period').onchange = (e) => {
    setState({ period: e.target.value });
    $('history').innerHTML = historyHtml(last?.history, e.target.value);
    wireHistory();
  };
}

/** One read of GET /api/digest and the page drawn from it. */
export async function load(state = getState()) {
  const q = state.company === undefined ? '' : `?company=${state.company}`;
  const a = await api('/api/digest' + q, undefined, { shownIn: 'head' });
  if (!a.ok) return a;
  last = a;
  const d = a.digest;
  $('head').innerHTML = headHtml(a);
  $('standing').innerHTML = standingHtml(d);
  if (!carded) {
    carded = true;
    $('share').innerHTML = chartCard('share-chart', 'Share of the leader', 'Our company value against the leader\'s, by game month; the digest escalates below 50% and 25% and when it falls two quarters running. Click a month for its row.', 'Our company value as a percent of the leader\'s, by game month');
    $('types').innerHTML = chartCard('types-chart', 'Profit per vehicle type', 'Last year\'s profit per vehicle, ours against the best company of each type (replan.<company>). Click for the source.', 'Profit per vehicle by transport type, ours against the leader of the type');
    for (const id of ['share-chart', 'types-chart']) {
      const box = $(`${id}-card`);
      box.querySelector('[data-toolbar]').append(maximizable(box, id, { getState, setState, onResize: () => drawCharts(last?.digest) }));
    }
  }
  $('types-chart-note').textContent = d?.by_type_source === 'replan'
    ? `Last year's profit per vehicle, ours against the best company of each type, from replan on ${d.by_type_date}. Click for the source.`
    : `Unknown: ${d?.by_type_source ?? 'no digest'} (replan.<company> missing or older than two game months).`;
  $('failed').innerHTML = failedHtml(d);
  $('changes').innerHTML = changesHtml(d);
  $('history').innerHTML = historyHtml(a.history, state.period);
  wireHistory();
  drawCharts(d);
  return a;
}

/** The whole page; exported so a test mounts it in the fake browser (test/digest.test.mjs). */
export async function mount() {
  shareChart = null;
  typesChart = null;
  carded = false;
  await load(getState());
  setInterval(() => load(getState()), 15000);
}

if (globalThis.document && $('head') && $('history')) mount();
