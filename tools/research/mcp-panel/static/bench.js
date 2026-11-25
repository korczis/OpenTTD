/* Rust performance benchmarks in the panel. /bench is every bench of one record (the newest, or
   the one the address names with ?rec=): the regressions against its baseline, the record's rev,
   host and toolchain, a table filtered by group and text and sorted by any column, and every
   bench's median across the records relative to its first. /bench/<group>/<name> is one bench
   across every record: its median with the min-max band and the regressions marked, and a row per
   record. Records come from GET /api/bench (research/results/bench-*.json, schema rust-bench/v1,
   src/panel_bench.rs). Charts go through the one door (components.js ecInit), are rows of CHARTS,
   keep their parameters in the address and maximize (Escape restores). Every bench name links to
   its page, every rev to its commit. */

import { ecInit, maximizable, chartCard, PALETTE, UI, CLS, $, esc, api, INK, AXIS as axis, patchAddress, badge } from './components.js';

/** The page's echarts, read when a chart is drawn (a test installs its fake after importing). */
const echartsNow = () => globalThis.echarts;

/** The registry: every chart of the bench pages, the address keys it reads and where a click goes. */
export const CHARTS = [
  { id: 'bench-trend', page: '/bench', kind: 'line', params: ['group', 'q', 'rec', 'max'], drill: '/bench/<group>/<name>' },
  { id: 'bench-history', page: '/bench/<group>/<name>', kind: 'line+band+scatter', params: ['rec', 'max'], drill: 'the clicked record (?rec=), then /bench?rec=<record>' },
];

/** The summary table's columns; `num` sorts as a number. */
export const COLUMNS = [
  { key: 'group', label: 'Group' },
  { key: 'name', label: 'Bench' },
  { key: 'median', label: 'Median', num: true },
  { key: 'stddev', label: '± stddev', num: true },
  { key: 'samples', label: 'Samples', num: true },
  { key: 'change', label: 'vs previous', num: true, title: 'Against the newest earlier record of the same host and crate' },
];

/* ---- address state ------------------------------------------------------------------------- */
export function getState(href = globalThis.location?.href) {
  const p = new URL(href).searchParams;
  const sort = p.get('sort') || '';
  const known = COLUMNS.some((c) => c.key === sort.replace(/^-/, ''));
  return { group: p.get('group') || '', q: p.get('q') || '', sort: known ? sort : 'group', rec: p.get('rec') || '', max: p.get('max') || undefined };
}
export function setState(patch) { patchAddress(patch); }

/* ---- units, names and links (pure) --------------------------------------------------------- */
const SCALE = [[1e9, 's'], [1e6, 'ms'], [1e3, 'µs'], [1, 'ns']];
/** The unit a duration in nanoseconds reads best in: {div, unit}. */
export function scaleOf(ns) {
  const a = Math.abs(Number(ns) || 0);
  const [div, unit] = SCALE.find(([d]) => a >= d) || SCALE.at(-1);
  return { div, unit };
}
/** A duration as a person reads it (950 ns, 12.3 µs, 4.56 ms); another unit as it is. */
export function human(v, unit = 'ns') {
  if (v === null || v === undefined || !Number.isFinite(Number(v))) return '-';
  if (unit !== 'ns') return `${v} ${unit}`;
  const { div, unit: u } = scaleOf(v);
  const x = Number(v) / div;
  return `${x.toFixed(Math.abs(x) >= 100 ? 0 : Math.abs(x) >= 10 ? 1 : 2)} ${u}`;
}
/** A ratio of two medians as a change: 1.264 -> "+26.4%". */
export const pct = (ratio) => (ratio === null || ratio === undefined ? '-' : `${ratio >= 1 ? '+' : ''}${((ratio - 1) * 100).toFixed(1)}%`);
/** A bench's own page: /bench/<group>/<name>, each step escaped. */
export const benchHref = (id) => '/bench/' + String(id).split('/').map(encodeURIComponent).join('/');
/** The record's own view on the list page. */
export const recordHref = (id) => '/bench?rec=' + encodeURIComponent(id);
/** A rev's commit page in the repository's web view (GET /api/bench `repo_url`, derived from its
 *  origin remote), or null when the panel knows none. */
