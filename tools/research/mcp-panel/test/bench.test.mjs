// node --test tools/research/mcp-panel/test/ : the benchmark pages (static/bench.js) in the fake
// browser of fake-dom.mjs: /bench (regression banner, record metadata, the table's group filter,
// text filter and sort kept in the address, the relative trend chart through the one door) and
// /bench/<group>/<name> (the history chart with its band and regression marks, the record rows, the
// selection kept in the address). The records are the fixtures test/fixtures/bench/*.json, the same
// files src/panel_bench.rs's own tests read; the answer is shaped as GET /api/bench gives it.
import './panel-ui.mjs'; // first: shell.js publishes PanelUI before any page script
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fakeDom, respond } from './fake-dom.mjs';

// Before the fake browser: ECharts reads navigator when it loads.
const echartsSsr = createRequire(import.meta.url)('echarts');

const FIXTURES = path.join(path.dirname(new URL(import.meta.url).pathname), 'fixtures', 'bench');
const files = fs.readdirSync(FIXTURES).filter((f) => /^bench-.*\.json$/.test(f)).sort();
const parsed = files.map((f) => { try { return JSON.parse(fs.readFileSync(path.join(FIXTURES, f), 'utf8')); } catch { return null; } });
const records = parsed.filter((r) => r && r.schema === 'rust-bench/v1').sort((a, b) => Date.parse(a.generated_at) - Date.parse(b.generated_at));
const answer = {
  schema: 'rust-bench/v1', count: records.length, latest_id: records.at(-1).id, records, dir: 'research/results', repo_url: 'https://github.com/korczis/OpenTTD',
  skipped: files.filter((_, i) => !parsed[i] || parsed[i].schema !== 'rust-bench/v1').map((file) => ({ file, error: 'schema: off the contract' })),
};
let served = answer;
const routes = (url) => {
  if (url === '/api/bench') return served;
  return respond({ error: 'no route' }, { status: 404 });
};

// The page module and its server-side renders first, before the fake browser replaces the globals
// ECharts measures text with.
const bench = await import('../static/bench.js');
const render = (option) => {
  const c = echartsSsr.init(null, null, { renderer: 'svg', ssr: true, width: 800, height: 400 });
  c.setOption({ animation: false, ...option });
  const svg = c.renderToSVGString();
  c.dispose();
  return svg;
};
const ssrHistory = render(bench.historyOption(bench.history(records, 'planner/plan_fixture'), {}));
const ssrTrend = render(bench.trendOption(records, ['planner/plan_fixture', 'rhai/eval_small']));
const env = fakeDom({ url: 'http://127.0.0.1:8800/bench?sort=-median', routes });
const $ = env.byId;
const chart = (id) => env.charts.find((c) => c.box === $(id));
const param = (k) => new URL(env.location.href).searchParams.get(k);

test('the fixtures hold three records, oldest first, and two files off the contract', () => {
  assert.deepEqual(records.map((r) => r.id), ['bench-20261120T101500-aa11bb22', 'bench-20261121T101500-cc33dd44', 'bench-20261122T231500-ab12cd34']);
  assert.equal(answer.skipped.length, 2);
});

test('durations read in their own unit; ratios as changes; links escape each step', () => {
  assert.equal(bench.human(950), '950 ns');
  assert.equal(bench.human(120.1), '120 ns');
  assert.equal(bench.human(12345), '12.3 µs');
  assert.equal(bench.human(4560000), '4.56 ms');
  assert.equal(bench.human(1.2e9), '1.20 s');
  assert.equal(bench.human(null), '-');
  assert.equal(bench.human(3, 'ops'), '3 ops');
  assert.deepEqual(bench.scaleOf(0), { div: 1, unit: 'ns' });
  assert.equal(bench.pct(1.264), '+26.4%');
  assert.equal(bench.pct(0.9), '-10.0%');
  assert.equal(bench.pct(null), '-');
  assert.equal(bench.benchHref('planner/plan fixture?x'), '/bench/planner/plan%20fixture%3Fx');
  assert.equal(bench.recordHref('bench-1-a'), '/bench?rec=bench-1-a');
  assert.equal(bench.commitUrl('abc', 'https://github.com/o/r'), 'https://github.com/o/r/commit/abc');
  assert.equal(bench.commitUrl('abc', null), null, 'no commit page without a known remote');
  assert.equal(bench.shortTime('2026-11-22T23:15:00+00:00'), '2026-11-22 23:15');
  assert.equal(bench.shortTime('later'), 'later');
  assert.equal(bench.throughputText({ elements: 64 }), '64 elements');
  assert.equal(bench.throughputText(5), '5');
  assert.equal(bench.throughputText(null), '');
});

