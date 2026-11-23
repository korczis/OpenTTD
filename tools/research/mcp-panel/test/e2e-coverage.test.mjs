// node --test tools/research/mcp-panel/test/ : rule project.web-ui-is-covered-end-to-end. Every page the
// one shell declares (static/shell.js APPS) has a tools/research/*-check that drives it in Chrome and
// declares it (`// e2e-covers:`); tools/research/e2e-coverage is the enforcer, run here and by the gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shellPages, checks, coverage, declaredBy, runPlan, stripComments, parseError, verdictLine } from '../../e2e-coverage';

test('every page of the panel and the hub is covered by a browser check', () => {
  const v = coverage(shellPages(), checks());
  assert.ok(v.pages.length >= 10, `the pages come from the shell (${v.pages.length})`);
  assert.deepEqual(v.uncovered, [], `pages no browser check drives: ${v.uncovered.join(', ')}`);
  assert.deepEqual(v.refused, []);
});

test('a declaration counts only in a file that drives a browser, and only for a page the shell has', () => {
  const pages = [{ app: 'panel', href: '/' }, { app: 'panel', href: '/tools' }, { app: 'panel', href: '/scripts' }];
  const browser = "import x from 'playwright-core'; chromium.launch({}); await page.goto(u);";
  const v = coverage(pages, [
    declaredBy('a-check', `// e2e-covers: panel /scripts/rail_lib, panel /resource/game/state, panel /scriptsTYPO\n${browser}`),
    declaredBy('b-check', '// e2e-covers: panel /\ncurl -s http://x/'),
    declaredBy('c-check', `// e2e-covers: panel /nowhere\n${browser}`),
  ]);
  assert.deepEqual(Object.keys(v.covered).sort(), ['panel /scripts', 'panel /tools'], 'sub-pages count for their page');
  assert.deepEqual(v.uncovered, ['panel /']);
  assert.equal(v.refused.length, 3, 'a curl script, an unknown page and a typo past a path boundary are refused');
  const probe = declaredBy('d-check', "// e2e-covers: panel /\n// playwright-core chromium.launch( page.goto(\nconsole.log('nothing');");
  assert.equal(probe.browser, false, 'browser calls written only in comments do not count');
});

test('a check that does not parse drives no page, and every real check parses', () => {
  const ok = "#!/usr/bin/env node\nimport x from 'playwright-core';\nconst o = { a: 1, // why\n  b: 2 };\n";
  // The crawl's own defect: a trailing comment swallowed the rest of the object and its `)`.
  const bad = "#!/usr/bin/env node\nconst out = [];\nout.push({ a: 1, // why, b: 2 });\n";
  assert.equal(parseError(ok), '');
  assert.equal(parseError('#!/usr/bin/env bash\ncurl -s http://x/ ((\n'), '', 'only a node script is parsed');
  assert.match(parseError(bad), /^line \d+: SyntaxError: /, 'the line node names, and its error');
  const browser = "import x from 'playwright-core'; chromium.launch({}); await page.goto(u);";
  const broken = { ...declaredBy('x-check', `// e2e-covers: panel /\n${browser}`), parse: 'line 3: SyntaxError: x' };
  const v = coverage([{ app: 'panel', href: '/' }], [broken]);
  assert.deepEqual(v.uncovered, ['panel /'], 'its declaration counts for nothing');
  assert.deepEqual(v.refused, ['x-check does not parse (line 3: SyntaxError: x), so it drives no page']);
  assert.equal(verdictLine({ stdout: 'PASS a\nPASS b: 3/3\n', stderr: 'noise' }), 'PASS b: 3/3');
  assert.equal(verdictLine({ stdout: '', stderr: 'file:86\n  }\n\nSyntaxError: missing )\n' }), 'SyntaxError: missing )',
    'a check that died before it printed says why');
  assert.equal(verdictLine({ stdout: null, stderr: null, error: new Error('spawn EACCES') }), 'spawn EACCES');
  assert.equal(verdictLine({ stdout: '', stderr: '' }), '(no output)');
  const unparsed = checks().filter((c) => c.parse);
  assert.deepEqual(unparsed.map((c) => `${c.file}: ${c.parse}`), [], 'every tools/research/*-check parses');
});

test('--run gives each check the instance it covers, and says NOT RUN without a hub', () => {
  const b = "playwright-core chromium.launch( page.goto(";
  const plan = runPlan([declaredBy('p-check', `// e2e-covers: panel /\n${b}`), declaredBy('h-check', `// e2e-covers: hub /viz\n${b}`),
    declaredBy('both-check', `// e2e-covers: panel /ui, hub /docs\n${b}`)], 'http://p', '');
  assert.deepEqual(plan.map((r) => [r.file, r.argv, r.runnable]), [
    ['p-check', ['http://p'], true], ['h-check', [''], false], ['both-check', ['http://p'], true]]);
  assert.deepEqual(runPlan([declaredBy('both-check', `// e2e-covers: panel /ui, hub /docs\n${b}`)], 'http://p', 'http://h')[0].argv,
    ['http://p', '--hub', 'http://h']);
});

test('browser calls in block or trailing comments do not count, a URL in code does', () => {
  assert.equal(declaredBy('x', "// e2e-covers: panel /\n/* playwright-core chromium.launch( page.goto( */\nrun();").browser, false);
  assert.equal(declaredBy('y', "// e2e-covers: panel /\nrun(); // playwright-core chromium.launch( page.goto(").browser, false);
  assert.match(stripComments("await page.goto('http://127.0.0.1:1/x'); // note"), /http:\/\/127\.0\.0\.1:1\/x/);
});