export const commitUrl = (rev, repo) => (repo ? `${repo}/commit/${encodeURIComponent(rev)}` : null);
/** A rev, linked to its commit when there is a commit page. */
const revLink = (rev, repo, cls = 'hover:underline font-mono') => {
  const url = commitUrl(rev, repo);
  const text = esc(String(rev).slice(0, 10));
  return url ? `<a class="${cls}" href="${esc(url)}" rel="noopener">${text}</a>` : `<span class="font-mono">${text}</span>`;
};
/** 2026-11-22T23:15:00+00:00 -> 2026-11-22 23:15 (the record's own clock). */
export const shortTime = (iso) => {
  const m = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2})/.exec(iso || '');
  return m ? `${m[1]} ${m[2]}` : String(iso || '');
};
/** A bench's throughput ({elements: 64}, {bytes: 1024}, or a number) in words. */
export function throughputText(t) {
  if (t === null || t === undefined) return '';
  if (typeof t !== 'object') return String(t);
  return Object.entries(t).map(([k, v]) => `${v} ${k}`).join(', ');
}
const groupOf = (b) => b.group ?? String(b.id).split('/')[0];
const nameOf = (b) => b.name ?? String(b.id).split('/').slice(1).join('/');

/* ---- records (pure) ------------------------------------------------------------------------ */
/** The record the address picks (rec), else the newest (records are oldest first). */
export const selected = (records, rec) => records.find((r) => r.id === rec) || records.at(-1) || null;
/** Whether two records measured on one machine and one crate, so their numbers compare. */
export const comparable = (a, b) => (a?.host?.name ?? null) === (b?.host?.name ?? null) && (a?.crate ?? null) === (b?.crate ?? null);
/** The newest record before one of the same host and crate, or null: another machine's numbers
 *  are no change of this one's. */
export function previous(records, record) {
  for (let i = records.indexOf(record) - 1; i >= 0; i--) if (comparable(records[i], record)) return records[i];
  return null;
}
/** The groups of a record's benches, sorted. */
export const groups = (record) => [...new Set((record?.benches || []).map(groupOf))].sort();
/** The regressions a record's baseline names. */
export const regressionsOf = (record) => record?.baseline?.regressions || [];

/** The table's rows: every bench of a record with its change against the previous record, kept
 *  by group and text (in its id), sorted by a column (`-` first: descending), then by id. */
export function rows(record, prev, { group = '', q = '', sort = 'group' } = {}) {
  const before = new Map((prev?.benches || []).map((b) => [b.id, b]));
  const regressed = new Set(regressionsOf(record).map((r) => r.id));
  const t = q.trim().toLowerCase();
  const out = (record?.benches || []).map((b) => {
    const p = before.get(b.id);
    return { ...b, group: groupOf(b), name: nameOf(b), change: p && p.median ? b.median / p.median : null, regressed: regressed.has(b.id) };
  }).filter((b) => (!group || b.group === group) && (!t || b.id.toLowerCase().includes(t)));
  const desc = sort.startsWith('-');
  const key = sort.replace(/^-/, '');
  const num = COLUMNS.find((c) => c.key === key)?.num;
  const n = (v) => (v === null || v === undefined ? -Infinity : Number(v));
  out.sort((a, b) => {
    const c = num ? Math.sign(n(a[key]) - n(b[key]) || 0) : String(a[key] ?? '').localeCompare(String(b[key] ?? ''));
    return (desc ? -c : c) || a.id.localeCompare(b.id);
  });
  return out;
}

/** One bench across the records that measured it: [{record, bench, regression}], oldest first. */
export function history(records, id) {
  return records.flatMap((record) => {
    const bench = (record.benches || []).find((b) => b.id === id);
    return bench ? [{ record, bench, regression: regressionsOf(record).find((r) => r.id === id) || null }] : [];
  });
}

/* ---- the chart options (pure: data in, option out) ----------------------------------------- */
const zoom = [{ type: 'inside', zoomOnMouseWheel: 'ctrl', moveOnMouseWheel: false }, { type: 'slider', height: 18, bottom: 4, borderColor: INK.grid }];

/** One bench's median across the records, its min-max band under it, and a red triangle on every
 *  record whose baseline names it a regression; the record in the address is drawn larger. */