test('the address holds the group, text, sort and record; an unknown sort falls back', () => {
  const s = bench.getState('http://x/bench?group=rhai&q=eval&sort=-median&rec=bench-1-a&max=bench-trend');
  assert.deepEqual(s, { group: 'rhai', q: 'eval', sort: '-median', rec: 'bench-1-a', max: 'bench-trend' });
  assert.equal(bench.getState('http://x/bench?sort=nonsense').sort, 'group');
});

test('rows: change against the previous record, the regression flagged, filtered and sorted', () => {
  const [, prev, last] = records;
  const all = bench.rows(last, prev, {});
  assert.deepEqual(all.map((b) => b.id), ['planner/plan_fixture', 'planner/route_search', 'rhai/compile_library', 'rhai/eval_small', 'store/sample_rows']);
  const plan = all[0];
  assert.ok(plan.regressed);
  assert.ok(Math.abs(plan.change - 120.1 / 95) < 1e-9);
  assert.equal(bench.rows(records[1], records[0], {}).find((b) => b.id === 'store/sample_rows').change, null, 'new in its record: no change');
  assert.deepEqual(bench.rows(last, prev, { group: 'rhai' }).map((b) => b.name), ['compile_library', 'eval_small']);
  assert.deepEqual(bench.rows(last, prev, { q: 'ROUTE' }).map((b) => b.id), ['planner/route_search']);
  assert.deepEqual(bench.rows(last, prev, { sort: '-median' }).map((b) => b.id)[0], 'rhai/compile_library');
  assert.deepEqual(bench.rows(last, prev, { sort: 'median' }).map((b) => b.id)[0], 'planner/plan_fixture');
  assert.equal(bench.rows(last, null, { sort: '-change' }).at(-1).change, null);
  assert.equal(bench.rows(last, prev, { sort: '-change' })[0].id, 'planner/plan_fixture', 'the worst change first');
  assert.deepEqual(bench.groups(last), ['planner', 'rhai', 'store']);
  assert.deepEqual(bench.rows(null, null), []);
  assert.equal(bench.selected(records, 'nope'), records.at(-1));
  assert.equal(bench.selected([], ''), null);
  assert.equal(bench.previous(records, records[0]), null);
  // Another machine's record is skipped: its numbers are no change of this one's.
  const other = { ...records[1], id: 'bench-x-other', host: { name: 'elsewhere' } };
  const mixed = [records[0], other, records[2]];
  assert.equal(bench.previous(mixed, records[2]), records[0]);
  assert.equal(bench.previous(mixed, other), null);
  assert.ok(!bench.comparable(records[0], { ...records[0], crate: 'tools/other' }), 'another crate does not compare');
  assert.ok(bench.comparable({}, {}), 'two records without host or crate compare');
  assert.deepEqual(bench.rows({ benches: [{ id: 'solo', median: 1, unit: 'ns' }] }, null).map((b) => [b.group, b.name]), [['solo', '']], 'a bench without group and name derives them from its id');
});

test('history: a bench across the records that measured it, its regression attached', () => {
  const h = bench.history(records, 'store/sample_rows');
  assert.equal(h.length, 2, 'measured from the second record on');
  const p = bench.history(records, 'planner/plan_fixture');
  assert.deepEqual(p.map((x) => !!x.regression), [false, false, true]);
  assert.deepEqual(bench.history(records, 'no/such'), []);
});

