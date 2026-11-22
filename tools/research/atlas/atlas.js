// RESEARCH-ONLY. The Research Atlas page of the published site (surfaces.toml [[pages.items]] atlas).
// It draws data/atlas.json (research-atlas/v1, written by tools/research/atlas/atlas data) and
// nothing else: every number, label, class and link on the page comes from that file, and that file
// comes from research/results/*.json and the README's own diagrams. The page is read-only: no
// control here reaches a game, a panel or an MCP server.
//
// Views, each a section of one page and each a model the reader drives:
//   explorer  every duel and A/B record: filter, select, compare A with B, open the evidence
//   timeline  one seed and length over time: each record's planned and rival value, steps down kept
//   battle    the newest LAN battle's leaderboard and every battle record's headline over time
//   graph     the README's architecture diagram as a graph: select a part, filter edge kinds
//   process   the approval diagram stepped through in order: play, pause, replay, step
// All state lives in the URL (?view=&class=&seed=&sel=&node=&kinds=&step=...), so a view is a link.
// Every chart is created through ECharts and registered with the site's chart registry
// (window.surfaces.charts), so it resizes, maximizes (button, ?max=) and restores on Escape.
(function () {
  'use strict';

  // ------------------------------------------------------------------ pure core (node --test)

  const FILTERS = ['kind', 'class', 'seed', 'months', 'opponent', 'start', 'confounder', 'q'];
  const VIEWS = ['explorer', 'timeline', 'battle', 'graph', 'process'];

  /** The page state from URL parameters; unknown or absent values fall back to the defaults. */
  function readState(params) {
    const get = (k) => (params.get(k) || '').trim();
    const kinds = get('kinds');
    return {
      view: VIEWS.includes(get('view')) ? get('view') : '',
      kind: get('kind'), class: get('class'), seed: get('seed'), months: get('months'),
      opponent: get('opponent'), start: get('start'), confounder: get('confounder'), q: get('q'),
      sel: get('sel'), scale: get('scale') === 'linear' ? 'linear' : 'log',
      group: get('group'), graph: get('graph'), node: get('node'),
      kinds: kinds ? kinds.split(',').filter(Boolean) : null, step: Number(get('step') || 0) || 0,
    };
  }

  /** The rows that pass every filter of `st`; an empty filter passes everything. */
  function filterRows(rows, st) {
    const q = (st.q || '').toLowerCase();
    return rows.filter((r) => (!st.kind || r.kind === st.kind)
      && (!st.class || r.class === st.class)
      && (!st.seed || String(r.seed) === st.seed)
      && (!st.months || String(r.months) === st.months)
      && (!st.opponent || (r.opponent || 'not recorded') === st.opponent)
      && (!st.start || (st.start === 'held') === Boolean(r.held_start))
      && (!st.confounder || (st.confounder === 'none' ? !r.confounders.length : r.confounders.includes(st.confounder)))
      && (!q || (r.id + ' ' + r.label + ' ' + (r.bot_settings || '')).toLowerCase().includes(q)));
  }

  /** Every value a filter can take, counted over all rows, in a stable order. */
  function facets(rows) {
    const tally = (fn) => {
      const m = new Map();
      rows.forEach((r) => [].concat(fn(r)).forEach((v) => m.set(String(v), (m.get(String(v)) || 0) + 1)));
      return [...m.entries()].sort((a, b) => (isFinite(a[0]) && isFinite(b[0]) ? a[0] - b[0] : a[0].localeCompare(b[0])));
    };
    return {
      kind: tally((r) => r.kind), class: tally((r) => r.class), seed: tally((r) => r.seed),
      months: tally((r) => r.months), opponent: tally((r) => r.opponent || 'not recorded'),
      start: tally((r) => (r.held_start ? 'held' : 'not held')),
      confounder: tally((r) => (r.confounders.length ? r.confounders : ['none'])),
    };
  }

  /** A within-record comparison of A (planned) with B (the rival): both values, the difference, the
      relative difference only when B is positive, and what kind of comparison it is. */
  function compare(r) {
    const a = r.a.value; const b = r.b.value;
    // A company with nothing left reports value 1: a ratio against it means nothing.
    const meaningful = typeof b === 'number' && b > 1;
    return {
      a, b, delta: a - b, rel: meaningful ? (a - b) / b : null,
      verdict: r.class, why: r.problems.concat(r.caveats), confounders: r.confounders,
    };
  }

  /** Groups of records that share a kind, seed and length, largest first: what a timeline may put
      side by side. */
  function groups(rows) {
    const m = new Map();
    rows.forEach((r) => {
      const key = r.kind + ':' + r.seed + ':' + r.months;
      if (!m.has(key)) m.set(key, { key, kind: r.kind, seed: r.seed, months: r.months, rows: [] });
      m.get(key).rows.push(r);
    });
    return [...m.values()].filter((g) => g.rows.length > 1).sort((x, y) => y.rows.length - x.rows.length || x.key.localeCompare(y.key));
  }

  /** One group in time order, each step marked against the one before it (planned value). */
  function steps(group) {
    const rows = group.rows.slice().sort((x, y) => (x.at || '').localeCompare(y.at || '') || x.id.localeCompare(y.id));
    return rows.map((r, i) => {
      const prev = i ? rows[i - 1].a.value : null;
      const change = prev === null ? null : r.a.value - prev;
      return { r, prev, change, dir: change === null ? 'first' : change < 0 ? 'down' : change > 0 ? 'up' : 'same' };
    });
  }

  /** A node's edges in a graph, kept to the given kinds: what feeds it and what it feeds. */
  function neighbours(graph, id, kinds) {
    const keep = (e) => !kinds || kinds.includes(e.kind);
    return {
      in: graph.edges.filter((e) => keep(e) && (e.to === id || (e.both && e.from === id))),
      out: graph.edges.filter((e) => keep(e) && (e.from === id || (e.both && e.to === id))),
    };
  }

  /** The order a process diagram is walked in: from the first node, along its edges, breadth first;
      nodes no edge reaches follow in the order the diagram defines them. */
  function walk(graph) {
    if (!graph.nodes.length) return [];
    const order = [graph.nodes[0].id];
    for (let i = 0; i < order.length; i++) {
      graph.edges.filter((e) => e.from === order[i]).forEach((e) => { if (!order.includes(e.to)) order.push(e.to); });
    }
    graph.nodes.forEach((n) => { if (!order.includes(n.id)) order.push(n.id); });
    return order;
  }

  const Core = { FILTERS, VIEWS, readState, filterRows, facets, compare, groups, steps, neighbours, walk };
  if (typeof module !== 'undefined' && module.exports) { module.exports = Core; return; }

  // ------------------------------------------------------------------ page

  const S = window.surfaces;
  const mount = document.getElementById('page-app');
  if (!S || !mount) return;
  const esc = S.escapeHtml;
  const num = new Intl.NumberFormat('en');
  const pct = (x) => (x === null || x === undefined ? '—' : (x > 0 ? '+' : '') + (x * 100).toFixed(1) + '%');
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const compact = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });
  /** A diagram label on one line: its lines joined by " · ", except a line that continues the one
      before it (it opens with a bracket or a separator). */
  const oneLine = (label) => String(label).split('\n').reduce((acc, l) => (!acc ? l : /^[(·,]/.test(l) ? acc + ' ' + l : acc + ' · ' + l), '');

  const CARD = 'chart-card relative mb-4 rounded-lg bg-white p-4 shadow-sm sm:p-6 dark:bg-gray-800';
  const SELECT = 'rounded-lg border border-gray-300 bg-gray-50 p-1.5 text-sm text-gray-900 focus:border-primary-500 '
    + 'focus:ring-primary-500 dark:border-gray-600 dark:bg-gray-700 dark:text-white';
  const BTN = 'inline-flex items-center rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-sm font-medium '
    + 'text-gray-900 hover:bg-gray-100 focus:outline-none focus:ring-4 focus:ring-primary-300 dark:border-gray-600 '
    + 'dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700';
  const MAX = '<button type="button" class="maximize rounded-lg p-1.5 text-gray-500 hover:bg-gray-100 dark:text-gray-400 '
    + 'dark:hover:bg-gray-700" aria-label="Maximize" aria-pressed="false" title="Maximize (Esc to close)">&#x26F6;</button>';
  const CLASS_BADGE = {
    valid: 'bg-[var(--atlas-signal)] text-[var(--atlas-paper)]',
    limited: 'bg-gray-200 text-gray-800 dark:bg-gray-600 dark:text-gray-100',
    invalid: 'border border-[var(--atlas-signal)] text-[var(--atlas-signal)]',
  };
  const badge = (c) => '<span class="rounded px-2 py-0.5 font-mono text-xs ' + (CLASS_BADGE[c] || '') + '">' + esc(c) + '</span>';
  let data = null;
  let st = readState(S.params);

  function set(key, value, rerender) {
    st[key] = value;
    S.setParam(key, Array.isArray(value) ? value.join(',') : (value === 0 ? '' : value));
    if (rerender) rerender();
  }

  /** Chart colours: the NoirGFX palette from the data, in the page's theme. */
  function ink() {
    const p = data.palette; const dark = S.isDark();
    return { text: dark ? p.paper : p.ink, ground: dark ? '#1f2937' : '#ffffff', signal: p.signal,
      muted: '#8a8a8a', line: dark ? 'rgba(236,228,214,.25)' : 'rgba(11,11,11,.18)', dark };
  }

  let drawing = [];

  /** Every chart goes through here: one ECharts init per box, registered for resize and maximize.
      The page reports itself ready only once every chart it started has been drawn. */
  function chart(id, option, onClick) {
    const p = draw(id, option, onClick);
    drawing.push(p);
    return p;
  }

  async function draw(id, option, onClick) {
    const echarts = await S.echarts();
    const el = document.getElementById(id);
    if (!el) return null;
    if (S.charts[id]) S.charts[id].dispose();
    const c = echarts.init(el, null, { renderer: 'canvas' });
    const k = ink();
    c.setOption(Object.assign({
      backgroundColor: 'transparent', animation: !reduced(),
      textStyle: { color: k.text, fontFamily: 'Inter, ui-sans-serif, system-ui, sans-serif' },
    }, option));
    if (onClick) c.on('click', onClick);
    S.charts[id] = c;
    return c;
  }

  const axis = () => { const k = ink(); return { axisLine: { lineStyle: { color: k.line } }, splitLine: { lineStyle: { color: k.line } }, axisLabel: { color: k.text } }; };
  const toolbox = () => ({ right: 8, feature: { dataView: { readOnly: true }, saveAsImage: { backgroundColor: ink().ground } } });

  function card(id, title, subtitle, body) {
    return '<section id="' + id + '" class="' + CARD + '" aria-labelledby="' + id + '-h">'
      + '<div class="mb-3 flex flex-wrap items-start justify-between gap-2"><div>'
      + '<h2 id="' + id + '-h" class="text-xl font-bold leading-none text-gray-900 dark:text-white">' + esc(title) + '</h2>'
      + '<p class="mt-1 max-w-3xl text-sm text-gray-500 dark:text-gray-400">' + subtitle + '</p></div>' + MAX + '</div>'
      + body + '</section>';
  }

  function select(key, label, options, value) {
    return '<label class="flex flex-col gap-1 text-xs text-gray-500 dark:text-gray-400">' + esc(label)
      + '<select data-filter-key="' + key + '" class="' + SELECT + '"><option value="">all</option>'
      + options.map(([v, n]) => '<option value="' + esc(v) + '"' + (String(value) === String(v) ? ' selected' : '') + '>'
        + esc(v) + ' (' + n + ')</option>').join('') + '</select></label>';
  }

  // ------------------------------------------------------------------ explorer

  function renderExplorer() {
    const rows = data.comparisons;
    const f = facets(rows);
    const box = document.getElementById('explorer-body');
    box.innerHTML = '<div class="mb-3 grid grid-cols-2 gap-2 sm:grid-cols-4 xl:grid-cols-8">'
      + select('kind', 'kind', f.kind, st.kind) + select('class', 'comparability', f.class, st.class)
      + select('seed', 'seed', f.seed, st.seed) + select('months', 'months', f.months, st.months)
      + select('opponent', 'rival AI', f.opponent, st.opponent) + select('start', 'start', f.start, st.start)
      + select('confounder', 'confounder', f.confounder, st.confounder)
      + '<label class="flex flex-col gap-1 text-xs text-gray-500 dark:text-gray-400">label contains'
      + '<input data-filter-key="q" type="search" value="' + esc(st.q) + '" class="' + SELECT + '"></label></div>'
      + '<dl class="mb-3 grid grid-cols-1 gap-1 text-xs text-gray-600 md:grid-cols-3 dark:text-gray-300">'
      + data.comparability.map((c) => '<div><dt class="inline">' + badge(c.id) + '</dt> <dd class="inline">' + esc(c.meaning) + '</dd></div>').join('') + '</dl>'
      + '<div class="mb-2 flex flex-wrap items-center gap-2 text-sm text-gray-600 dark:text-gray-300">'
      + '<span id="explorer-count" class="font-mono"></span>'
      + '<button type="button" id="explorer-reset" class="' + BTN + '">Reset filters</button>'
      + '<button type="button" id="explorer-scale" class="' + BTN + '" aria-pressed="' + (st.scale === 'log') + '">'
      + 'Scale: ' + st.scale + '</button></div>'
      + '<div class="grid grid-cols-1 gap-4 xl:grid-cols-5"><div class="xl:col-span-3">'
      + '<div id="explorer-chart" class="chart h-96 w-full" role="img" aria-label="Planned against rival company value, one mark per record"></div>'
      + '<p class="mt-1 text-xs text-gray-500 dark:text-gray-400">One mark per record: across, the rival\'s final company value; up, the planned bot\'s. Above the line the planned bot ended ahead. Colour is the comparability class; click a mark to open the record.</p>'
      + '</div><div id="explorer-detail" class="xl:col-span-2" aria-live="polite"></div></div>'
      + '<div id="explorer-table" class="mt-4"></div>';
    box.querySelectorAll('[data-filter-key]').forEach((el) => {
      el.addEventListener(el.tagName === 'INPUT' ? 'input' : 'change', () => set(el.dataset.filterKey, el.value, drawExplorer));
    });
    document.getElementById('explorer-reset').addEventListener('click', () => {
      FILTERS.forEach((k) => set(k, ''));
      renderExplorer();
    });
    document.getElementById('explorer-scale').addEventListener('click', () => {
      set('scale', st.scale === 'log' ? 'linear' : 'log');
      renderExplorer();
    });
    return drawExplorer();
  }

  function drawExplorer() {
    const rows = filterRows(data.comparisons, st);
    const k = ink();
    document.getElementById('explorer-count').textContent = rows.length + ' of ' + data.comparisons.length + ' records';
    const table = document.getElementById('explorer-table');
    if (!rows.length) {
      table.innerHTML = '<div data-empty="explorer" class="rounded-lg border border-dashed border-gray-300 p-6 text-center text-sm text-gray-500 dark:border-gray-600 dark:text-gray-400">'
        + 'No record matches these filters. <button type="button" class="underline" data-reset>Reset filters</button></div>';
      table.querySelector('[data-reset]').addEventListener('click', () => { FILTERS.forEach((x) => set(x, '')); renderExplorer(); });
    } else {
      table.innerHTML = '<div class="relative max-h-[28rem] overflow-auto rounded-lg border border-gray-200 dark:border-gray-700">'
        + '<table class="w-full text-left text-sm text-gray-600 dark:text-gray-300"><caption class="sr-only">Records shown in the chart</caption>'
        + '<thead class="sticky top-0 bg-gray-50 text-xs uppercase text-gray-700 dark:bg-gray-700 dark:text-gray-300"><tr>'
        + ['Record', 'Seed', 'Months', 'Planned', 'Rival', 'Δ', 'Class', 'Confounders'].map((h) => '<th scope="col" class="px-3 py-2">' + h + '</th>').join('')
        + '</tr></thead><tbody>'
        + rows.map((r) => {
          const c = compare(r);
          return '<tr tabindex="0" data-row="' + esc(r.id) + '" aria-selected="' + (r.id === st.sel) + '" class="cursor-pointer border-b border-gray-200 hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-[var(--atlas-signal)] aria-selected:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-700 dark:aria-selected:bg-gray-700">'
            + '<td class="px-3 py-2 font-mono text-xs">' + esc(r.id) + '</td><td class="px-3 py-2">' + esc(r.seed) + '</td>'
            + '<td class="px-3 py-2">' + esc(r.months) + '</td><td class="px-3 py-2 text-right font-mono">' + num.format(c.a) + '</td>'
            + '<td class="px-3 py-2 text-right font-mono">' + num.format(c.b) + '</td><td class="px-3 py-2 text-right font-mono">' + pct(c.rel) + '</td>'
            + '<td class="px-3 py-2">' + badge(r.class) + '</td><td class="px-3 py-2 text-xs">' + esc(r.confounders.join(', ') || '—') + '</td></tr>';
        }).join('') + '</tbody></table></div>';
      table.querySelectorAll('tr[data-row]').forEach((tr) => {
        const pick = () => selectRecord(tr.dataset.row, true);
        tr.addEventListener('click', pick);
        tr.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(); } });
      });
    }
    const series = data.comparability.map((cl) => ({
      name: cl.id, type: 'scatter',
      itemStyle: cl.id === 'valid' ? { color: k.signal } : cl.id === 'limited' ? { color: k.muted } : { color: 'transparent', borderColor: k.signal, borderWidth: 2 },
      symbolSize: (v) => 6 + Math.sqrt(v[2]) * 1.4,
      emphasis: { focus: 'series', itemStyle: { borderColor: k.text, borderWidth: 2 } },
      data: rows.filter((r) => r.class === cl.id).map((r) => ({
        value: [r.b.value, r.a.value, r.months], id: r.id, name: r.id,
        itemStyle: r.id === st.sel ? { borderColor: k.text, borderWidth: 3 } : undefined,
      })),
    }));
    const vals = rows.flatMap((r) => [r.a.value, r.b.value]).filter((v) => v > 0);
    const lo = vals.length ? Math.min(...vals) : 1; const hi = vals.length ? Math.max(...vals) : 10;
    series.push({ name: 'equal value', type: 'line', silent: true, symbol: 'none', lineStyle: { color: k.text, type: 'dashed', width: 1, opacity: 0.5 }, data: [[lo, lo], [hi, hi]], tooltip: { show: false } });
    const byId = new Map(data.comparisons.map((r) => [r.id, r]));
    const type = st.scale === 'log' ? 'log' : 'value';
    const narrow = document.getElementById('explorer-chart').clientWidth < 560;
    const tick = { formatter: (v) => compact.format(v), color: k.text, hideOverlap: true };
    chart('explorer-chart', {
      color: [k.signal, k.muted, k.signal],
      legend: { top: 0, left: 0, textStyle: { color: k.text }, data: data.comparability.map((c) => c.id) },
      tooltip: {
        trigger: 'item', formatter: (p) => {
          const r = byId.get(p.data && p.data.id);
          if (!r) return '';
          const c = compare(r);
          return '<b>' + esc(r.id) + '</b><br>seed ' + esc(r.seed) + ' · ' + esc(r.months) + ' months · ' + esc(r.class)
            + '<br>planned ' + num.format(c.a) + ' · rival ' + num.format(c.b) + ' · ' + pct(c.rel)
            + '<br><span style="opacity:.7">' + esc((r.label || '').slice(0, 90)) + '</span>';
        },
      },
      toolbox: toolbox(),
      grid: { left: 8, right: 32, top: 44, bottom: 40, containLabel: true },
      xAxis: Object.assign({ type, name: 'rival (B) value', nameLocation: 'middle', nameGap: 26, min: type === 'log' ? lo / 1.5 : 0 }, axis(), { axisLabel: tick }),
      yAxis: Object.assign({ type, name: narrow ? '' : 'planned (A) value', min: type === 'log' ? lo / 1.5 : 0 }, axis(), { axisLabel: tick }),
      dataZoom: [{ type: 'inside', xAxisIndex: 0 }, { type: 'inside', yAxisIndex: 0 }],
      series,
    }, (p) => p.data && p.data.id && selectRecord(p.data.id, false));
    drawDetail();
  }

  function selectRecord(id, fromTable) {
    set('sel', id === st.sel && fromTable ? '' : id);
    document.querySelectorAll('#explorer-table tr[data-row]').forEach((tr) => tr.setAttribute('aria-selected', String(tr.dataset.row === st.sel)));
    const c = S.charts['explorer-chart'];
    if (c) {
      const k = ink();
      const opt = c.getOption();
      opt.series.forEach((s) => (s.data || []).forEach((d) => { if (d && d.id) d.itemStyle = d.id === st.sel ? { borderColor: k.text, borderWidth: 3 } : undefined; }));
      c.setOption({ series: opt.series.map((s) => ({ data: s.data })) });
    }
    if (!fromTable) {
      const tr = document.querySelector('#explorer-table tr[data-row="' + CSS.escape(st.sel) + '"]');
      if (tr) tr.scrollIntoView({ block: 'nearest' });
    }
    drawDetail();
  }

  function drawDetail() {
    const box = document.getElementById('explorer-detail');
    const r = data.comparisons.find((x) => x.id === st.sel);
    if (!r) {
      box.innerHTML = '<div class="rounded-lg border border-dashed border-gray-300 p-4 text-sm text-gray-500 dark:border-gray-600 dark:text-gray-400">'
        + 'Select a mark or a row to compare its two companies and open its evidence.</div>';
      return;
    }
    const c = compare(r);
    const kv = (k, v) => '<div><dt class="text-xs text-gray-500 dark:text-gray-400">' + k + '</dt><dd class="font-mono text-sm text-gray-900 dark:text-white">' + v + '</dd></div>';
    const fleet = (x) => (x.trains === null ? 'not recorded' : x.trains + ' / ' + x.road + ' / ' + x.planes);
    const rev = r.revision ? (r.revision_known
      ? '<a class="underline" href="' + esc(S.web + '/commit/' + r.revision) + '">' + esc(r.revision) + '</a>'
      : esc(r.revision) + ' <span class="text-xs">(not a commit of this repository)</span>') + (r.revision_dirty ? ' -dirty' : '') : 'not recorded';
    box.innerHTML = '<div data-detail="' + esc(r.id) + '" class="rounded-lg border border-gray-200 p-4 dark:border-gray-700">'
      + '<div class="mb-2 flex flex-wrap items-center gap-2"><span class="font-mono text-sm font-bold text-gray-900 dark:text-white">' + esc(r.id) + '</span>' + badge(r.class) + '</div>'
      + '<p class="mb-3 text-sm text-gray-600 dark:text-gray-300">' + esc(r.label || r.experiment || '') + '</p>'
      + '<dl class="mb-3 grid grid-cols-2 gap-2">'
      + kv('planned (A)', num.format(c.a)) + kv('rival (B)', num.format(c.b))
      + kv('A − B', (c.delta > 0 ? '+' : '') + num.format(c.delta)) + kv('relative', c.rel === null ? 'not meaningful (B has no assets)' : pct(c.rel))
      + kv('length', r.recorded_months + ' of ' + r.months + ' months') + kv('start', r.held_start ? 'held (tick ' + esc(r.held_start_tick) + ')' : 'not held')
      + kv('A fleet: trains / road / air', fleet(r.a)) + kv('B fleet: trains / road / air', fleet(r.b))
      + kv('rival AI', esc(r.opponent || 'not recorded')) + kv('revision', rev) + '</dl>'
      + (c.why.length ? '<p class="mb-1 text-xs font-semibold uppercase text-gray-500">Why ' + esc(r.class) + '</p><ul class="mb-2 list-disc ps-5 text-sm text-gray-600 dark:text-gray-300">' + c.why.map((w) => '<li>' + esc(w) + '</li>').join('') + '</ul>' : '<p class="mb-2 text-sm text-gray-600 dark:text-gray-300">Paired in one game from a held start, every month recorded.</p>')
      + (c.confounders.length ? '<p class="mb-1 text-xs font-semibold uppercase text-gray-500">Confounders</p><ul class="mb-2 list-disc ps-5 text-sm text-gray-600 dark:text-gray-300">' + c.confounders.map((w) => '<li>' + esc(w) + '</li>').join('') + '</ul>' : '')
      + '<div id="detail-chart" class="chart h-44 w-full" role="img" aria-label="Company value per month, planned against rival"></div>'
      + '<p class="mt-2 text-xs text-gray-500 dark:text-gray-400">Evidence: '
      + '<a data-evidence class="underline" href="' + esc(S.href(r.page)) + '">write-up</a> → '
      + '<a data-evidence class="underline" href="' + esc(S.href(r.path)) + '">record JSON</a> → '
      + '<a data-evidence class="underline" href="' + esc(S.href(r.kind === 'ab' ? 'tools/research/mcp-ab' : 'tools/research/mcp-duel')) + '">tool</a> → '
      + '<a data-evidence class="underline" href="' + esc(S.href('.ai/repo/rules/project/a-tuning-claim-needs-held-start-pairs.v1.md')) + '">rule</a></p>'
      + (r.bot_settings || r.rival_settings ? '<details class="mt-2 text-xs text-gray-500 dark:text-gray-400"><summary class="cursor-pointer">Settings</summary>'
        + (r.bot_settings ? '<p class="mt-1 break-all font-mono">A: ' + esc(r.bot_settings) + '</p>' : '')
        + (r.rival_settings ? '<p class="mt-1 break-all font-mono">B: ' + esc(r.rival_settings) + '</p>' : '') + '</details>' : '')
      + '</div>';
    const k = ink();
    chart('detail-chart', {
      tooltip: { trigger: 'axis' }, legend: { top: 0, textStyle: { color: k.text } },
      grid: { left: 4, right: 8, top: 28, bottom: 4, containLabel: true },
      xAxis: Object.assign({ type: 'category', data: r.series.month, name: 'month' }, axis()),
      yAxis: Object.assign({ type: 'value' }, axis(), { axisLabel: { formatter: (v) => compact.format(v), color: ink().text } }),
      series: [
        { name: 'planned (A)', type: 'line', showSymbol: false, data: r.series.a, lineStyle: { color: k.signal, width: 2 }, itemStyle: { color: k.signal } },
        { name: 'rival (B)', type: 'line', showSymbol: false, data: r.series.b, lineStyle: { color: k.muted, width: 2 }, itemStyle: { color: k.muted } },
      ],
    });
  }

  // ------------------------------------------------------------------ timeline

  function renderTimeline() {
    const gs = groups(data.comparisons);
    const box = document.getElementById('timeline-body');
    if (!gs.length) { box.innerHTML = '<p data-empty="timeline" class="text-sm text-gray-500">No seed and length has more than one record.</p>'; return; }
    const g = gs.find((x) => x.key === st.group) || gs[0];
    box.innerHTML = '<label class="mb-3 flex max-w-md flex-col gap-1 text-xs text-gray-500 dark:text-gray-400">records that share a kind, seed and length'
      + '<select id="timeline-group" class="' + SELECT + '">' + gs.map((x) => '<option value="' + esc(x.key) + '"' + (x === g ? ' selected' : '') + '>'
        + esc(x.kind + ' · seed ' + x.seed + ' · ' + x.months + ' months') + ' (' + x.rows.length + ')</option>').join('') + '</select></label>'
      + '<div id="timeline-chart" class="chart h-96 w-full" role="img" aria-label="Planned and rival value per record, in time order"></div>'
      + '<p class="mt-1 text-xs text-gray-500 dark:text-gray-400">Each bar is one record, oldest first; every step changed code or settings (its label says which). Bright red: a valid record (held start); dark red: limited; outlined: invalid. ▼ marks a step down: a regression is a result, not noise to smooth away.</p>'
      + '<div id="timeline-table" class="mt-3"></div>';
    document.getElementById('timeline-group').addEventListener('change', (e) => { set('group', e.target.value); renderTimeline(); });
    const s = steps(g);
    const k = ink();
    const downs = s.map((x, i) => (x.dir === 'down' ? { coord: [i, x.r.a.value], value: '▼' } : null)).filter(Boolean);
    chart('timeline-chart', {
      tooltip: { trigger: 'axis', formatter: (ps) => {
        const x = s[ps[0].dataIndex];
        return '<b>' + esc(x.r.id) + '</b> ' + esc(x.r.class) + '<br>planned ' + num.format(x.r.a.value) + (x.change === null ? '' : ' (' + (x.change > 0 ? '+' : '') + num.format(x.change) + ' on the previous)')
          + '<br>rival ' + num.format(x.r.b.value) + '<br><span style="opacity:.7">' + esc((x.r.label || '').slice(0, 100)) + '</span>';
      } },
      color: [k.signal, k.muted],
      legend: { top: 0, left: 0, textStyle: { color: k.text } }, toolbox: Object.assign(toolbox(), { feature: Object.assign(toolbox().feature, { magicType: { type: ['bar', 'line'] } }) }),
      grid: { left: 8, right: 24, top: 48, bottom: 48, containLabel: true },
      xAxis: Object.assign({ type: 'category', data: s.map((x, i) => 'r' + (i + 1)) }, axis()),
      yAxis: Object.assign({ type: 'value', name: 'company value' }, axis(), { axisLabel: { formatter: (v) => compact.format(v), color: k.text } }),
      dataZoom: s.length > 16 ? [{ type: 'inside' }, { type: 'slider', height: 16, bottom: 6 }] : [],
      series: [
        { name: 'planned (A)', type: 'bar', itemStyle: { color: k.signal }, data: s.map((x) => ({ value: x.r.a.value, id: x.r.id, itemStyle: { color: x.r.class === 'valid' ? k.signal : x.r.class === 'limited' ? data.palette.signal_dark : 'transparent', borderColor: k.signal, borderWidth: x.r.class === 'invalid' ? 2 : 0 } })),
          markPoint: { symbol: 'pin', symbolSize: 34, label: { color: data.palette.paper }, itemStyle: { color: k.text }, data: downs } },
        { name: 'rival (B)', type: 'line', data: s.map((x) => ({ value: x.r.b.value, id: x.r.id })), lineStyle: { color: k.muted }, itemStyle: { color: k.muted } },
      ],
    }, (p) => p.data && p.data.id && goRecord(p.data.id));
    document.getElementById('timeline-table').innerHTML = '<details><summary class="cursor-pointer text-sm text-gray-600 dark:text-gray-300">The steps as a table</summary>'
      + '<div class="mt-2 max-h-80 overflow-auto rounded-lg border border-gray-200 dark:border-gray-700"><table class="w-full text-left text-sm text-gray-600 dark:text-gray-300"><thead class="bg-gray-50 text-xs uppercase dark:bg-gray-700"><tr>'
      + ['Step', 'Record', 'Planned', 'Change', 'Rival', 'Class', 'What changed'].map((h) => '<th scope="col" class="px-3 py-2">' + h + '</th>').join('') + '</tr></thead><tbody>'
      + s.map((x, i) => '<tr class="border-b border-gray-200 dark:border-gray-700"><td class="px-3 py-2">r' + (i + 1) + '</td><td class="px-3 py-2 font-mono text-xs"><a class="underline" href="' + esc(S.href(x.r.page)) + '">' + esc(x.r.id) + '</a></td>'
        + '<td class="px-3 py-2 text-right font-mono">' + num.format(x.r.a.value) + '</td><td class="px-3 py-2 text-right font-mono">' + (x.change === null ? '—' : (x.dir === 'down' ? '▼ ' : '') + num.format(x.change)) + '</td>'
        + '<td class="px-3 py-2 text-right font-mono">' + num.format(x.r.b.value) + '</td><td class="px-3 py-2">' + badge(x.r.class) + '</td><td class="px-3 py-2 text-xs">' + esc(x.r.label) + '</td></tr>').join('')
      + '</tbody></table></div></details>';
  }

  function goRecord(id) {
    const r = data.comparisons.find((x) => x.id === id);
    if (!r) return;
    FILTERS.forEach((k) => set(k, ''));
    set('view', 'explorer');
    renderExplorer();
    selectRecord(id, false);
    document.getElementById('explorer').scrollIntoView({ block: 'start', behavior: reduced() ? 'auto' : 'smooth' });
  }

  // ------------------------------------------------------------------ battle

  /** OpenTTD stores a map side as a power of two (game_creation.map_x = 8 is 256 tiles). */
  const mapSide = (log2) => (Number.isInteger(log2) && log2 > 0 && log2 < 16 ? 2 ** log2 : log2);

  function renderBattle() {
    const b = data.battles;
    const box = document.getElementById('battle-body');
    if (!b || !b.latest) { box.innerHTML = '<p data-empty="battle" class="text-sm text-gray-500">No battle record.</p>'; return; }
    const L = b.latest; const k = ink();
    const ours = (r) => r.ai === 'PrismaticAI';
    box.innerHTML = '<p class="mb-3 text-sm text-gray-600 dark:text-gray-300">Newest record <a class="font-mono underline" href="' + esc(S.href(L.page)) + '">' + esc(L.id) + '</a>: '
      + esc(L.matches) + ' scored matches, ' + esc((L.match_cfg || {}).years) + ' years from ' + esc((L.match_cfg || {}).start_year) + ', '
      + esc(mapSide((L.match_cfg || {}).map_x)) + '×' + esc(mapSide((L.match_cfg || {}).map_y)) + ', on ' + esc(L.hosts.map((h) => h.name).join(', '))
      + '; replays identical on another arena: <b>' + L.determinism.filter((d) => d.identical).length + ' of ' + L.determinism.length + '</b>. '
      + 'Scoring <code>' + esc(L.scoring) + '</code>; PrismaticAI and the referee at <a class="font-mono underline" href="' + esc(S.web + '/commit/' + L.rev) + '">' + esc(String(L.rev).slice(0, 10)) + '</a>.</p>'
      + '<div class="grid grid-cols-1 gap-4 xl:grid-cols-2"><div><div id="battle-board" class="chart h-96 w-full" role="img" aria-label="Points per match by contestant, newest battle"></div></div>'
      + '<div><div id="battle-history" class="chart h-96 w-full" role="img" aria-label="PrismaticAI best average rank per battle record over time"></div>'
      + '<p class="mt-1 text-xs text-gray-500 dark:text-gray-400">Every battle record (' + b.records + '), oldest first: PrismaticAI\'s best average rank (lower is better) and the winner\'s points. Records differ in lineup and edition; click one for its page.</p></div></div>';
    const board = L.leaderboard.slice().reverse();
    chart('battle-board', {
      tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, formatter: (ps) => {
        const r = board[ps[0].dataIndex];
        return '<b>' + esc(r.key) + '</b><br>' + r.avg_points + ' points/match · avg rank ' + r.avg_rank + '<br>played ' + r.played + ', won ' + r.wins + ' · avg value ' + num.format(r.avg_value) + '<br>errors ' + r.errors + ' · anomalies ' + r.anomalies;
      } },
      toolbox: toolbox(), grid: { left: 8, right: 40, top: 40, bottom: 16, containLabel: true },
      xAxis: Object.assign({ type: 'value', name: 'points/match' }, axis()),
      yAxis: Object.assign({ type: 'category', data: board.map((r) => r.key), axisLabel: { color: k.text, width: 180, overflow: 'truncate' } }, { axisLine: axis().axisLine }),
      series: [{ type: 'bar', data: board.map((r) => ({ value: r.avg_points, itemStyle: { color: ours(r) ? k.signal : k.muted, borderRadius: [0, 3, 3, 0] } })), label: { show: true, position: 'right', color: k.text } }],
    }, () => { location.href = S.href(L.page); });
    const h = b.history;
    chart('battle-history', {
      tooltip: { trigger: 'axis', formatter: (ps) => {
        const r = h[ps[0].dataIndex];
        return '<b>' + esc(r.id) + '</b><br>' + r.matches + ' matches, ' + r.field + ' contestants<br>PrismaticAI best: ' + esc(r.prismatic_best || '—') + ', rank ' + (r.prismatic_best_rank ?? '—')
          + '<br>winner ' + esc(r.winner || '—') + ', ' + (r.winner_points ?? '—') + ' points/match' + (r.prismatic_last ? '<br><b>PrismaticAI last</b>' : '');
      } },
      legend: { top: 0, left: 0, textStyle: { color: k.text } }, toolbox: toolbox(),
      grid: { left: 8, right: 24, top: 64, bottom: 48, containLabel: true },
      xAxis: Object.assign({ type: 'category', data: h.map((r) => (r.at || '').slice(0, 16).replace('T', ' ')) }, axis()),
      yAxis: [Object.assign({ type: 'value', name: 'best avg rank', inverse: true, min: 1 }, axis()), Object.assign({ type: 'value', name: 'winner pts', splitLine: { show: false } }, { axisLabel: { color: k.text } })],
      dataZoom: [{ type: 'inside' }, { type: 'slider', height: 16, bottom: 6 }],
      series: [
        { name: 'PrismaticAI best avg rank', type: 'line', step: 'end', showSymbol: false, data: h.map((r) => ({ value: r.prismatic_best_rank, page: r.page })), lineStyle: { color: k.signal }, itemStyle: { color: k.signal } },
        { name: 'winner points/match', type: 'line', yAxisIndex: 1, showSymbol: false, data: h.map((r) => ({ value: r.winner_points, page: r.page })), lineStyle: { color: k.muted, type: 'dashed' }, itemStyle: { color: k.muted } },
      ],
    }, (p) => { if (p.data && p.data.page) location.href = S.href(p.data.page); });
  }

  // ------------------------------------------------------------------ graph

  const KIND_STYLE = (kind) => {
    const k = ink();
    return { control: { color: k.text, type: 'solid', width: 1.5 }, data: { color: k.muted, type: 'dashed', width: 1.5 }, governance: { color: k.signal, type: 'solid', width: 3.5 } }[kind] || { color: k.muted };
  };

  function graphOf(id) { return data.graphs.find((g) => g.id === id) || data.graphs[0]; }

  function renderGraph() {
    const box = document.getElementById('graph-body');
    const arch = data.graphs.filter((g) => g.id !== 'approval');
    if (!arch.length) { box.innerHTML = '<p data-empty="graph" class="text-sm text-gray-500">No diagram is marked for the atlas.</p>'; return; }
    const g = arch.find((x) => x.id === st.graph) || arch[0];
    const kinds = st.kinds || data.edge_kinds.map((x) => x.id);
    box.innerHTML = '<div class="mb-3 flex flex-wrap items-end gap-3">'
      + (arch.length > 1 ? '<label class="flex flex-col gap-1 text-xs text-gray-500">diagram<select id="graph-pick" class="' + SELECT + '">' + arch.map((x) => '<option value="' + esc(x.id) + '"' + (x === g ? ' selected' : '') + '>' + esc(x.title) + '</option>').join('') + '</select></label>' : '')
      + '<fieldset class="flex flex-wrap gap-3"><legend class="sr-only">Edge kinds</legend>' + data.edge_kinds.map((x) => '<label class="flex items-center gap-1 text-sm text-gray-700 dark:text-gray-300" title="' + esc(x.meaning) + '">'
        + '<input type="checkbox" data-kind="' + esc(x.id) + '" class="rounded border-gray-300 text-[var(--atlas-signal)] focus:ring-[var(--atlas-signal)]"' + (kinds.includes(x.id) ? ' checked' : '') + '>'
        + '<span class="font-mono text-xs">' + esc(x.arrow) + '</span> ' + esc(x.id) + '</label>').join('') + '</fieldset>'
      + '<span class="text-xs text-gray-500 dark:text-gray-400">from <a class="underline" href="' + esc(S.href(g.source)) + '">' + esc(g.source) + '</a>, line ' + g.line + '</span></div>'
      + '<div class="grid grid-cols-1 gap-4 xl:grid-cols-3"><div class="xl:col-span-2"><div id="graph-chart" class="chart h-[32rem] w-full" role="img" aria-label="' + esc(g.title) + '"></div></div>'
      + '<div><div id="graph-detail" aria-live="polite"></div><p class="mb-1 mt-3 text-xs font-semibold uppercase text-gray-500">Parts (Tab, Enter)</p><ul id="graph-list" class="flex flex-wrap gap-1"></ul></div></div>'
      + '<dl class="mt-3 grid grid-cols-1 gap-1 text-xs text-gray-500 sm:grid-cols-3 dark:text-gray-400">' + data.edge_kinds.map((x) => '<div><dt class="inline font-mono">' + esc(x.arrow) + ' ' + esc(x.id) + '</dt> <dd class="inline">' + esc(x.meaning) + '</dd></div>').join('') + '</dl>';
    const pick = document.getElementById('graph-pick');
    if (pick) pick.addEventListener('change', () => { set('graph', pick.value); set('node', ''); renderGraph(); });
    box.querySelectorAll('[data-kind]').forEach((cb) => cb.addEventListener('change', () => {
      const on = [...box.querySelectorAll('[data-kind]:checked')].map((x) => x.dataset.kind);
      set('kinds', on.length === data.edge_kinds.length ? '' : on);
      st.kinds = on.length === data.edge_kinds.length ? null : on;
      renderGraph();
    }));
    const list = document.getElementById('graph-list');
    list.innerHTML = g.nodes.map((n) => '<li><button type="button" data-node="' + esc(n.id) + '" aria-pressed="' + (n.id === st.node) + '" class="rounded border border-gray-300 px-2 py-0.5 text-xs text-gray-700 hover:bg-gray-100 aria-pressed:border-[var(--atlas-signal)] aria-pressed:text-[var(--atlas-signal)] dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700">'
      + esc(oneLine(n.label).split(' · ')[0]) + (n.status === 'proposed' ? ' ·proposed' : '') + '</button></li>').join('');
    list.querySelectorAll('[data-node]').forEach((btn) => btn.addEventListener('click', () => selectNode(g, btn.dataset.node === st.node ? '' : btn.dataset.node)));
    drawGraph(g);
  }

  function drawGraph(g) {
    const k = ink();
    const kinds = st.kinds || data.edge_kinds.map((x) => x.id);
    const groupsUsed = [...new Set(g.nodes.map((n) => n.group || '—'))];
    const label = (id) => { const gr = g.groups.find((x) => x.id === id); return gr ? gr.label : id; };
    const nb = st.node ? neighbours(g, st.node, kinds) : null;
    const near = nb ? new Set([st.node, ...nb.in.map((e) => e.from), ...nb.out.map((e) => e.to), ...nb.in.map((e) => e.to), ...nb.out.map((e) => e.from)]) : null;
    chart('graph-chart', {
      tooltip: { formatter: (p) => (p.dataType === 'node' ? '<b>' + esc(p.data.full) + '</b><br>' + esc(label(p.data.group)) + ' · ' + esc(p.data.status) + (p.data.tooltip ? '<br>' + esc(p.data.tooltip) : '')
        : esc(p.data.kind) + (p.data.text ? ': ' + esc(p.data.text) : '')) },
      legend: { bottom: 0, type: 'scroll', textStyle: { color: k.text }, data: groupsUsed.map(label) },
      toolbox: { right: 8, feature: { saveAsImage: { backgroundColor: k.ground } } },
      series: [{
        type: 'graph', layout: 'force', roam: true, draggable: true, animation: !reduced(),
        force: { repulsion: 420, edgeLength: [80, 160], gravity: 0.08, layoutAnimation: !reduced() },
        categories: groupsUsed.map((x) => ({ name: label(x) })),
        edgeSymbol: ['none', 'arrow'], edgeSymbolSize: 7,
        label: { show: true, color: k.text, fontSize: 11, formatter: (p) => p.data.short },
        lineStyle: { curveness: 0.12, opacity: 0.9 },
        emphasis: { focus: 'adjacency' },
        data: g.nodes.map((n) => ({
          id: n.id, name: n.id, short: (() => { const t = oneLine(n.label); return t.length > 30 ? t.slice(0, 28) + '…' : t; })(), full: oneLine(n.label), status: n.status, tooltip: n.tooltip,
          group: n.group || '—', category: groupsUsed.indexOf(n.group || '—'), href: n.href,
          symbol: n.status === 'proposed' ? 'emptyCircle' : 'circle', symbolSize: n.id === st.node ? 30 : 20,
          itemStyle: Object.assign({ borderColor: n.id === st.node ? k.signal : k.ground, borderWidth: n.id === st.node ? 4 : 1 },
            near && !near.has(n.id) ? { opacity: 0.2 } : {}),
          label: near && !near.has(n.id) ? { opacity: 0.25 } : {},
        })),
        links: g.edges.filter((e) => kinds.includes(e.kind)).map((e) => ({
          source: e.from, target: e.to, kind: e.kind, text: e.label,
          symbol: e.both ? ['arrow', 'arrow'] : ['none', e.directed ? 'arrow' : 'none'],
          lineStyle: Object.assign({}, KIND_STYLE(e.kind), near && !(near.has(e.from) && near.has(e.to) && (e.from === st.node || e.to === st.node)) ? { opacity: 0.12 } : {}),
        })),
      }],
    }, (p) => { if (p.dataType === 'node') selectNode(g, p.data.id === st.node ? '' : p.data.id); });
    drawNode(g);
  }

  function selectNode(g, id) {
    set('node', id);
    document.querySelectorAll('#graph-list [data-node]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.node === id)));
    drawGraph(g);
  }

  function drawNode(g) {
    const box = document.getElementById('graph-detail');
    const n = g.nodes.find((x) => x.id === st.node);
    if (!n) { box.innerHTML = '<p class="rounded-lg border border-dashed border-gray-300 p-3 text-sm text-gray-500 dark:border-gray-600 dark:text-gray-400">Select a part to see what feeds it, what it feeds and its page.</p>'; return; }
    const nb = neighbours(g, n.id, st.kinds);
    const name = (id) => { const x = g.nodes.find((y) => y.id === id); return x ? x.label.split('\n')[0] : id; };
    const edgeList = (es, other) => (es.length ? '<ul class="space-y-1">' + es.map((e) => '<li class="text-sm"><button type="button" class="underline" data-go="' + esc(e[other]) + '">' + esc(name(e[other])) + '</button> '
      + '<span class="font-mono text-xs text-gray-500">' + esc(e.kind) + (e.label ? ': ' + esc(e.label.replace(/\n/g, ' ')) : '') + '</span></li>').join('') + '</ul>' : '<p class="text-sm text-gray-500">none (in the kinds shown)</p>');
    const group = g.groups.find((x) => x.id === n.group);
    box.innerHTML = '<div data-node-detail="' + esc(n.id) + '" class="rounded-lg border border-gray-200 p-3 dark:border-gray-700">'
      + '<p class="font-semibold text-gray-900 dark:text-white">' + esc(oneLine(n.label)) + '</p>'
      + '<p class="mb-2 text-xs text-gray-500 dark:text-gray-400">' + esc(group ? group.label.replace(/\n/g, ' ') : 'no group') + ' · <span class="font-mono">' + esc(n.status) + '</span>'
      + ' · drawn in <a class="underline" href="' + esc(S.href(g.source)) + '">' + esc(g.source) + '</a>:' + n.line + '</p>'
      + (n.href ? '<p class="mb-2"><a data-evidence class="text-sm font-medium text-[var(--atlas-signal)] underline" href="' + esc(S.href(n.href)) + '">Open its page →</a>' + (n.tooltip ? ' <span class="text-xs text-gray-500">' + esc(n.tooltip) + '</span>' : '') + '</p>' : '')
      + '<p class="text-xs font-semibold uppercase text-gray-500">Fed by</p>' + edgeList(nb.in, 'from')
      + '<p class="mt-2 text-xs font-semibold uppercase text-gray-500">Feeds</p>' + edgeList(nb.out, 'to') + '</div>';
    box.querySelectorAll('[data-go]').forEach((b) => b.addEventListener('click', () => selectNode(g, b.dataset.go)));
  }

  // ------------------------------------------------------------------ process

  let timer = null;

  function renderProcess() {
    const box = document.getElementById('process-body');
    const g = data.graphs.find((x) => x.id === 'approval');
    if (!g) { box.innerHTML = '<p data-empty="process" class="text-sm text-gray-500">No diagram is marked <code>%% atlas: approval</code>.</p>'; return; }
    const order = walk(g);
    const at = Math.min(Math.max(st.step, 0), order.length - 1);
    box.innerHTML = '<div class="mb-3 flex flex-wrap items-center gap-2">'
      + '<button type="button" id="proc-play" class="' + BTN + '" aria-pressed="' + Boolean(timer) + '">' + (timer ? 'Pause' : 'Play') + '</button>'
      + '<button type="button" id="proc-prev" class="' + BTN + '" aria-label="Previous step">←</button>'
      + '<button type="button" id="proc-next" class="' + BTN + '" aria-label="Next step">→</button>'
      + '<button type="button" id="proc-replay" class="' + BTN + '">Replay</button>'
      + '<input id="proc-pos" type="range" min="0" max="' + (order.length - 1) + '" value="' + at + '" aria-label="Step" class="h-2 w-40 cursor-pointer accent-[var(--atlas-signal)]">'
      + '<span class="font-mono text-xs text-gray-500">step ' + (at + 1) + ' of ' + order.length + '</span>'
      + '<span class="text-xs text-gray-500 dark:text-gray-400">from <a class="underline" href="' + esc(S.href(g.source)) + '">' + esc(g.source) + '</a>, line ' + g.line + '</span></div>'
      + '<ol id="proc-steps" class="relative grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-4">' + order.map((id, i) => {
        const n = g.nodes.find((x) => x.id === id);
        const outs = g.edges.filter((e) => e.from === id);
        const gate = outs.some((e) => e.kind === 'governance') || g.edges.some((e) => e.to === id && e.kind === 'governance');
        const on = i === at; const past = i < at;
        return '<li data-step="' + i + '" aria-current="' + (on ? 'step' : 'false') + '" class="rounded-lg border p-3 transition-opacity '
          + (on ? 'border-[var(--atlas-signal)] ring-2 ring-[var(--atlas-signal)]' : 'border-gray-200 dark:border-gray-700') + (past || on ? '' : ' opacity-50') + '">'
          + '<p class="font-mono text-xs text-gray-500">' + String(i + 1).padStart(2, '0') + (gate ? ' · <span class="text-[var(--atlas-signal)]">approval boundary</span>' : '') + '</p>'
          + '<p class="font-semibold text-gray-900 dark:text-white">' + esc(oneLine(n.label)) + '</p>'
          + (n.tooltip ? '<p class="text-xs text-gray-500 dark:text-gray-400">' + esc(n.tooltip) + '</p>' : '')
          + (outs.length ? '<p class="mt-1 text-xs text-gray-600 dark:text-gray-300">' + outs.map((e) => '→ ' + esc((g.nodes.find((x) => x.id === e.to) || { label: e.to }).label.split('\n')[0]) + ' <span class="font-mono">(' + esc(e.kind) + (e.label ? ': ' + esc(e.label.replace(/\n/g, ' ')) : '') + ')</span>').join('<br>') + '</p>' : '')
          + (n.href ? '<a data-evidence class="mt-1 inline-block text-xs underline" href="' + esc(S.href(n.href)) + '">source</a>' : '') + '</li>';
      }).join('') + '</ol>'
      + '<p class="mt-2 text-xs text-gray-500 dark:text-gray-400">A walk through the diagram in the order its edges lead. Proposing a change is not making it: a mutation waits as <code>pending_approval</code> until the owner approves those exact arguments, and only then runs as a game command. This page is read-only; nothing here reaches a game.</p>';
    const go = (i) => { set('step', Math.min(Math.max(i, 0), order.length - 1)); renderProcess(); };
    document.getElementById('proc-prev').addEventListener('click', () => { stop(); go(at - 1); });
    document.getElementById('proc-next').addEventListener('click', () => { stop(); go(at + 1); });
    document.getElementById('proc-pos').addEventListener('input', (e) => { stop(); go(Number(e.target.value)); });
    document.getElementById('proc-replay').addEventListener('click', () => { stop(); go(0); play(order.length); });
    document.getElementById('proc-play').addEventListener('click', () => { if (timer) { stop(); renderProcess(); } else play(order.length); });
  }

  function stop() { if (timer) clearInterval(timer); timer = null; }

  function play(n) {
    stop();
    if (reduced()) { set('step', n - 1); renderProcess(); return; } // reduced motion: jump to the end
    timer = setInterval(() => {
      if (st.step >= n - 1) { stop(); renderProcess(); return; }
      set('step', st.step + 1);
      renderProcess();
    }, 1400);
    renderProcess();
  }

  // ------------------------------------------------------------------ assembly

  function layout() {
    const nav = [['explorer', 'Duel explorer'], ['timeline', 'Bot evolution'], ['battle', 'LAN battle'], ['graph', 'Architecture'], ['process', 'From intent to evidence']];
    mount.innerHTML = '<nav aria-label="Atlas views" class="sticky top-16 z-10 mb-4 flex flex-wrap gap-2 rounded-lg bg-gray-50/95 py-2 dark:bg-gray-900/95">'
      + nav.map(([id, t]) => '<a href="?view=' + id + '#' + id + '" data-view="' + id + '" class="rounded-full border border-gray-300 px-3 py-1 text-sm text-gray-700 hover:border-[var(--atlas-signal)] hover:text-[var(--atlas-signal)] dark:border-gray-600 dark:text-gray-300">' + t + '</a>').join('') + '</nav>'
      + card('explorer', 'Duel explorer', 'Every duel (one game, the planned bot against the same bot unplanned) and every A/B record (two games side by side). Each record is classed by what it can support (the rules are under the filters). Different lengths, seeds and bot revisions are never pooled into one number.', '<div id="explorer-body"></div>')
      + card('timeline', 'Bot evolution', 'One seed and one length, record after record. The bot changed between records, so this is a history of revisions, not a repeated measurement.', '<div id="timeline-body"></div>')
      + card('battle', 'LAN battle', 'PrismaticAI against BaNaNaS company AIs in headless free-for-all matches. The standing is the newest record\'s; the history shows every record, losses and lost ground included.', '<div id="battle-body"></div>')
      + card('graph', 'Architecture', 'The README\'s own diagram, read as data: a part per node, a connection per edge, each edge typed by its stroke. Select a part to see what feeds it and what it feeds; every part opens its page.', '<div id="graph-body"></div>')
      + card('process', 'From intent to evidence', 'The loop an agent goes through to change the game, stepped through from the README\'s approval diagram.', '<div id="process-body"></div>');
    mount.querySelectorAll('[data-view]').forEach((a) => a.addEventListener('click', (e) => {
      e.preventDefault();
      set('view', a.dataset.view);
      document.getElementById(a.dataset.view).scrollIntoView({ block: 'start', behavior: reduced() ? 'auto' : 'smooth' });
    }));
    // A card on the page links to a view of this page: keep the reader here and set that view.
    document.querySelectorAll('a.evidence-card-link[href^="?"]').forEach((a) => a.addEventListener('click', (e) => {
      e.preventDefault();
      const q = new URLSearchParams(a.getAttribute('href'));
      FILTERS.concat(['sel']).forEach((key) => set(key, q.get(key) || ''));
      const view = q.get('view') || 'explorer';
      set('view', view);
      renderAll();
      document.getElementById(view).scrollIntoView({ block: 'start' });
    }));
  }

  function theme() {
    const p = data.palette;
    mount.style.setProperty('--atlas-ink', p.ink);
    mount.style.setProperty('--atlas-paper', p.paper);
    mount.style.setProperty('--atlas-signal', p.signal);
    document.querySelectorAll('[data-card]').forEach((c) => c.style.setProperty('--atlas-signal', p.signal));
  }

  function renderAll() {
    renderExplorer();
    renderTimeline();
    renderBattle();
    renderGraph();
    renderProcess();
  }

  async function start() {
    try {
      const res = await fetch(S.root + mount.dataset.src);
      if (!res.ok) throw new Error('HTTP ' + res.status + ' for ' + mount.dataset.src);
      data = await res.json();
      if (data.schema !== 'research-atlas/v1') throw new Error('unexpected schema ' + data.schema);
    } catch (err) {
      mount.dataset.state = 'error';
      mount.innerHTML = '<div role="alert" class="rounded-lg border border-red-300 bg-red-50 p-4 text-sm text-red-800 dark:border-red-800 dark:bg-gray-800 dark:text-red-400">'
        + 'The atlas data did not load (' + esc(err.message) + '). The cards above are complete without it; the data file is <a class="underline" href="' + esc(S.root + mount.dataset.src) + '">' + esc(mount.dataset.src) + '</a>.</div>';
      console.warn('[atlas] data did not load:', err.message);
      return;
    }
    theme();
    layout();
    await S.echarts();
    drawing = [];
    renderAll();
    await Promise.all(drawing);
    S.onTheme(() => renderAll());
    S.restoreMax();
    if (st.view) document.getElementById(st.view).scrollIntoView({ block: 'start' });
    mount.dataset.charts = String(Object.keys(S.charts).length);
    mount.dataset.state = 'ready';
  }

  start();
})();