export function historyOption(points, { sel = '' } = {}) {
  const unit = points[0]?.bench.unit || 'ns';
  const { div, unit: u } = unit === 'ns' ? scaleOf(Math.max(...points.map((p) => p.bench.median))) : { div: 1, unit };
  const v = (x) => (x === null || x === undefined ? null : Number((x / div).toPrecision(6)));
  const byId = new Map(points.map((p) => [p.record.id, p]));
  const lo = (p) => p.bench.min ?? p.bench.median;
  const hi = (p) => p.bench.max ?? p.bench.median;
  return {
    backgroundColor: 'transparent',
    tooltip: {
      trigger: 'axis',
      formatter: (ps) => {
        const p = byId.get(ps?.[0]?.axisValue);
        if (!p) return '';
        const b = p.bench;
        return [`<b>${esc(p.record.id)}</b>`, `${esc(shortTime(p.record.generated_at))} · rev ${esc(String(p.record.rev).slice(0, 10))}`,
          `median ${esc(human(b.median, unit))} ± ${esc(human(b.stddev, unit))}`, `min–max ${esc(human(lo(p), unit))} – ${esc(human(hi(p), unit))}`,
          p.regression ? `regression ${esc(pct(p.regression.ratio))} against its baseline` : ''].filter(Boolean).join('<br>');
      },
    },
    legend: { top: 0, textStyle: { color: INK.text }, data: ['median', 'min–max', 'regression'] },
    grid: { left: 8, right: 24, top: 52, bottom: 56, containLabel: true },
    xAxis: { type: 'category', data: points.map((p) => p.record.id), ...axis, axisLabel: { color: INK.muted, hideOverlap: true, formatter: (id) => shortTime(byId.get(id)?.record.generated_at) } },
    yAxis: { type: 'value', scale: true, name: u, nameTextStyle: { color: INK.muted, align: 'left' }, ...axis },
    dataZoom: zoom,
    series: [
      { id: 'band-low', name: 'min–max', type: 'line', stack: 'band', silent: true, showSymbol: false, lineStyle: { opacity: 0 }, color: PALETTE.dark[0], data: points.map((p) => v(lo(p))) },
      { id: 'band', name: 'min–max', type: 'line', stack: 'band', silent: true, showSymbol: false, lineStyle: { opacity: 0 }, color: PALETTE.dark[0], areaStyle: { opacity: 0.2 }, data: points.map((p) => v(hi(p) - lo(p))) },
      { id: 'median', name: 'median', type: 'line', cursor: 'pointer', color: PALETTE.dark[0], symbolSize: 8,
        data: points.map((p) => ({ value: v(p.bench.median), name: p.record.id, ...(p.record.id === sel ? { symbolSize: 16, itemStyle: { borderColor: '#fff', borderWidth: 2 } } : {}) })) },
      { id: 'regressions', name: 'regression', type: 'scatter', cursor: 'pointer', color: PALETTE.dark[7], symbol: 'triangle', symbolSize: 18, z: 5,
        data: points.filter((p) => p.regression).map((p) => ({ value: [p.record.id, v(p.bench.median)], name: p.record.id, ratio: p.regression.ratio })) },
    ],
  };
}

/** Every listed bench's median across the records, as a multiple of its first measured median, so
 *  benches of nanoseconds and of milliseconds share one axis; a vertical line marks the record in
 *  the address. A line carries its bench id for the click. */
export function trendOption(records, ids, { sel = '' } = {}) {
  const cats = records.map((r) => r.id);
  const series = ids.map((id, i) => {
    const pts = records.map((r) => (r.benches || []).find((b) => b.id === id) || null);
    const first = pts.find((b) => b && b.median)?.median;
    return {
      id, name: id, type: 'line', cursor: 'pointer', connectNulls: true, showSymbol: records.length < 30, color: PALETTE.dark[i % PALETTE.dark.length],
      endLabel: { show: ids.length <= 8, color: INK.text, formatter: (p) => `${id} ${(p.value ?? 0).toFixed(2)}×` },
      labelLayout: { moveOverlap: 'shiftY' },
      data: pts.map((b) => (b && first ? Number((b.median / first).toFixed(4)) : null)),
      ...(i === 0 && sel ? { markLine: { silent: true, symbol: 'none', label: { show: false }, lineStyle: { color: INK.muted, type: 'dashed' }, data: [{ xAxis: sel }] } } : {}),
    };
  });
  return {
    backgroundColor: 'transparent',
    tooltip: { trigger: 'axis', valueFormatter: (x) => (x === null || x === undefined ? '-' : `${x.toFixed(3)}×`) },
    legend: { type: 'scroll', top: 0, textStyle: { color: INK.text }, pageTextStyle: { color: INK.muted } },
    grid: { left: 8, right: ids.length <= 8 ? 160 : 24, top: 52, bottom: 56, containLabel: true },
    xAxis: { type: 'category', data: cats, ...axis, axisLabel: { color: INK.muted, hideOverlap: true, formatter: (id) => shortTime(records.find((r) => r.id === id)?.generated_at) } },
    yAxis: { type: 'value', scale: true, name: '× first median', nameTextStyle: { color: INK.muted, align: 'left' }, ...axis },
    dataZoom: zoom,
    series,
  };
}