test('the history option: median, the min-max band stacked under it, regressions as triangles', () => {
  const points = bench.history(records, 'planner/plan_fixture');
  const o = bench.historyOption(points, { sel: records[1].id });
  assert.deepEqual(o.series.map((s) => s.id), ['band-low', 'band', 'median', 'regressions']);
  assert.deepEqual(o.series[0].data, [84, 88, 110]);
  assert.deepEqual(o.series[1].data, [17, 16, 30], 'the band is max - min, stacked on min');
  assert.equal(o.series[0].stack, o.series[1].stack);
  assert.deepEqual(o.series[2].data.map((d) => d.value), [90, 95, 120.1]);
  assert.equal(o.series[2].data[1].symbolSize, 16, 'the record in the address is drawn larger');
  assert.deepEqual(o.series[3].data, [{ value: [records[2].id, 120.1], name: records[2].id, ratio: 1.264 }]);
  assert.equal(o.yAxis.name, 'ns');
  assert.equal(o.xAxis.axisLabel.formatter(records[0].id), '2026-11-20 10:15');
  const tip = o.tooltip.formatter([{ axisValue: records[2].id }]);
  assert.match(tip, /median 120 ns ± 3\.20 ns/);
  assert.match(tip, /regression \+26\.4%/);
  assert.equal(o.tooltip.formatter([{ axisValue: 'nope' }]), '');
  const ms = bench.historyOption(bench.history(records, 'rhai/compile_library'));
  assert.equal(ms.yAxis.name, 'ms', 'a slow bench reads in ms');
  assert.deepEqual(ms.series[2].data.map((d) => d.value), [4.52, 4.48, 4.47]);
  const other = bench.historyOption([{ record: records[0], bench: { id: 'x/y', unit: 'ops', median: 5 }, regression: null }]);
  assert.equal(other.yAxis.name, 'ops');
  assert.deepEqual(other.series[1].data, [0], 'without min and max the band is the median itself');
  assert.match(ssrHistory, /<svg/);
  assert.match(ssrHistory, /<path/, 'the server-side render draws the lines');
});

test('the trend option: each bench relative to its first median, a line per bench carrying its id', () => {
  const o = bench.trendOption(records, ['planner/plan_fixture', 'store/sample_rows'], { sel: records[1].id });
  assert.deepEqual(o.series.map((s) => s.id), ['planner/plan_fixture', 'store/sample_rows']);
  assert.deepEqual(o.series[0].data, [1, Number((95 / 90).toFixed(4)), Number((120.1 / 90).toFixed(4))]);
  assert.deepEqual(o.series[1].data, [null, 1, Number((805 / 812).toFixed(4))], 'a bench not yet measured is a gap');
  assert.deepEqual(o.series[0].markLine.data, [{ xAxis: records[1].id }], 'the selected record is marked');
  assert.equal(o.series[1].markLine, undefined);
  assert.equal(o.tooltip.valueFormatter(1.5), '1.500×');
  assert.equal(o.tooltip.valueFormatter(null), '-');
  assert.equal(o.series[0].endLabel.formatter({ value: 1.25 }), 'planner/plan_fixture 1.25×');
  assert.equal(o.xAxis.axisLabel.formatter(records[2].id), '2026-11-22 23:15');
  const many = bench.trendOption(records, Array.from({ length: 9 }, (_, i) => `g/b${i}`));
  assert.equal(many.grid.right, 24, 'nine lines: no end labels, no room kept for them');
  assert.match(ssrTrend, /<path/);
});

test('the empty state names where records come from and lists what was skipped', () => {
  const html = bench.emptyHtml({ dir: 'research/results', skipped: [{ file: 'bench-x.json', error: 'schema: <bad>' }] });
  assert.match(html, /No benchmark record yet/);
  assert.match(html, /research\/results\/bench-\*\.json/);
  assert.match(html, /bench-x\.json<\/code>: schema: &lt;bad&gt;/, 'the reason is escaped');
  assert.doesNotMatch(bench.emptyHtml({}), /skipped/);
  assert.equal(bench.regressionsHtml({ baseline: null }), '');
  assert.match(bench.regressionsHtml({ benches: [], baseline: { id: null, regressions: [{ id: 'a/b', median: 2, baseline_median: 1, ratio: 2 }] } }), /1 regression against the baseline<\/span>/);
  const plain = bench.recordHtml({ rev: 'abc', benches: [] });
  assert.match(plain, /none/);
  assert.match(plain, /the first record of its series/);
  assert.doesNotMatch(plain, /Dirty tree/);
});

