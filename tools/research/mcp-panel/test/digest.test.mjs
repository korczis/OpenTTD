// node --test tools/research/mcp-panel/test/ : the Digest page (static/digest.js). Its pure helpers
// read answers shaped like src/panel_digest.rs's (GET /api/digest); the page is then mounted in the
// fake browser of fake-dom.mjs. Fictional companies; nothing here talks to a panel or a game.
import './panel-ui.mjs'; // first: shell.js publishes PanelUI before any page script
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeDom, respond } from './fake-dom.mjs';

const digest = {
  company: 0, name: 'Ours <b>', date: '1954-04-01', period: 'quarter', label: '1954-Q1', level: 'alert', owner: true,
  standing: { rank: 2, of: 3, share: 0.3, share_change: -0.1, leader: 'Big', leader_changed: false, vehicles: 1, largest_fleet: 140, largest_fleet_name: 'Big', we_lead: false },
  series: [{ month: '1953-12', share: 0.4, leader: 'Big', rank: 2, of: 3 }, { month: '1954-03', share: 0.3, leader: 'Big', rank: 2, of: 3 }],
  by_type: [{ type: 'train', ours_per_vehicle: 900, leader_per_vehicle: 30000, leader: 'Big' }], by_type_source: 'replan', by_type_date: '1954-01-05',
  profit: { quarter: 30000, previous: 20000, change: 10000 },
  money: { cash: 90000, loan: 300000, max_loan: 300000, idle_cash: null },
  failed: ['service arena was given up', 'replan did not switch: no line pays'],
  changes: { adopt: false, why: 'no line pays', proposed: ['fr_from', 'fr_to'], portfolio: ['rail A -> B'], date: '1954-01-05' },
  approvals: { pending: 1, soonest_days: 5, items: [{ id: 5, what: 'ai.set_setting fr_from=3', age_days: 10, countdown_days: 5, verdict: 'keep' }], since: { approved: 1, denied: 2, by_deadline: 2 } },
  alerts: [{ key: 'fleet_gap', level: 'alert', text: '1 vehicles against Big\'s 140' }, { key: 'behind', level: 'watch', text: '30% of Big\'s value' }],
  new_alerts: ['fleet_gap'], cleared: ['stalled'], unknown: [],
};
const answer = {
  company: 0, requested_missing: null, digest,
  companies: [{ company: 0, name: 'Ours <b>', level: 'alert' }, { company: 3, name: 'Second', level: 'ok' }],
  history: { week: [], month: [{ label: '1954-02', level: 'watch', rank: 2, of: 3, share: 0.32, leader: 'Big', vehicles: 1, largest_fleet: 140, pending: 0, alerts: [] },
    { label: '1954-03', level: 'alert', rank: 2, of: 3, share: 0.3, leader: 'Big', vehicles: 1, largest_fleet: 140, pending: 1, alerts: ['fleet_gap'] }],
  quarter: [{ label: '1954-Q1', level: 'alert', rank: 2, of: 3, share: 0.3, alerts: ['fleet_gap'] }] },
  alerts: [], heartbeat: { alive: true, age_s: 4 },
};
const routes = (url) => {
  if (url === '/api/digest' || url === '/api/digest?company=0') return answer;
  if (url === '/api/digest?company=3') return { ...answer, company: 3, digest: { ...digest, company: 3, name: 'Second', owner: false } };
  return respond({ error: 'no route' }, { status: 404 });
};

const p = await import('../static/digest.js');

test('the address holds the company, the period and a maximized chart', () => {
  assert.deepEqual(p.getState('http://x/digest?company=3&period=quarter&max=share-chart'), { company: 3, period: 'quarter', max: 'share-chart' });
  assert.deepEqual(p.getState('http://x/digest?company=x&period=year'), { company: undefined, period: 'month', max: undefined });
});