/* ---- markup pieces ------------------------------------------------------------------------- */
const LINK = 'font-medium text-primary-600 hover:underline dark:text-primary-500';
const apiLink = (op) => `<a href="/docs#/default/${op}" class="text-sm text-primary-600 hover:underline dark:text-primary-500">API</a>`;
const stat = (label, value, sub = '', href = '') => `<div class="p-3 rounded-lg bg-gray-50 dark:bg-gray-700/50">
  <dt class="text-xs text-gray-500 dark:text-gray-400">${esc(label)}</dt>
  <dd class="mt-1 text-lg font-semibold text-gray-900 dark:text-white break-words">${href ? `<a class="hover:underline" href="${esc(href)}" ${/^https?:/.test(href) ? 'rel="noopener"' : ''}>${esc(value)}</a>` : esc(value)}</dd>
  ${sub ? `<dd class="text-xs text-gray-500 dark:text-gray-400 break-words">${esc(sub)}</dd>` : ''}</div>`;
const ALERT_ICON = '<svg class="shrink-0 w-4 h-4 mt-0.5 me-3" aria-hidden="true" fill="currentColor" viewBox="0 0 20 20"><path d="M10 .5a9.5 9.5 0 1 0 9.5 9.5A9.51 9.51 0 0 0 10 .5ZM9.5 4a1.5 1.5 0 1 1 0 3 1.5 1.5 0 0 1 0-3ZM12 15H8a1 1 0 0 1 0-2h1v-3H8a1 1 0 0 1 0-2h2a1 1 0 0 1 1 1v4h1a1 1 0 0 1 0 2Z"/></svg>';
/** A Flowbite alert: red for a regression, yellow for a warning. */
const callout = (tone, html) => `<div role="alert" data-alert="${tone}" class="flex items-start p-4 mb-4 text-sm rounded-lg border ${tone === 'red'
  ? 'text-red-800 border-red-300 bg-red-50 dark:bg-gray-800 dark:text-red-400 dark:border-red-800'
  : 'text-yellow-800 border-yellow-300 bg-yellow-50 dark:bg-gray-800 dark:text-yellow-300 dark:border-yellow-800'}">${ALERT_ICON}<div class="min-w-0">${html}</div></div>`;

/** The regressions of a record against its baseline, each linked to its bench; empty without any. */
export function regressionsHtml(record) {
  const regs = regressionsOf(record);
  if (!regs.length) return '';
  const unit = (id) => (record.benches || []).find((b) => b.id === id)?.unit || 'ns';
  const base = record.baseline?.id;
  return callout('red', `<span class="font-medium">${regs.length} regression${regs.length === 1 ? '' : 's'} against the baseline${base ? ` <a class="underline" href="${esc(recordHref(base))}">${esc(base)}</a>` : ''}</span>
    <ul class="mt-1.5 list-disc list-inside">${regs.map((r) => `<li><a class="font-semibold underline" href="${esc(benchHref(r.id))}">${esc(r.id)}</a>: median ${esc(human(r.median, unit(r.id)))} where the baseline had ${esc(human(r.baseline_median, unit(r.id)))}, ${esc(Number(r.ratio).toFixed(3))}× (${esc(pct(r.ratio))})</li>`).join('')}</ul>`);
}

/** The record's provenance: rev (linked to its commit), host, toolchain, crate, baseline; a dirty
 *  tree is a warning, since the numbers are then not of the rev alone. */
