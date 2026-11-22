// RESEARCH-ONLY. The Research Atlas page's pure functions (tools/research/atlas/atlas.js), run with
// `node --test tools/research/atlas/test/*.test.mjs`. The page itself is driven in Chrome by atlas.e2e.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const Core = createRequire(import.meta.url)('../atlas.js');

const row = (id, o = {}) => Object.assign({
  id, kind: 'duel', class: 'valid', seed: 1, months: 36, opponent: 'PrismaticAI', held_start: true,
  confounders: [], caveats: [], problems: [], label: '', at: id, a: { value: 10 }, b: { value: 5 },
}, o);

test('readState falls back to defaults and parses lists', () => {
  const st = Core.readState(new URLSearchParams('view=nope&kinds=data,control&step=3&scale=linear'));
  assert.equal(st.view, '');
  assert.deepEqual(st.kinds, ['data', 'control']);
  assert.equal(st.step, 3);
  assert.equal(st.scale, 'linear');
  assert.equal(Core.readState(new URLSearchParams('')).scale, 'log');
  assert.equal(Core.readState(new URLSearchParams('')).kinds, null);
});

test('filterRows applies every filter and none when empty', () => {
  const rows = [row('a'), row('b', { class: 'limited', held_start: false, confounders: ['rival ran no train'] }),
    row('c', { seed: 2, opponent: null, label: 'Frozen set' })];
  const f = (q) => Core.filterRows(rows, Core.readState(new URLSearchParams(q))).map((r) => r.id);
  assert.deepEqual(f(''), ['a', 'b', 'c']);
  assert.deepEqual(f('class=limited'), ['b']);
  assert.deepEqual(f('start=held'), ['a', 'c']);
  assert.deepEqual(f('confounder=none'), ['a', 'c']);
  assert.deepEqual(f('confounder=rival+ran+no+train'), ['b']);
  assert.deepEqual(f('opponent=not+recorded'), ['c']);
  assert.deepEqual(f('q=frozen&seed=2'), ['c']);
  assert.deepEqual(f('q=zzz'), []);
});

test('compare gives a relative difference only when the rival is positive', () => {
  assert.equal(Core.compare(row('a')).rel, 1);
  assert.equal(Core.compare(row('a', { b: { value: 0 } })).rel, null);
  assert.equal(Core.compare(row('a', { b: { value: 1 } })).rel, null, 'value 1 is a company with nothing left');
  assert.equal(Core.compare(row('a', { b: { value: -3 } })).delta, 13);
});

test('groups and steps keep regressions', () => {
  const rows = [row('r1', { a: { value: 5 } }), row('r2', { a: { value: 9 } }), row('r3', { a: { value: 4 } }), row('x', { seed: 9 })];
  const gs = Core.groups(rows);
  assert.equal(gs.length, 1, 'a single record is no timeline');
  assert.deepEqual(Core.steps(gs[0]).map((s) => s.dir), ['first', 'up', 'down']);
});

test('neighbours respect kinds and bidirectional edges; walk follows edges', () => {
  const g = { nodes: [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'z' }], edges: [
    { from: 'a', to: 'b', kind: 'control' }, { from: 'b', to: 'c', kind: 'data' }, { from: 'c', to: 'a', kind: 'governance', both: true }] };
  const nb = Core.neighbours(g, 'a', null);
  assert.equal(nb.out.length, 2);
  assert.equal(nb.in.length, 1);
  assert.equal(Core.neighbours(g, 'b', ['control']).out.length, 0);
  assert.deepEqual(Core.walk(g), ['a', 'b', 'c', 'z']);
});
