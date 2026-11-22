// node --test tools/research/mcp-panel/test/ : the Strategy page (static/strategy.js). Its pure
// helpers read answers shaped like src/panel_strategy.rs's (GET /api/strategy, GET /api/blackboard);
// the page is then mounted in the fake browser of fake-dom.mjs. Fictional towns and lines; nothing
// here talks to a panel or a game.
import './panel-ui.mjs'; // first: shell.js publishes PanelUI before any page script
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeDom, respond } from './fake-dom.mjs';

const line = { kind: 'freight', cargo: 'Coal', from: 'Mine <b>', to: 'Plant', mode: 'freight_rail', score: 4000, investment: 80000, monthly_profit: 9000, track: null };
const summary = { cycle: 2, game_date: '1951-02-01', trigger: 'month', lines: 1, total_score: 4000, solver: 'builtin knapsack (MCP_ALGOGLYPH_DIR not set or the runner refused)',
  investment_lines: 0, town_warnings: 1, tuning_proposals: 2, duration_ms: 812,
  algoglyph: { available: false, dir: '/nowhere', runner: 'none: the built-in DP decides, and investment and track are skipped' } };
const strategy = {
  enabled: true, thread: true, running: false, cycles: 2, game: true, next_run_in_s: 240,
  config: { every_s: 300, min_gap_s: 30, workers: 3 },
  status: { state: 'ok', reason: 'the last cycle completed', notes: [], mesh: { records_skipped: 'no MAJORDOMUS_SESSION', announce: { claim: 'openttd:panel-8800:strategy', announced: true } } },
  summary,
  last: { cycle: 2, duration_ms: 812, parts: { plan: { ok: true, ms: 700 }, track: { ok: true, ms: 0, skipped: 'no AlgoGlyph checkout' }, towns: { ok: false, ms: 5, error: 'town_lib: boom' }, tuning: { ok: true, ms: 1 } } },
  plan: { portfolio: { solver: summary.solver, total_score: 4000, investment_k: 80, budget_k: 300, chosen: [line] } },
  investment: { skipped: 'no AlgoGlyph checkout at /nowhere: transport-investment needs one' },
  track: { skipped: 'no AlgoGlyph checkout: track-planning needs one' },
  towns: { towns: 2, rated_companies: [0], tolerance: 'hostile', tolerance_source: 'console', warning_count: 1,
    companies: [{ company: 0, warnings: [{ town: 'Aville', rating: -260, band: 'TOWN_RATING_VERY_POOR', refuses_stations: true, advice: [{ what: 'plant 9 tree tiles near the town', why: 'rating -260', source: 'src/tree_cmd.cpp' }] }] }] },
  tuning: { rows: 218, distinct_settings: 40, file: '/h/brain.json', mtime: 1, proposals: [{ kind: 'safe', predicted_share: 1.29, spread: 0.05, settings: 'own_lines=10' }, { kind: 'explore', predicted_share: 1.2, spread: 0.28, settings: 'own_lines=12' }] },
  bot_settings_suggestion: { text: 'fr_from=3,fr_to=7', data_only: 'a suggestion: never applied' },
  peers: { source: 'mesh.events', reason: 'typed records are not served here', records: [], intents: [{ session: 'openttd-7e', intent: 'battle', via: 'claim_acquired', at: 1 }] },
};
const entry = (key, value, by = 'strategist') => ({ key, value, by, revision: 3, written_at: 1, game_date: '1951-02-01', history_len: 2 });
const board = { count: 2, reserved: ['strategy.', 'mesh.'], history_kept: 8, entries: [entry('rail.hint', { prefer: 'coal' }, 'my_script'), entry('strategy.summary', summary)] };
const routes = (url) => {
  if (url === '/api/strategy') return strategy;
  if (url === '/api/strategy/run') return { cycle: 3, duration_ms: 900 };
  if (url.startsWith('/api/blackboard?key=rail.hint')) return { ...entry('rail.hint', { prefer: 'coal' }, 'my_script'), history: [entry('rail.hint', { prefer: 'oil' }, 'my_script')] };
  if (url.startsWith('/api/blackboard?key=')) return respond({ error: 'no key' }, { status: 404 });
  if (url.startsWith('/api/blackboard')) return board;
  return respond({ error: 'no route' }, { status: 404 });
};