export function recordHtml(record, { narrow = false, repo = null } = {}) {
  const h = record.host || {};
  const base = record.baseline?.id;
  const regs = regressionsOf(record);
  return `${record.tree_dirty ? callout('yellow', `<span class="font-medium">Dirty tree.</span> This record was measured with uncommitted changes on top of rev ${esc(String(record.rev).slice(0, 10))}: its numbers are not of that commit alone.`) : ''}
    <dl class="grid gap-3 ${narrow ? 'grid-cols-2 lg:grid-cols-3 2xl:grid-cols-2' : 'grid-cols-2 lg:grid-cols-3 2xl:grid-cols-6'}">
      ${stat('Revision', String(record.rev).slice(0, 10), record.tree_dirty ? 'tree dirty' : 'clean tree', commitUrl(record.rev, repo) || '')}
      ${stat('Generated', shortTime(record.generated_at), record.generated_at)}
      ${stat('Host', h.name || '-', [h.os, h.cpus ? `${h.cpus} CPUs` : ''].filter(Boolean).join(' · '))}
      ${stat('Toolchain', record.toolchain || '-', `profile ${record.profile || '-'}`)}
      ${stat('Crate', record.crate || '-', `${(record.benches || []).length} benches`)}
      ${stat('Baseline', base || 'none', base ? `${regs.length} regression${regs.length === 1 ? '' : 's'}` : 'the first record of its series', base ? recordHref(base) : '')}
    </dl>`;
}

/** The page when GET /api/bench failed: what it answered (the kit's toast says it too). */
export const loadError = (d) => `<div id="bench-error" class="${CLS.card}">The benchmark records did not load: ${esc(d.error || `HTTP ${d.status}`)}.</div>`;

/** The page when no record exists yet, with every file the panel refused and why. */
export function emptyHtml(d) {
  const skipped = d.skipped || [];
  return `<div id="bench-empty" class="${CLS.card}">
    <div class="${UI.cardHead} mb-3"><h2 class="${UI.cardHeadTitle} ${UI.cardTitle}">No benchmark record yet</h2>${apiLink('bench')}</div>
    <p class="text-sm text-gray-700 dark:text-gray-300">The Rust benchmark harness writes one record per run to <code>${esc(d.dir || 'research/results')}/bench-*.json</code> (schema <code>rust-bench/v1</code>). This page shows every bench of the newest record, its history and its regressions as soon as one is there.</p>
    ${skippedHtml(skipped)}</div>`;
}

/** The files that are not records, with the reason (a list, or nothing). */
export function skippedHtml(skipped) {
  if (!skipped?.length) return '';
  return `<div id="bench-skipped" class="mt-4">${callout('yellow', `<span class="font-medium">${skipped.length} file${skipped.length === 1 ? '' : 's'} skipped: not a rust-bench/v1 record</span>
    <ul class="mt-1.5 list-disc list-inside">${skipped.map((s) => `<li><code>${esc(s.file)}</code>: ${esc(s.error)}</li>`).join('')}</ul>`)}</div>`;
}

