// node --test tools/research/mcp-panel/test/ : My bot (static/bots.js), laid out from the names the
// game declares. The fixture is fictional; nothing here talks to a game.
import './panel-ui.mjs'; // first: shell.js publishes PanelUI before any page script
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { layout, changes, kindOf, editorMarkup, controlsOf } from '../static/bots.js';

// The fixture src/tests/test_prismatic_mcp_ai_control.cpp reads too: openttd://ai's controls.
const SHARED = JSON.parse(fs.readFileSync(new URL('../../../../src/tests/prismatic_mcp_ai_controls.json', import.meta.url), 'utf8'));

const st = (name, value, description = 'a number', extra = {}) => ({ name, value, default: value, min: -1, max: 100, step: 1, flags: ['in_game'], ingame: true, editable: true, labels: {}, description, ...extra });
const line = (p, f, t, c) => [st(`${p}_from`, f, 'Producing industry (industry id; -1: none)'), st(`${p}_to`, t, 'Accepting industry (industry id; -1: none)'), st(`${p}_cargo`, c, 'Cargo (cargo id; -1: none)')];
const settings = [
  st('max_routes', 4), st('intercity', 1, 'flag', { flags: ['boolean'] }), st('air_a', 1, 'town (town id; -1: none)'), st('air_b', 2, 'town (town id; -1: none)'),
  st('rail_a', 3), ...line('fr', 1, 2, 1), ...line('fr2', 2, 1, 1), ...line('fr3', -1, -1, -1), st('own_lines', 8), st('max_routes', 99),
];

test('numbered names form one table of lines; the bare prefix is line 1 and an all -1 line is unused', () => {
  const { tables } = layout(settings);
  assert.equal(tables.length, 1);
  const [t] = tables;
  assert.equal(t.base, 'fr');
  assert.deepEqual(t.fields, ['from', 'to', 'cargo']);
  assert.deepEqual(t.rows.map((r) => r.n), [1, 2, 3]);
  assert.deepEqual(t.rows.map((r) => r.unset), [false, false, true]);
  assert.equal(t.rows[0].cells.from.name, 'fr_from');
});

test('the other settings group by a shared first word, the rest are general, a name twice is shown once', () => {
  const { groups } = layout(settings);
  assert.deepEqual(groups.map((g) => g.title), ['general', 'air']);
  assert.deepEqual(groups[0].settings.map((s) => s.name), ['max_routes', 'intercity', 'rail_a', 'own_lines'], 'a prefix with one line is no table');
  assert.equal(groups[0].settings[0].value, 4, 'the first declaration wins');
  assert.deepEqual(groups[1].settings.map((s) => s.name), ['air_a', 'air_b']);
});

test('Apply asks exactly the keys that differ, and nothing for an empty field', () => {
  const values = { max_routes: 6, intercity: 1, fr2_cargo: -1, own_lines: null };
  assert.deepEqual(changes(settings.slice(0, -1), values), [{ name: 'max_routes', from: 4, to: 6 }, { name: 'fr2_cargo', from: 1, to: -1 }]);
  assert.deepEqual(changes(settings, {}), []);
});

test('a world id picks from the live world only when the panel has that list', () => {
  const air = settings.find((s) => s.name === 'air_a');
  assert.equal(kindOf(air, {}), 'range');
  assert.equal(kindOf(air, { town: [{ id: 1, name: 'Fictional Town' }] }), 'picker');
  assert.equal(kindOf(settings[1]), 'toggle');
  assert.equal(kindOf(st('budget', 0, 'money', { max: 1000000 })), 'number');
});

test('the editor draws every setting once, folds unused lines and never lists an id as a name it is not', () => {
  const choices = { industry: [{ id: 1, name: 'Mine', produces: ['Ore'] }], cargo: [{ id: 1, name: 'Ore' }], town: [] };
  const html = editorMarkup(settings, 'c7', { choices });
  const names = [...html.matchAll(/data-setting="([^"]+)"/g)].map((m) => m[1]);
  assert.equal(names.length, new Set(names).size, 'no setting drawn twice');
  assert.equal(names.length, settings.length - 1);
  const folded = html.slice(html.indexOf('<details'));
  assert.match(folded, /data-line="3"/);
  assert.doesNotMatch(folded, /data-line="[12]"/);
  assert.match(html, /1 unused line \(3\)/);
  assert.match(html, /#2 \(not in this world\)/, 'an industry id the world does not list says so');
  assert.doesNotMatch(html, /data-apply=/, 'no Apply per field');
});

test('a setting without a control is derived by the rules openttd://ai uses: the shared fixture', () => {
  assert.ok(SHARED.cases.length >= 2);
  for (const c of SHARED.cases) assert.deepEqual(controlsOf(c.settings), c.controls, c.name);
});

test('the game\'s control wins over the panel\'s own derivation: kind, options, picker and place', () => {
  const names = ({ groups, tables }) => ({
    groups: groups.map((g) => [g.title, g.settings.map((s) => s.name)]),
    tables: tables.map((t) => [t.base, t.fields, t.rows.map((r) => [r.n, r.unset, Object.values(r.cells).map((s) => s.name)])]),
  });
  for (const c of SHARED.cases) {
    const served = c.settings.map((s, i) => ({ ...s, control: c.controls[i] }));
    assert.deepEqual(names(layout(served)), names(layout(c.settings)), `${c.name}: the same layout either way`);
  }
  const plain = st('rail_a', 3);
  assert.equal(kindOf(plain), 'range');
  assert.equal(kindOf({ ...plain, control: { kind: 'number', group: 'general' } }), 'number');
  assert.equal(kindOf({ ...plain, control: { kind: 'number', picker: 'town', group: 'general' } }, { town: [{ id: 1, name: 'Fictional Town' }] }), 'picker');
  const served = [{ ...plain, control: { kind: 'select', options: [{ value: 3, label: 'three' }, { value: 1, label: 'one' }], group: 'rails' } }];
  const { groups, tables } = layout(served);
  assert.deepEqual([groups.map((g) => g.title), tables.length], [['rails'], 0], 'the place comes from the control');
  const html = editorMarkup(served, 'c1');
  assert.match(html, /data-kind="select"/);
  assert.deepEqual([...html.matchAll(/<option value="(\d+)"/g)].map((m) => m[1]), ['3', '1'], 'options as the game orders them');
  assert.match(html, /<option value="3" selected>three<\/option>/);
});