test('/bench: banner, record, table and trend chart; filter, sort and record live in the address', async () => {
  await bench.listPage();
  await env.settle();
  // The regression banner of the latest record, linked to its bench and its baseline.
  assert.match($('bench-regressions').innerHTML, /1 regression against the baseline/);
  assert.match($('bench-regressions').innerHTML, /href="\/bench\/planner\/plan_fixture"/);
  assert.match($('bench-regressions').innerHTML, /href="\/bench\?rec=bench-20261121T101500-cc33dd44"/);
  assert.match($('bench-regressions').innerHTML, /1\.264× \(\+26\.4%\)/);
  // The record: rev to its commit, host, toolchain, dirty tree warned.
  const rec = $('bench-record').innerHTML;
  assert.match(rec, /href="https:\/\/github\.com\/korczis\/OpenTTD\/commit\/3{40}"/);
  assert.match(rec, /fixture-host/);
  assert.match(rec, /rustc 1\.91\.0 \(fixture\)/);
  assert.match(rec, /Dirty tree/);
  assert.equal($('rec').value, records[2].id);
  // The table, sorted by the address (-median): the slowest first, each name its page.
  const order = () => [...$('bench-rows').innerHTML.matchAll(/data-bench="([^"]+)"/g)].map((m) => m[1]);
  assert.equal(order()[0], 'rhai/compile_library');
  assert.match($('bench-rows').innerHTML, /href="\/bench\/store\/sample_rows"/);
  assert.match($('bench-rows').innerHTML, /64 elements per iteration/);
  assert.match($('bench-rows').innerHTML, /4\.47 ms/);
  assert.match($('bench-head').innerHTML, /aria-sort="descending"/);
  assert.match($('bench-skipped').innerHTML, /2 files skipped/);
  // Sort: a header click toggles, and the address follows.
  await $('bench-head').querySelector('[data-sort="name"]').click();
  assert.equal(param('sort'), 'name');
  assert.deepEqual(order(), ['rhai/compile_library', 'rhai/eval_small', 'planner/plan_fixture', 'planner/route_search', 'store/sample_rows']);
  await $('bench-head').querySelector('[data-sort="name"]').click();
  assert.equal(param('sort'), '-name');
  await $('bench-head').querySelector('[data-sort="group"]').click();
  assert.equal(param('sort'), null, 'the default sort leaves the address');
  await $('bench-head').click();
  // Group: the select and a group cell both filter.
  $('group').value = 'rhai';
  await $('group').onchange({ target: $('group') });
  assert.equal(param('group'), 'rhai');
  assert.deepEqual(order(), ['rhai/compile_library', 'rhai/eval_small']);
  assert.match($('bench-n').textContent, /2 of 5/);
  await env.tick();
  const c = chart('bench-trend');
  assert.ok(c, 'the trend chart is drawn through ecInit');
  assert.deepEqual(c.getOption().series.map((s) => s.id), ['rhai/compile_library', 'rhai/eval_small'], 'the chart follows the filter');
  c.handlers.click({ seriesId: 'rhai/eval_small' });
  assert.equal(env.location.pathname, '/bench/rhai/eval_small', 'a line leads to its bench');
  env.location.href = 'http://127.0.0.1:8800/bench?group=rhai';
  await $('bench-rows').querySelector('[data-group="rhai"]').click();
  assert.equal(param('group'), 'rhai');
  await $('bench-rows').click();
  // Text.
  $('q').value = 'eval';
  await $('q').oninput({ target: $('q') });
  assert.equal(param('q'), 'eval');
  assert.deepEqual(order(), ['rhai/eval_small']);
  $('q').value = 'nothing-matches';
  await $('q').oninput({ target: $('q') });
  assert.match($('bench-rows').innerHTML, /no bench matches the filter/);
  $('q').value = '';
  await $('q').oninput({ target: $('q') });
  // Record: an older one, whose baseline had no regression; back to the latest clears it.
  $('rec').value = records[1].id;
  await $('rec').onchange({ target: $('rec') });
  assert.equal(param('rec'), records[1].id);
  assert.equal($('bench-regressions').innerHTML, '');
  assert.doesNotMatch($('bench-record').innerHTML, /Dirty tree/);
  assert.match($('bench-record').innerHTML, /0 regressions/);
  assert.deepEqual(chart('bench-trend').getOption().series[0].markLine.data, [{ xAxis: records[1].id }]);
  $('rec').value = records[2].id;
  await $('rec').onchange({ target: $('rec') });
  assert.equal(param('rec'), null);
  // Maximize keeps max.
  const max = $('bench-trend-card').querySelector('[data-toolbar]').kids.find((k) => k.localName === 'button');
  await max.click();
  assert.equal(param('max'), 'bench-trend');
  await env.document.dispatch('keydown', { key: 'Escape' });
  assert.equal(param('max'), null, 'Escape restores');
});