/* ---- /bench -------------------------------------------------------------------------------- */
/** /bench; exported so a test can mount it in a fresh document (test/bench.test.mjs). */
export async function listPage() {
  const state = getState();
  const d = await api('/api/bench');
  if (!d.ok) { $('page').innerHTML = loadError(d); return; }
  const records = d.records || [];
  if (!records.length) { $('page').innerHTML = emptyHtml(d); return; }
  $('page').innerHTML = `
    <div id="bench-regressions" aria-live="polite"></div>
    <div id="bench-record-card" class="${CLS.card}">
      <div class="${UI.cardHead} mb-3">
        <div class="${UI.cardHeadTitle}"><h2 class="${UI.cardTitle}">Record <span id="bench-record-id" class="${UI.cardSub}"></span></h2><p class="mt-2 ${UI.cardSub}">${esc(records.length)} records in ${esc(d.dir || 'research/results')}; pick one to see its benches</p></div>
        <div class="flex flex-wrap items-center gap-2"><label for="rec" class="sr-only">Record</label>
          <select id="rec" class="${UI.select}">${records.slice().reverse().map((r) => `<option value="${esc(r.id)}">${esc(shortTime(r.generated_at))} · ${esc(String(r.rev).slice(0, 8))}${r.id === d.latest_id ? ' (latest)' : ''}</option>`).join('')}</select>
          ${apiLink('bench')}</div>
      </div>
      <div id="bench-record"></div>
    </div>
    <div class="${CLS.card}">
      <div class="${UI.cardHead} mb-3">
        <h2 class="${UI.cardHeadTitle} ${UI.cardTitle}">Benches <span id="bench-n" class="${UI.cardSub}"></span></h2>
        <div class="flex flex-wrap items-center gap-2">
          <label for="group" class="sr-only">Group</label><select id="group" class="${UI.select}"></select>
          <label for="q" class="sr-only">Filter</label><input id="q" type="search" value="${esc(state.q)}" placeholder="filter by name" class="${UI.input} w-full sm:w-56">
        </div>
      </div>
      <div class="${UI.tableWrap}"><table class="${UI.table}">
        <thead class="${UI.thead}"><tr id="bench-head"></tr></thead>
        <tbody id="bench-rows"></tbody></table></div>
    </div>
    ${chartCard('bench-trend', 'Every bench across the records', 'Median as a multiple of its first record: above 1 is slower; click a line for its bench, Ctrl+wheel zooms', 'The median of every listed bench across the records, relative to its first record')}
    ${skippedHtml(d.skipped)}`;

  let chart = null;
  const record = () => selected(records, state.rec);
  const shown = () => rows(record(), previous(records, record()), state);
  const draw = () => {
    chart = chart || ecInit($('bench-trend'), echartsNow());
    if (!chart) return;
    chart.setOption(trendOption(records, shown().map((b) => b.id), { sel: record().id }), true);
    if (!chart._wired) { chart._wired = true; chart.on('click', (p) => { if (p.seriesId) location.href = benchHref(p.seriesId); }); }
    chart.resize();
  };
  const head = () => {
    const key = state.sort.replace(/^-/, '');
    const desc = state.sort.startsWith('-');
    $('bench-head').innerHTML = COLUMNS.map((c) => {
      const on = c.key === key;
      return `<th scope="col" class="px-3 py-2${c.num ? ' text-right' : ''}"${on ? ` aria-sort="${desc ? 'descending' : 'ascending'}"` : ''}><button type="button" data-sort="${c.key}" title="Sort by ${esc(c.label)}${c.title ? `: ${esc(c.title)}` : ''}" class="inline-flex items-center gap-1 uppercase hover:text-gray-900 dark:hover:text-white">${esc(c.label)}<span aria-hidden="true">${on ? (desc ? '▼' : '▲') : '↕'}</span></button></th>`;
    }).join('');
  };
  const table = () => {
    const list = shown();
    $('bench-n').textContent = `(${list.length} of ${(record().benches || []).length})`;
    $('bench-rows').innerHTML = list.map((b) => `<tr class="border-b dark:border-gray-700${b.regressed ? ' bg-red-50 dark:bg-red-900/20' : ''}" data-bench="${esc(b.id)}">
      <td class="px-3 py-2"><a href="?group=${encodeURIComponent(b.group)}" data-group="${esc(b.group)}" class="hover:underline">${esc(b.group)}</a></td>
      <td class="px-3 py-2 whitespace-nowrap"><a href="${esc(benchHref(b.id))}" class="${LINK}">${esc(b.name)}</a>${b.regressed ? ' ' + badge('regression', 'red') : ''}${b.throughput ? `<span class="block text-xs text-gray-500 dark:text-gray-400">${esc(throughputText(b.throughput))} per iteration</span>` : ''}</td>
      <td class="px-3 py-2 text-right font-medium text-gray-900 dark:text-white whitespace-nowrap">${esc(human(b.median, b.unit))}</td>
      <td class="px-3 py-2 text-right whitespace-nowrap">± ${esc(human(b.stddev, b.unit))}</td>
      <td class="px-3 py-2 text-right">${esc(b.samples ?? '-')}</td>
      <td class="px-3 py-2 text-right whitespace-nowrap">${b.change === null ? '-' : badge(pct(b.change), b.change > 1.05 ? 'red' : b.change < 0.95 ? 'green' : 'gray')}</td></tr>`).join('')
      || `<tr><td class="px-3 py-2" colspan="${COLUMNS.length}">no bench matches the filter</td></tr>`;
  };
  const groupSelect = () => {
    const gs = groups(record());
    if (state.group && !gs.includes(state.group)) gs.push(state.group);
    $('group').innerHTML = `<option value="">every group</option>${gs.map((g) => `<option value="${esc(g)}"${g === state.group ? ' selected' : ''}>${esc(g)}</option>`).join('')}`;
  };
  const showRecord = () => {
    const r = record();
    $('rec').value = r.id;
    $('bench-record-id').textContent = r.id;
    $('bench-record').innerHTML = recordHtml(r, { repo: d.repo_url });
    $('bench-regressions').innerHTML = regressionsHtml(r);
    groupSelect();
  };
  const all = () => { showRecord(); head(); table(); draw(); };

  $('rec').onchange = (e) => { state.rec = e.target.value === d.latest_id ? '' : e.target.value; setState({ rec: state.rec }); all(); };
  $('group').onchange = (e) => { state.group = e.target.value; setState({ group: state.group }); table(); draw(); };
  $('q').oninput = (e) => { state.q = e.target.value; setState({ q: state.q }); table(); draw(); };
  $('bench-head').addEventListener('click', (e) => {
    const k = e.target?.closest?.('[data-sort]')?.dataset.sort;
    if (!k) return;
    state.sort = state.sort === k ? `-${k}` : k;
    setState({ sort: state.sort === 'group' ? '' : state.sort });
    head(); table();
  });
  $('bench-rows').addEventListener('click', (e) => {
    const g = e.target?.dataset?.group;
    if (g === undefined) return;
    e.preventDefault?.();
    state.group = g; setState({ group: g }); groupSelect(); table(); draw();
  });
  const box = $('bench-trend-card');
  box.querySelector('[data-toolbar]').append(maximizable(box, 'bench-trend', { getState, setState, onResize: () => draw() }));
  showRecord(); head(); table();
  setTimeout(draw, 30);
}