const s = await import('../static/strategy.js');

test('the headline and the parts read the summary and the last cycle', () => {
  assert.match(s.headline(summary), /Cycle 2 at 1951-02-01 \(month\): 1 lines, score 4,000, by builtin knapsack/);
  assert.match(s.headline(null), /No strategy yet/);
  assert.deepEqual(s.parts(strategy.last).map((p) => [p.name, p.ok, p.note]),
    [['plan', true, ''], ['track', true, 'no AlgoGlyph checkout'], ['towns', false, 'town_lib: boom'], ['tuning', true, '']]);
  assert.equal(s.stateTone('error'), 'red');
  assert.equal(s.preview({ a: 'x'.repeat(200) }).length, 120);
  assert.equal(s.ago(100, 112), '12 s');
});

test('portfolio, towns, tuning and peers render escaped, with skips said', () => {
  const pf = s.portfolioHtml('Portfolio', strategy.plan, 'strategy');
  assert.match(pf, /Mine &lt;b&gt;/);
  assert.match(pf, /builtin knapsack/);
  assert.match(s.portfolioHtml('Investment', strategy.investment, 'strategy'), /transport-investment needs one/);
  assert.match(s.townsHtml(strategy.towns), /Aville[\s\S]*refuses stations[\s\S]*plant 9 tree tiles/);
  const t = s.tuningHtml(strategy.tuning);
  assert.match(t, /Tuning proposals \(BattleBrain\)/);
  assert.match(t, /explore[\s\S]*1\.2[\s\S]*0\.28/);
  assert.match(s.tuningHtml({ skipped: '/h/brain.json: No such file' }), /No such file/);
  const p = s.peersHtml(strategy.peers, strategy.status.mesh);
  assert.match(p, /openttd-7e[\s\S]*battle/);
  assert.match(p, /announce openttd:panel-8800:strategy: announced/);
  assert.deepEqual(s.warnings(strategy.towns).map((w) => w.company), [0]);
});

test('the address holds the prefix and the open key', () => {
  assert.deepEqual(s.getState('http://x/strategy?prefix=strategy.&key=strategy.plan'), { prefix: 'strategy.', key: 'strategy.plan' });
  assert.deepEqual(s.getState('http://x/strategy'), { prefix: '', key: undefined });
});

test('/strategy mounts every section, opens a key and runs a cycle on demand', async () => {
  const env = fakeDom({ url: 'http://127.0.0.1:8800/strategy', routes });
  const $ = env.byId;
  await s.mount();
  await env.settle();
  assert.match($('status').innerHTML, /Strategist[\s\S]*town_lib: boom/);
  assert.match($('summary').textContent, /Cycle 2/);
  assert.match($('plan').innerHTML, /Plant/);
  assert.match($('tuning').innerHTML, /BattleBrain/);
  assert.match($('board').innerHTML, /rail\.hint[\s\S]*my_script[\s\S]*strategy\.summary/);
  const btn = document.querySelector('#board button[data-key]');
  await btn.onclick();
  await env.settle();
  assert.match($('detail').innerHTML, /rail\.hint[\s\S]*History[\s\S]*oil/);
  assert.equal(new URL(env.location.href).searchParams.get('key'), 'rail.hint');
  await $('s-run').onclick();
  await env.settle();
  const run = env.fetches.find((f) => f.url === '/api/strategy/run');
  assert.equal(run.init.method, 'POST');
  assert.deepEqual(JSON.parse(run.init.body), { wait: true });
  assert.match($('s-run-msg').textContent, /cycle 3/);
  const prefix = $('b-prefix');
  prefix.value = 'strategy.';
  await prefix.onchange({ target: prefix });
  await env.settle();
  assert.ok(env.fetches.some((f) => f.url === '/api/blackboard?prefix=strategy.&history=0'));
});
