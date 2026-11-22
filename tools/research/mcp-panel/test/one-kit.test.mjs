// Rule project.web-ui-is-covered-end-to-end (v2), "one kit, no copies": every page script takes $,
// esc, api and notify from the shell (static/shell.js KIT, published as PanelKit; components.js for
// modules), and talks to its server only through api(), which shows every non-2xx answer. A copy of
// a helper, or a fetch( past the kit, fails here. The page scripts are derived from static/, never
// listed; the only fetch( allowed past the kit is one a line before names its reason with
// `// raw fetch: <reason>` (a stream read piece by piece, another server's API).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const dir = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'static');
const KIT = 'shell.js';
const scripts = fs.readdirSync(dir).filter((f) => f.endsWith('.js') && f !== KIT);
const htmls = fs.readdirSync(dir).filter((f) => f.endsWith('.html'));
/** The code of a file without its comments, line numbers kept. */
const code = (text) => text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' ')).split('\n').map((l) => l.replace(/(^|[^:'"`])\/\/.*$/, '$1'));

test('the kit is the one definition of $, esc and api', () => {
  const copies = [];
  for (const f of [...scripts, ...htmls]) {
    code(fs.readFileSync(path.join(dir, f), 'utf8')).forEach((l, i) => {
      if (/\b(const|let|var)\s+(esc|\$|api|notify)\s*=\s*(async\s*)?\(/.test(l) || /\bfunction\s+(esc|\$|api|notify)\s*\(/.test(l)) copies.push(`${f}:${i + 1}: ${l.trim()}`);
    });
  }
  assert.deepEqual(copies, [], 'take them from PanelKit (classic script) or components.js (module)');
});

test('a page talks to its server only through the kit', () => {
  const past = [];
  for (const f of [...scripts, ...htmls]) {
    const raw = fs.readFileSync(path.join(dir, f), 'utf8').split('\n');
    code(raw.join('\n')).forEach((l, i) => {
      if (/\bfetch\(/.test(l) && !/\/\/ raw fetch: \S/.test(raw[i - 1] || '')) past.push(`${f}:${i + 1}: ${l.trim().slice(0, 100)}`);
    });
  }
  assert.deepEqual(past, [], 'use api() (static/shell.js), or name why not on the line before: // raw fetch: <reason>');
});

test('the kit is published before any page script and shows what fails', async () => {
  const { KIT: kit } = (await import('node:module')).createRequire(import.meta.url)('../static/shell.js');
  assert.deepEqual(Object.keys(kit).sort(), ['$', 'api', 'esc', 'image', 'liveImage', 'notify']);
  for (const f of htmls) {
    const html = fs.readFileSync(path.join(dir, f), 'utf8');
    // Vendor libraries may come first; of the page's own scripts (/static/, or inline), the shell is first.
    const own = [...html.matchAll(/<script\b([^>]*)>/g)].map((m) => m[1]).filter((a) => /src="\/static\//.test(a) || !/\bsrc=/.test(a) && !/importmap/.test(a));
    if (own.length) assert.match(own[0], /src="\/static\/shell\.js"/, `${f}: shell.js is the page's first own script, so PanelKit exists before any other`);
  }
  // A non-2xx answer is shown to the person; a network failure too; neither throws.
  const shown = [];
  const fetches = { ok: async () => ({ ok: true, status: 200, text: async () => '{"a":1}' }),
    bad: async () => ({ ok: false, status: 403, statusText: 'Forbidden', text: async () => '{"error":"foreign origin"}' }),
    down: async () => { throw new Error('connection refused'); } };
  const prev = { fetch: globalThis.fetch, document: globalThis.document };
  const box = { children: [], append(c) { this.children.push(c); } };
  globalThis.document = { body: { insertAdjacentHTML() {} }, getElementById: (id) => (id === 'toasts' ? box : null),
    createElement: () => ({ dataset: {}, setAttribute() {}, set innerHTML(v) { shown.push(v); }, querySelector: () => ({}), remove() {} }) };
  try {
    globalThis.fetch = fetches.ok;
    const a = await kit.api('/api/x');
    assert.equal(a.a, 1); assert.equal(a.ok, true); assert.equal(a.status, 200);
    assert.deepEqual(Object.keys(a), ['a'], 'ok and status ride along unlisted');
    globalThis.fetch = fetches.bad;
    const b = await kit.api('/api/approve', { id: 1 });
    assert.equal(b.ok, false); assert.equal(b.status, 403);
    assert.match(shown.at(-1), /POST \/api\/approve: 403 foreign origin/);
    globalThis.fetch = fetches.down;
    const c = await kit.api('/api/status');
    assert.equal(c.ok, false);
    assert.match(shown.at(-1), /GET \/api\/status: connection refused/);
    const n = shown.length;
    const card = { dataset: {} };
    await kit.api('/api/status', undefined, { shownIn: card });
    assert.equal(shown.length, n, 'a page that writes the state into an element gets no toast');
    assert.match(card.dataset.failure, /^GET \/api\/status: connection refused/, 'the element is marked, so the crawl can check it');
    globalThis.fetch = fetches.bad;
    await kit.api('/api/approve', { id: 1 }, { shownIn: card });
    assert.equal(card.dataset.failure.split('\n').length, 2, 'two failures of one card, one line each');
    globalThis.fetch = fetches.ok;
    await kit.api('/api/status', undefined, { shownIn: card });
    assert.match(card.dataset.failure, /^POST \/api\/approve: 403/, 'a success clears its own line, not another failure');
    await kit.api('/api/approve', { id: 1 }, { shownIn: card });
    assert.equal(card.dataset.failure, undefined, 'and the last success clears the mark');
  } finally {
    globalThis.fetch = prev.fetch; globalThis.document = prev.document;
  }
});
