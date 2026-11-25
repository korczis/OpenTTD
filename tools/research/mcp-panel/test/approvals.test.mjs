// node --test tools/research/mcp-panel/test/ : the approvals list's pure parts (static/approvals.js):
// the filter and its address, the rows it shows, the facets, the ids a bulk decision takes, a
// Shift+click range, the header checkbox, the rule editor's body and the markup of rows, rules and
// dialogs. The page itself is driven in Chrome by tools/research/mcp-approval-ui-check.
import './panel-ui.mjs'; // first: shell.js publishes PanelUI before any page script
import test from 'node:test';
import assert from 'node:assert/strict';
import { UI } from './panel-ui.mjs';

const A = await import('../static/approvals.js');

const row = (id, tool, args, extra = {}) => ({ id, tool, args, state: 'pending', expired: false, owner_only: false, ...extra });
const queue = [
  row(1, 'openttd.ai.set_setting', { company: 1, name: 'max_trains', value: 40 }),
  row(2, 'openttd.ai.set_setting', { company: 2, name: 'max_trains', value: 12 }),
  row(3, 'openttd.cmd.BuildRoad', { company: 1, tile: 777 }),
  row(4, 'openttd.ai.set_settings', { company: 1, settings: {} }, { owner_only: true }),
  row(5, 'openttd.game.set_pause', { pause: true }, { expired: true }),
  row(6, 'openttd.game.set_pause', { pause: false }, { state: 'consumed' }),
  row(7, 'openttd.cmd.CreateGoal', { company: 'deity', params: [255] }),
];
const none = { tool: '', company: '', q: '', expired: false };

test('the filter round-trips through the address', () => {
  assert.deepEqual(A.filterOf(''), none);
  const f = A.filterOf('?a_tool=openttd.cmd.BuildRoad&a_company=1&a_q=road&a_expired=show&x=1');
  assert.deepEqual(f, { tool: 'openttd.cmd.BuildRoad', company: '1', q: 'road', expired: true });
  assert.deepEqual(A.filterPatch(f), { tool: 'openttd.cmd.BuildRoad', company: '1', q: 'road', expired: 'show' });
  assert.equal(A.filterPatch(none).expired, '', 'hidden expired rows drop the key');
});

test('the rows shown: waiting ones the filter takes, expired ones only when asked for', () => {
  const ids = (f) => A.rowsOf(queue, { ...none, ...f }).map((a) => a.id);
  assert.deepEqual(ids({}), [1, 2, 3, 4, 7], 'decided and expired rows left out');
  assert.deepEqual(ids({ expired: true }), [1, 2, 3, 4, 5, 7]);
  assert.deepEqual(ids({ tool: 'openttd.ai.set_setting' }), [1, 2]);
  assert.deepEqual(ids({ company: '1' }), [1, 3, 4]);
  assert.deepEqual(ids({ company: 'deity' }), [7]);
  assert.deepEqual(ids({ q: 'MAX_TRAINS' }), [1, 2], 'search ignores case and reads the arguments');
  assert.deepEqual(ids({ q: '#3' }), [3]);
  assert.deepEqual(A.rowsOf(undefined, none), []);
  assert.equal(A.companyOf({ company: 0 }), '0');
  assert.equal(A.companyOf({ pause: true }), '');
  assert.equal(A.companyOf(null), '');
});

test('facets count the waiting approvals, most frequent first', () => {
  assert.deepEqual(A.facet(queue, (a) => a.tool).slice(0, 2), [['openttd.ai.set_setting', 2], ['openttd.ai.set_settings', 1]], 'ties by name');
  assert.deepEqual(A.facet(queue, (a) => A.companyOf(a.args)), [['1', 3], ['2', 1], ['deity', 1]]);
  assert.deepEqual(A.facet(undefined, (a) => a.tool), []);
  assert.match(A.optionsHtml([['1', 3]], 'Every company'), /<option value="">Every company<\/option><option value="1">1 \(3\)<\/option>/);
});

test('a bulk decision takes the picked rows, else every row shown, never one the panel may not decide', () => {
  const rows = A.rowsOf(queue, { ...none, expired: true });
  assert.deepEqual(A.bulkIds(rows, new Set()), [1, 2, 3, 7], 'owner-only and expired left out');
  assert.deepEqual(A.bulkIds(rows, new Set([2, 4, 5, 99])), [2], 'only picks the filter shows and the panel may decide');
  assert.deepEqual(A.bulkLabels(4, false), { approve: 'Approve all (4)', deny: 'Deny all (4)' });
  assert.deepEqual(A.bulkLabels(1, true), { approve: 'Approve selected (1)', deny: 'Deny selected (1)' });
  assert.equal(A.decidable(queue[3]), false);
  assert.equal(A.decidable(queue[0]), true);
});