test('pure helpers: tones, percent, points and the chart options', () => {
  assert.equal(p.levelTone('alert'), 'red');
  assert.equal(p.levelTone('unknown'), 'gray');
  assert.equal(p.pct(0.555), '56%');
  assert.equal(p.pct(null), '–');
  assert.equal(p.pts(-0.1), '-10 pt');
  assert.equal(p.pts(0.05), '+5 pt');
  const s = p.shareOption(digest.series);
  assert.deepEqual(s.xAxis.data, ['1953-12', '1954-03']);
  assert.deepEqual(s.series[0].data.map((d) => d.value), [40, 30]);
  assert.deepEqual(s.series[0].markLine.data.map((m) => m.yAxis), [50, 25]);
  assert.match(s.tooltip.formatter([{ data: s.series[0].data[1] }]), /1954-03<br>30% of Big · rank 2 of 3/);
  assert.equal(s.tooltip.formatter([]), '');
  assert.equal(s.yAxis.max({ max: 130 }), 130);
  const t = p.typesOption(digest.by_type, { dark: false });
  assert.deepEqual(t.series.map((x) => x.data[0].value), [900, 30000]);
  assert.deepEqual(p.CHARTS.map((c) => c.id), ['share-chart', 'types-chart']);
  assert.ok(p.CHARTS.every((c) => c.params.length && c.drill));
});

test('renderers escape, say the countdown and verdict, and mark what is unknown', () => {
  const h = p.headHtml(answer);
  assert.match(h, /Ours &lt;b&gt;: 1954-Q1/);
  assert.match(h, /data-alert="fleet_gap"[\s\S]*new/);
  assert.match(h, /Cleared: stalled/);
  assert.match(h, /service alive \(4 s ago\)/);
  assert.match(p.headHtml({ ...answer, requested_missing: 9 }), /Company 9 has no digest/);
  const none = p.headHtml({ companies: [], heartbeat: { alive: false, age_s: null, why: 'the digest service has not run' } });
  assert.match(none, /No digest yet[\s\S]*service not running[\s\S]*has not run/);
  assert.match(p.headHtml({ ...answer, heartbeat: { alive: false, age_s: 300 }, digest: { ...digest, unknown: ['series'] } }), /service stale \(300 s\)[\s\S]*Unknown \(source missing or stale\): series/);
  const st = p.standingHtml(digest);
  assert.match(st, /2 of 3[\s\S]*30%[\s\S]*-10 pt[\s\S]*1 vs 140[\s\S]*30,000[\s\S]*\+10,000/);
  assert.match(p.standingHtml({ ...digest, standing: { ...digest.standing, we_lead: true, runner_up: 'Big', leader_changed: true }, money: { idle_cash: 5000, reinvestment: 0.12 }, profit: {}, approvals: {} }), /leading Big[\s\S]*idle cash 5,000 \(gap watch\); reinvested 12% of cash flow/);
  assert.match(p.standingHtml({ ...digest, standing: null }), /not among the companies/);
  assert.equal(p.standingHtml(null), '');
  assert.match(p.failedHtml(digest), /arena was given up/);
  assert.match(p.failedHtml({ failed: [] }), /Nothing failed[\s\S]*Nobody changed governance/);
  const gov = p.failedHtml({ failed: [], governance: [{ setting: 'auto_approve', from: false, to: true, via: 'POST /api/auto-approve', credential: 'none', origin: '<x>', user_agent: 'curl/8', peer: '', game_date: '1954-04-20' }] });
  assert.match(gov, /Who changed governance[\s\S]*auto_approve false -&gt; true via POST \/api\/auto-approve on 1954-04-20 \(credential none, Origin &lt;x&gt;, UA curl\/8\)/);
  assert.equal(p.governanceLine({ kind: 'governance.changed' }), 'governance.changed ? -> ? via ?');
  assert.match(p.failedHtml({ failed: [] }), /service_eval has not run/);
  const ev = p.evaluationHtml({ caveat: 'associative, not causal: x', poor: [{ entry: 'replan.propose', verdict: 'poor', streak: 4 }, { entry: 'a.b', verdict: 'unclear' }],
    coverage: [{ service: 'replan', reacted: 7, episodes: 255, share: 7 / 255 }, { service: 'w', reacted: 0, episodes: 0, share: null }] });
  assert.match(ev, /associative, not causal[\s\S]*replan\.propose is persistently poor \(streak 4\)[\s\S]*a\.b is persistently unclear<[\s\S]*replan reacted to 7 of 255 deterioration episodes \(2\.7%\)[\s\S]*w reacted to 0 of 0 deterioration episodes</);
  assert.match(p.evaluationHtml({ poor: [], coverage: [] }), /associative, not causal[\s\S]*Nothing poor/);
  const c = p.changesHtml(digest);
  assert.match(c, /hold: no line pays[\s\S]*2 setting\(s\): fr_from, fr_to[\s\S]*1 approved, 2 denied, 2 settled by the deadline/);
  assert.match(c, /data-approval="5"[\s\S]*fr_from=3[\s\S]*5 d[\s\S]*keep/);
  assert.match(p.changesHtml({ changes: { adopt: true, proposed: Array.from({ length: 13 }, (_, i) => `s${i}`) }, approvals: { items: [{ id: 1, what: 'x', age_days: 1, countdown_days: null, verdict: 'approve' }] } }), /switch[\s\S]*13 setting\(s\)[\s\S]*, …[\s\S]*no deadline/);
  assert.equal(p.failedHtml(null) + p.changesHtml(null), '');
  const hist = p.historyHtml(answer.history, 'month', '1954-02');
  assert.ok(hist.indexOf('1954-03') < hist.indexOf('1954-02'), 'newest first');
  assert.match(hist, /data-label="1954-02" aria-current="true"/);
  assert.match(p.historyHtml(answer.history, 'week'), /No week digest yet/);
});