/* ---- /bench/<group>/<name> ----------------------------------------------------------------- */
/** /bench/<id>; exported for the same reason. */
export async function benchPage(id) {
  const state = getState();
  const d = await api('/api/bench');
  if (!d.ok) { $('page').innerHTML = loadError(d); return; }
  const records = d.records || [];
  const points = history(records, id);
  if (!points.length) {
    $('page').innerHTML = `<div class="${CLS.card}">No record measured the bench <code>${esc(id)}</code>. <a class="${LINK}" href="/bench">All benches</a>.</div>`;
    return;
  }
  $('crumbs')?.insertAdjacentHTML('beforeend', `<li class="text-gray-400">/</li><li class="text-gray-500 dark:text-gray-400">${esc(id)}</li>`);
  const group = groupOf(points[0].bench);
  const siblings = [...new Set(records.flatMap((r) => (r.benches || []).map((b) => b.id)))].filter((x) => x !== id && groupOf({ id: x }) === group).sort();
  $('page').innerHTML = `
    <div id="bench-regression" aria-live="polite"></div>
    <dl id="bench-stats" class="grid grid-cols-2 gap-3 mb-4 md:grid-cols-4"></dl>
    ${chartCard('bench-history', 'Median across the records', 'The band is each record\'s fastest to slowest sample, a red triangle a regression against the baseline; click a record to select it, Ctrl+wheel zooms', `The median of ${id} across the records, with its min-max band and the regressions marked`)}
    <div class="grid gap-4 2xl:grid-cols-3">
      <div class="${CLS.card} 2xl:col-span-2">
        <div class="${UI.cardHead} mb-3"><h2 class="${UI.cardHeadTitle} ${UI.cardTitle}">Every record <span class="${UI.cardSub}">(${esc(points.length)})</span></h2>${apiLink('bench')}</div>
        <div class="${UI.tableWrap}"><table class="${UI.table}">
          <thead class="${UI.thead}"><tr><th class="px-3 py-2">Record</th><th class="px-3 py-2">Rev</th><th class="px-3 py-2 text-right">Median</th><th class="px-3 py-2 text-right">± stddev</th><th class="px-3 py-2 text-right">Min – max</th><th class="px-3 py-2 text-right">Samples</th><th class="px-3 py-2">Baseline</th></tr></thead>
          <tbody id="bench-records"></tbody></table></div>
      </div>
      <div class="${CLS.card}">
        <h2 class="mb-3 ${UI.cardTitle}">Selected record</h2>
        <div id="bench-selected" aria-live="polite"></div>
        <h3 class="mt-4 mb-2 text-sm font-semibold text-gray-900 dark:text-white">Same group: ${esc(group)}</h3>
        <ul id="bench-siblings" class="space-y-1 text-sm">${siblings.map((s) => `<li><a class="${LINK}" href="${esc(benchHref(s))}">${esc(s)}</a></li>`).join('') || '<li class="text-gray-500 dark:text-gray-400">no other bench in this group</li>'}</ul>
        <a class="inline-block mt-3 text-sm ${LINK}" href="/bench?group=${encodeURIComponent(group)}">The group on the summary</a>
      </div>
    </div>`;

  let chart = null;
  const point = () => points.find((p) => p.record.id === state.rec) || points.at(-1);
  const draw = () => {
    chart = chart || ecInit($('bench-history'), echartsNow());
    if (!chart) return;
    chart.setOption(historyOption(points, { sel: point().record.id }), true);
    if (!chart._wired) {
      chart._wired = true;
      chart.on('click', (p) => { const rec = p.data?.name || p.name; if (rec && points.some((x) => x.record.id === rec)) select(rec); });
    }
    chart.resize();
  };
  const show = () => {
    const p = point();
    const b = p.bench;
    const i = points.indexOf(p);
    const prior = points.slice(0, i).reverse().find((x) => comparable(x.record, p.record));
    const before = prior ? prior.bench : null;
    const change = before && before.median ? b.median / before.median : null;
    $('bench-stats').innerHTML = [
      stat('Median', human(b.median, b.unit), `mean ${human(b.mean, b.unit)} · ± ${human(b.stddev, b.unit)}`),
      stat('Min – max', `${human(b.min, b.unit)} – ${human(b.max, b.unit)}`, `${b.samples ?? '-'} samples${b.throughput ? ' · ' + throughputText(b.throughput) : ''}`),
      stat('vs previous record', pct(change), before ? `was ${human(before.median, before.unit)} on ${prior.record.host?.name || 'the same host'}` : 'the first record of this host and crate that measured it'),
      stat('Record', shortTime(p.record.generated_at), p.record.id, recordHref(p.record.id)),
    ].join('');
    $('bench-regression').innerHTML = p.regression ? regressionsHtml({ ...p.record, baseline: { ...p.record.baseline, regressions: [p.regression] } }) : '';
    $('bench-selected').innerHTML = `${recordHtml(p.record, { narrow: true, repo: d.repo_url })}<a class="inline-block mt-3 text-sm ${LINK}" href="${esc(recordHref(p.record.id))}">Every bench of this record</a>`;
    $('bench-records').innerHTML = points.slice().reverse().map((x) => `<tr class="border-b dark:border-gray-700${x === p ? ' bg-gray-100 dark:bg-gray-700' : ''}"${x === p ? ' aria-current="true"' : ''}>
      <td class="px-3 py-2 whitespace-nowrap"><a href="?rec=${encodeURIComponent(x.record.id)}" data-rec="${esc(x.record.id)}" class="${LINK}">${esc(shortTime(x.record.generated_at))}</a></td>
      <td class="px-3 py-2">${revLink(x.record.rev, d.repo_url)}${x.record.tree_dirty ? ' ' + badge('dirty', 'yellow') : ''}</td>
      <td class="px-3 py-2 text-right font-medium text-gray-900 dark:text-white whitespace-nowrap">${esc(human(x.bench.median, x.bench.unit))}</td>
      <td class="px-3 py-2 text-right whitespace-nowrap">± ${esc(human(x.bench.stddev, x.bench.unit))}</td>
      <td class="px-3 py-2 text-right whitespace-nowrap">${esc(human(x.bench.min, x.bench.unit))} – ${esc(human(x.bench.max, x.bench.unit))}</td>
      <td class="px-3 py-2 text-right">${esc(x.bench.samples ?? '-')}</td>
      <td class="px-3 py-2">${x.regression ? badge(`regression ${pct(x.regression.ratio)}`, 'red') : x.record.baseline?.id ? badge('ok', 'green') : badge('no baseline', 'gray')}</td></tr>`).join('');
  };
  function select(rec) {
    state.rec = rec === points.at(-1).record.id ? '' : rec;
    setState({ rec: state.rec });
    show(); draw();
  }
  $('bench-records').addEventListener('click', (e) => {
    const rec = e.target?.dataset?.rec;
    if (!rec) return;
    e.preventDefault?.();
    select(rec);
  });
  const box = $('bench-history-card');
  box.querySelector('[data-toolbar]').append(maximizable(box, 'bench-history', { getState, setState, onResize: () => draw() }));
  show();
  setTimeout(draw, 30);
}

if (globalThis.document && $('page')) {
  const path = location.pathname;
  if (path === '/bench') listPage();
  else if (path.startsWith('/bench/')) benchPage(path.slice('/bench/'.length).split('/').map(decodeURIComponent).join('/'));
}