test('a Shift+click range and the header checkbox', () => {
  assert.deepEqual(A.rangeIds([1, 2, 3, 7], 2, 7), [2, 3, 7]);
  assert.deepEqual(A.rangeIds([1, 2, 3, 7], 7, 1), [1, 2, 3, 7], 'either direction');
  assert.deepEqual(A.rangeIds([1, 2], 9, 2), [2], 'an anchor no longer shown: the one row');
  const rows = A.rowsOf(queue, none);
  assert.deepEqual(A.headerState(rows, new Set()), { checked: false, indeterminate: false, disabled: false });
  assert.deepEqual(A.headerState(rows, new Set([1])), { checked: false, indeterminate: true, disabled: false });
  assert.deepEqual(A.headerState(rows, new Set([1, 2, 3, 7])), { checked: true, indeterminate: false, disabled: false });
  assert.deepEqual(A.headerState([queue[3]], new Set()), { checked: false, indeterminate: false, disabled: true });
});

test('a row: Yes and No in the decide column, an owner-only one sent to the console, an expired one marked', () => {
  const r = A.rowHtml(queue[0], { picked: true, current: true });
  assert.match(r, /data-approve="1"[^>]*>Yes</);
  assert.match(r, /data-deny="1"[^>]*>No</);
  assert.match(r, /aria-label="Approve #1"/);
  assert.match(r, /data-select="1"[^>]*checked/);
  assert.match(r, /tabindex="0"/);
  assert.ok(r.includes(UI.check), "the pick is the shell's check");
  const owner = A.rowHtml(queue[3]);
  assert.doesNotMatch(owner, /data-approve|data-deny/);
  assert.match(owner, /decide at the game&#39;s console|decide at the game's console/);
  assert.match(owner, /prismatic_mcp approve 4/);
  assert.match(owner, /data-select="4"[^>]*disabled/);
  assert.match(A.rowHtml(queue[4]), /expired/);
  const matched = A.rowHtml({ ...queue[0], matched_rule: { id: 2, effect: 'deny', decides: true } });
  assert.match(matched, /rule #2 deny/);
  assert.match(A.rowHtml({ ...queue[3], matched_rule: { id: 3, effect: 'allow', decides: false } }), /rule #3 allow, held: you decide it/);
  assert.match(A.rowHtml(queue[0], { rules: false }), /data-always="1"[^>]*disabled/);
});

test('the rule editor: prefilled from the row, its body checked before it is sent', () => {
  assert.deepEqual(A.matcherOf({ company: 1 }), { company: 1 });
  assert.equal(A.matcherOf({}), null);
  assert.equal(A.matcherOf([1]), null);
  const e = A.editorHtml(queue[0]);
  assert.match(e, /value="openttd\.ai\.set_setting"/);
  assert.match(e, /&quot;max_trains&quot;/);
  assert.match(e, /value="allow" checked/);
  const owner = A.editorHtml(queue[3], { ownerOnly: true });
  assert.match(owner, /value="allow" disabled/);
  assert.match(owner, /value="deny" checked/);
  assert.match(A.editorHtml(row(9, 'openttd.game.set_pause', {})), /<textarea[^>]*>\{\}<\/textarea>/);
  const f = { effect: 'allow', tool: ' openttd.ai.* ', matcher: '{"name": "max_*"}', useMatcher: true, persist: 1 };
  assert.deepEqual(A.ruleBody(f).body, { effect: 'allow', tool: 'openttd.ai.*', persist: true, match: { name: 'max_*' } });
  assert.deepEqual(A.ruleBody({ ...f, useMatcher: false }).body, { effect: 'allow', tool: 'openttd.ai.*', persist: true });
  assert.deepEqual(A.ruleBody({ ...f, matcher: '  ' }).body, { effect: 'allow', tool: 'openttd.ai.*', persist: true });
  assert.match(A.ruleBody({ ...f, matcher: '{nope' }).error, /not JSON/);
  assert.match(A.ruleBody({ ...f, matcher: '[1]' }).error, /JSON object/);
  assert.equal(A.ruleText({ effect: 'deny', tool: 'openttd.cmd.*', match: { company: 2 } }), 'always deny openttd.cmd.* where {"company":2}');
  assert.equal(A.ruleText({ effect: 'allow', tool: 'x', match: null }), 'always allow x');
});

test('the rules list, the bulk confirmation and a failed id', () => {
  assert.match(A.rulesHtml([]), /no standing rule yet/);
  const r = A.rulesHtml([{ id: 3, effect: 'deny', tool: 'openttd.cmd.*', match: { company: 2 }, created_via: 'panel', hits: 4, persisted: true },
    { id: 4, effect: 'allow', tool: 'x', match: null, hits: 0, persisted: false }]);
  assert.match(r, /data-rule-remove="3"/);
  assert.match(r, /always deny/);
  assert.match(r, /kept/);
  assert.match(r, /any arguments/);
  assert.match(r, /this run/);
  assert.match(A.confirmHtml(5, false), /Approve 5 approvals\?/);
  assert.match(A.confirmHtml(1, true), /Approve 1 selected approval\?/);
  assert.equal(A.failureText({ id: 3, code: 409, answer: { error: 'approval #3 is consumed, not pending' } }), '#3: 409 approval #3 is consumed, not pending');
  assert.equal(A.failureText({ id: 4, code: 504, answer: null }), '#4: 504');
});

test("the core's matcher and glob, mirrored (panel.rs core_match has the same cases)", () => {
  const args = { company: 1, name: 'max_trains', value: 40, params: [255, 0, 'goal'], nested: { a: { name: 'x' } } };
  for (const [m, want] of [
    [{}, true], [{ company: 1 }, true], [{ company: 2 }, false], [{ company: '1' }, true], [{ name: 'max_*' }, true],
    [{ name: 'max_?rains' }, true], [{ name: 'min_*' }, false], [{ value: { $gte: 0, $lt: 100 } }, true], [{ value: { $gt: 40 } }, false],
    [{ value: { $lte: 40 } }, true], [{ value: { $lt: 'x' } }, false], [{ company: { $in: [2, 1] } }, true], [{ company: { $in: 1 } }, false],
    [{ company: { $ne: 1 } }, false], [{ '$.params[0]': 255 }, true], [{ '$.params[*]': '*' }, true], [{ '$.params[*]': 255 }, false],
    [{ '$..name': '*' }, true], [{ '$.missing': '*' }, false], [{ "$['name']": 'max_trains' }, true], [{ "$['nope']": 1 }, false],
    [{ '$.params[x]': 1 }, false], [{ '$.params[9]': 1 }, false], [{ params: [255, 0, 'go*'] }, true], [{ params: [255] }, false],
    [{ missing: '*' }, false], [{ $: '*' }, true], [{ $ne: null }, true], [{ '$.nested.*.name': 'x' }, true], [{ 'x$': 1 }, false],
    [{ '$x': 1 }, false], [{ '$.': 1 }, false], [{ '$..': 1 }, false], [{ '$[0': 1 }, false], [{ '$.params.*': '*' }, true],
  ]) assert.equal(A.coreMatch(m, args), want, JSON.stringify(m));
  assert.equal(A.coreMatch(5, '5'), true, 'a number against the string of its JSON text');
  assert.equal(A.coreMatch(null, null), true);
  assert.equal(A.coreMatch({ a: 1 }, null), false);
  assert.ok(A.coreGlob('openttd.cmd.*', 'openttd.cmd.BuildRoad') && A.coreGlob('a\\*b', 'a*b') && !A.coreGlob('a\\*b', 'axb') && A.coreGlob('?', '火'));
  assert.ok(!A.coreGlob('a', 'ab') && A.coreGlob('*', '') && A.coreGlob('a**', 'a'));
  assert.ok(A.hasWildcard('openttd.*') && A.hasWildcard('a?') && !A.hasWildcard('a\\*') && !A.hasWildcard('openttd.ai.set_setting'));
  assert.equal(A.pathSelect({ a: 1 }, 'a'), null, 'a path starts with $');
  assert.deepEqual(A.pathSelect({ a: [1, 2] }, '$.a[*]'), [1, 2]);
  assert.deepEqual(A.pathSelect({ a: { b: 1, c: 2 } }, '$.a.*'), [1, 2]);
  assert.deepEqual(A.pathSelect([{ name: 1 }, { k: { name: 2 } }], '$..name'), [1, 2]);
  assert.equal(A.actionOf('openttd.cmd.BuildRoad'), 'command.BuildRoad');
  assert.equal(A.actionOf('openttd.ai.set_setting'), 'ai.set_setting');
});

test('a broad matcher, and the dry run of a rule over the rows the page holds', () => {
  // The same cases as panel.rs: broad constrains no value, whatever keys it names.
  for (const m of [null, undefined, {}, { $: '*' }, { $ne: null }, { company: '*' }, { '$.*': '*' }, { '$..name': '**' }, { a: { b: '*' } },
    { x: { $ne: null } }, { "$['a b']": '*', '$.l[*]': '***' }]) assert.ok(A.broad(m), JSON.stringify(m));
  for (const m of [{ company: 0 }, { name: 'a*' }, { n: { $lt: 5 } }, { c: { $in: [1] } }, { c: '' }, { c: '?' }, { c: [1] }, { c: true },
    { a: { b: 'x' } }, { x: { $ne: null, $gt: 1 } }, { company: '*', name: 'max_*' }, 5, [1], { c: null }]) assert.ok(!A.broad(m), JSON.stringify(m));
  const rows = [row(1, 'openttd.ai.set_setting', { company: 1, name: 'max_trains' }), row(2, 'openttd.ai.set_setting', { company: 2, name: 'max_trains' }),
    row(3, 'openttd.ai.set_settings', { company: 1 }, { owner_only: true }), row(4, 'openttd.ai.set_setting', { company: 1 }, { expired: true }),
    row(5, 'openttd.cmd.BuildRoad', { company: 1 }), row(6, 'openttd.ai.set_setting', { company: 1 }, { state: 'consumed' })];
  const f = { effect: 'allow', tool: 'openttd.ai.*', matcher: '{"company": 1}', useMatcher: true };
  assert.deepEqual(A.dryRun(rows, f), { ids: [1] }, 'never an owner-only, expired or decided row for allow');
  assert.deepEqual(A.dryRun(rows, { ...f, effect: 'deny' }), { ids: [1, 3] }, 'a deny rule takes an owner-only one');
  assert.deepEqual(A.dryRun(rows, { ...f, tool: 'command.Build*' }), { ids: [5] }, 'the action type matches too');
  assert.match(A.dryRun(rows, { ...f, useMatcher: false }).error, /^too broad/);
  assert.match(A.dryRun(rows, { ...f, matcher: '{"$ne": null}' }).error, /^too broad/);
  assert.deepEqual(A.dryRun(rows, { ...f, tool: 'openttd.ai.set_setting', useMatcher: false }), { ids: [1, 2] }, 'one tool with any arguments is fine');
  assert.match(A.dryRun(rows, { ...f, matcher: '{nope' }).error, /not JSON/);
  assert.match(A.dryRun(rows, { ...f, tool: ' ' }).error, /name a tool/);
  assert.equal(A.dryRunText({ ids: [] }), 'Would decide 0 waiting approvals now');
  assert.equal(A.dryRunText({ ids: [7] }), 'Would decide 1 waiting approval now: #7');
  assert.equal(A.dryRunText({ ids: [7, 8] }), 'Would decide 2 waiting approvals now: #7, #8');
  assert.equal(A.dryRunText({ error: 'x' }), 'x');
  assert.match(A.askHtml('Remove rule #2 (<b>)?', 'Remove'), /Remove rule #2 \(&lt;b&gt;\)\?/);
  assert.ok(A.DECISIVE.has('Enter') && A.DECISIVE.has(' ') && !A.DECISIVE.has('j') && A.GUARD_MS === 400);
});

test('the rows a refresh moved under the pointer: re-ordered, a row inserted above, a new row', () => {
  assert.deepEqual(A.movedRows(null, [1, 2, 3]), [], 'none on the first drawing');
  assert.deepEqual(A.movedRows([1, 2, 3], [1, 2, 3]), []);
  assert.deepEqual(A.movedRows([1, 2, 3], [2, 3]), [], 'a row leaving above moves nothing');
  assert.deepEqual(A.movedRows([1, 2, 3], [9, 1, 2, 3]), [9, 1, 2, 3], 'a row inserted above moves every row below it');
  assert.deepEqual(A.movedRows([1, 2, 3], [1, 2, 3, 4]), [4], 'a row added at the end moves only itself');
  assert.deepEqual(A.movedRows([1, 2, 3], [2, 1, 3]), [1], 'a re-order moves the row that got a new row above it');
  assert.equal(A.ROW_GUARD_MS, 600);
});