test('/digest mounts, draws both charts, follows a click and keeps the period in the address', async () => {
  const env = fakeDom({ url: 'http://127.0.0.1:8800/digest?company=0', routes });
  const $ = env.byId;
  await p.mount();
  await env.settle();
  assert.match($('head').innerHTML, /fleet_gap/);
  assert.match($('standing').innerHTML, /Share of the leader/);
  assert.match($('changes').innerHTML, /keep/);
  assert.equal(env.charts.length, 2, 'two charts through the one door');
  const share = env.charts.find((c) => c.box === $('share-chart'));
  assert.deepEqual(share.getOption().xAxis.data, ['1953-12', '1954-03']);
  share.handlers.click({ data: { month: '1954-02' } });
  assert.equal(new URL(env.location.href).searchParams.get('period'), 'month');
  assert.match($('history').innerHTML, /data-label="1954-02" aria-current="true"/);
  share.handlers.click({ data: {} });
  const sel = $('h-period');
  sel.value = 'quarter';
  sel.onchange({ target: sel });
  assert.equal(new URL(env.location.href).searchParams.get('period'), 'quarter');
  assert.match($('history').innerHTML, /1954-Q1/);
  const types = env.charts.find((c) => c.box === $('types-chart'));
  types.handlers.click({});
  assert.match(env.location.href, /\/api\/rhai\/shared\?key=replan\.0$/);
  await p.load({ company: 3, period: 'month' });
  assert.match($('head').innerHTML, /Second: 1954-Q1/);
  await p.load({ company: 9, period: 'month' });
  assert.match($('head').dataset.failure || '', /GET \/api\/digest: 404/);
  await p.load({ company: 0, period: 'week' });
  assert.match($('types-chart-note').textContent, /from replan on 1954-01-05/);
});

test('without a replan comparison the types chart says unknown', async () => {
  const env = fakeDom({ url: 'http://127.0.0.1:8800/digest', routes: () => ({ ...answer, digest: { ...digest, by_type_source: 'stale' } }) });
  await p.mount();
  await env.settle();
  assert.match(env.byId('types-chart-note').textContent, /Unknown: stale/);
});
