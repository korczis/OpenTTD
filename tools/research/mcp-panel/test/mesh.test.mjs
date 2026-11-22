// The Mesh page's pure parts (static/mesh.js): what it renders from the panel's GET /api/mesh, which
// answers Majordomus's /api/v1/mesh, /peers and /nodes unchanged.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { tone, ago, kpis, providersHtml, sessionRows, peersHtml, nodesHtml } = createRequire(import.meta.url)('../static/mesh.js');

const answer = {
  source: 'http://127.0.0.1:8741',
  reachable: true,
  errors: {},
  mesh: {
    active: true,
    identity: { node_id: '5d81b5c94b02767fc07b181613a7fc71', display_name: '5d81b5c9' },
    trust_policy: 'deny_unknown',
    providers: [{ id: 'tailscale', state: 'running', detail: '3 online', sent: 10, received: 20 }],
    tallies: { nodes: 15, trusted: 12 },
    started_at: '2026-11-20T07:01:20Z',
  },
  peers: { machines: [
    { node: 'a', name: '5d81b5c9', local: true, runtimes: [
      { runtime: 'a-81b2', this_runtime: true, liveness: 'own', last_beat_ms: 2100, sessions: [
        { key: 'k', state: 'active', info: { session: 'board-p1', client: 'battle-postmortem', intent: '<i>x</i>' }, claims: [{ scope: 'src/**' }], claims_held: 1 },
      ] },
      { runtime: 'a-99', liveness: 'stale', sessions: [] },
    ] },
  ] },
  nodes: { count: 2, nodes: [
    { node_id: 'z', display_name: 'zz', runtime: '2', trust: { state: 'observed' }, presence: 'present', last_seen: '2026-11-22T04:00:00Z' },
    { node_id: 'a', display_name: 'aa', runtime: '1', trust: { state: 'trusted', detail: 'allowlist' }, presence: 'present',
      endpoints: ['10.0.0.1:8791'], sources: [{ source: 'udp_multicast' }], version: '1.0.2', last_seen: '2026-11-22T04:00:00Z' },
  ] },
};

test('states are toned by what they mean, unknown ones gray', () => {
  assert.equal(tone('running'), 'green');
  assert.equal(tone('trusted'), 'green');
  assert.equal(tone('observed'), 'yellow');
  assert.equal(tone('refused'), 'red');
  assert.equal(tone('something-new'), 'gray');
});

test('ago reads an ISO time relative to now, and nothing from garbage', () => {
  const now = Date.parse('2026-11-22T04:01:00Z');
  assert.equal(ago('2026-11-22T04:00:30Z', now), '30 s');
  assert.equal(ago('2026-11-22T03:51:00Z', now), '10 min');
  assert.equal(ago('nope', now), '');
});

test('the KPIs take every tally the server reports, in its order', () => {
  const k = kpis(answer);
  assert.equal(k.active, true);
  assert.equal(k.node, '5d81b5c9');
  assert.equal(k.policy, 'deny_unknown');
  assert.deepEqual(k.tallies, [['nodes', 15], ['trusted', 12]]);
  assert.equal(kpis({}).active, false);
});

test('providers are rendered from the list, not a fixed set', () => {
  const html = providersHtml(answer.mesh.providers);
  assert.match(html, /tailscale/);
  assert.match(html, /running/);
  assert.equal(providersHtml(undefined), '');
});

test('every session of every runtime is a row; a runtime without one still shows', () => {
  const rows = sessionRows(answer.peers);
  assert.equal(rows.length, 2);
  assert.equal(rows[1].session, null);
  const html = peersHtml(answer.peers);
  assert.match(html, /board-p1/);
  assert.match(html, /this machine/);
  assert.match(html, /src\/\*\*/);
  assert.match(html, /no session/);
  assert.match(html, /&lt;i&gt;x&lt;\/i&gt;/, 'the intent is escaped');
});

test('nodes are sorted by name and show trust, presence and endpoints', () => {
  const html = nodesHtml(answer.nodes, Date.parse('2026-11-22T04:00:10Z'));
  assert.ok(html.indexOf('aa') < html.indexOf('zz'));
  assert.match(html, /allowlist/);
  assert.match(html, /10\.0\.0\.1:8791/);
  assert.match(html, /udp_multicast/);
  assert.match(html, /10 s/);
});