test('/bench: a group in the address the record lacks is kept; a failed load says so', async () => {
  env.location.href = 'http://127.0.0.1:8800/bench?group=gone';
  await bench.listPage();
  assert.match($('group').innerHTML, /<option value="gone" selected>/);
  assert.match($('bench-rows').innerHTML, /no bench matches the filter/);
  served = respond({ error: 'boom' }, { status: 500 });
  await bench.listPage();
  assert.match($('page').innerHTML, /did not load: boom/);
  env.location.href = 'http://127.0.0.1:8800/bench/planner/plan_fixture';
  await bench.benchPage('planner/plan_fixture');
  assert.match($('page').innerHTML, /did not load: boom/, 'a bench page shows the failure, not "no record"');
  assert.doesNotMatch($('page').innerHTML, /No record measured/);
  served = { ...answer, repo_url: null };
  await bench.benchPage('planner/plan_fixture');
  assert.doesNotMatch($('bench-records').innerHTML, /\/commit\//, 'without a remote a rev is not a link');
  assert.match($('bench-records').innerHTML, /<span class="font-mono">1111111111<\/span>/);
  env.location.href = 'http://127.0.0.1:8800/bench';
  await bench.listPage();
  assert.doesNotMatch($('bench-record').innerHTML, /\/commit\//);
  served = { schema: 'rust-bench/v1', count: 0, latest_id: null, records: [], skipped: [], dir: 'research/results' };
  await bench.listPage();
  assert.match($('page').innerHTML, /No benchmark record yet/);
  served = answer;
});

test('/bench/<group>/<name>: stats, history chart, record rows and the selection in the address', async () => {
  env.location.href = 'http://127.0.0.1:8800/bench/planner/plan_fixture';
  await bench.benchPage('planner/plan_fixture');
  await env.settle();
  assert.match($('bench-stats').innerHTML, /120 ns/);
  assert.match($('bench-stats').innerHTML, /\+26\.4%/);
  assert.match($('bench-regression').innerHTML, /1 regression against the baseline/);
  assert.match($('bench-records').innerHTML, /href="https:\/\/github\.com\/korczis\/OpenTTD\/commit\/1{40}"/);
  assert.match($('bench-records').innerHTML, /regression \+26\.4%/);
  assert.match($('bench-records').innerHTML, /no baseline/);
  assert.match($('bench-records').innerHTML, /dirty/);
  assert.match($('bench-siblings').innerHTML, /href="\/bench\/planner\/route_search"/, 'the group\'s other benches are linked');
  assert.match($('bench-selected').innerHTML, /href="\/bench\?rec=bench-20261122T231500-ab12cd34"/);
  await env.tick();
  const c = chart('bench-history');
  assert.ok(c, 'the history is drawn through ecInit');
  assert.deepEqual(c.getOption().series.map((s) => s.id), ['band-low', 'band', 'median', 'regressions']);
  // A click on a record selects it: the address, the stats and the rows follow.
  c.handlers.click({ data: { name: records[0].id } });
  assert.equal(param('rec'), records[0].id);
  assert.match($('bench-stats').innerHTML, /90\.0 ns/);
  assert.match($('bench-stats').innerHTML, /the first record of this host and crate that measured it/);
  assert.equal($('bench-regression').innerHTML, '');
  assert.equal(chart('bench-history').getOption().series[2].data[0].symbolSize, 16);
  c.handlers.click({ name: 'not-a-record' });
  assert.equal(param('rec'), records[0].id, 'a click off the records changes nothing');
  await $('bench-records').querySelector(`[data-rec="${records[2].id}"]`).click();
  assert.equal(param('rec'), null, 'the latest record is the default and leaves the address');
  await $('bench-records').click();
  const max = $('bench-history-card').querySelector('[data-toolbar]').kids.find((k) => k.localName === 'button');
  await max.click();
  assert.equal(param('max'), 'bench-history');
  await env.document.dispatch('keydown', { key: 'Escape' });
  assert.equal(param('max'), null);
});

test('/bench/<group>/<name>: a group of one bench, and an unknown bench', async () => {
  env.location.href = 'http://127.0.0.1:8800/bench/store/sample_rows';
  await bench.benchPage('store/sample_rows');
  assert.match($('bench-siblings').innerHTML, /no other bench in this group/);
  assert.match($('bench-stats').innerHTML, /64 elements/);
  await bench.benchPage('no/such');
  assert.match($('page').innerHTML, /No record measured the bench <code>no\/such<\/code>/);
});

test('the module mounts the page its address names, as the browser loads it', async () => {
  env.location.href = 'http://127.0.0.1:8800/bench';
  await import('../static/bench.js?mount=list');
  await env.settle();
  assert.match($('bench-regressions').innerHTML, /1 regression against the baseline/);
  env.location.href = 'http://127.0.0.1:8800/bench/rhai/compile_library';
  await import('../static/bench.js?mount=one');
  await env.settle();
  assert.match($('bench-stats').innerHTML, /4\.47 ms/);
  assert.match($('crumbs').innerHTML, /rhai\/compile_library/);
  env.location.href = 'http://127.0.0.1:8800/elsewhere';
  await import('../static/bench.js?mount=none');
});
