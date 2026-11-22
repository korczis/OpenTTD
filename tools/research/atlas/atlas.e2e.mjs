// RESEARCH-ONLY. The Research Atlas page's browser scenario, run by tools/surfaces/pages-check
// against a built site (surfaces.toml [[pages.items]] atlas, e2e). Every expected number is
// computed from the same data/atlas.json the page draws, never typed here, so the scenario proves
// the page agrees with its data rather than with a snapshot of it.
//
// It drives: filtering, chart <-> table selection both ways, the record's evidence links, a deep
// link reopened in a new context, maximize and Escape, the graph's node selection and edge-kind
// filter, the process stepper under reduced motion, the empty state and the error state, and that
// every view drew its chart.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const Core = require('./atlas.js');

export default async function scenario({ browser, url, site, root, open, check }) {
  const data = JSON.parse(fs.readFileSync(path.join(site, 'data', 'atlas.json'), 'utf8'));
  const rows = data.comparisons;
  const params = (u) => new URL(u).searchParams;

  // Every view drew: each chart box holds a canvas, the process lists every step of its diagram.
  {
    const { ctx, page, errors } = await open(url, { viewport: { width: 1440, height: 900 } });
    for (const id of ['explorer-chart', 'timeline-chart', 'battle-board', 'battle-history', 'graph-chart']) {
      check(`view ${id} drew`, (await page.locator(`#${id} canvas`).count()) > 0);
    }
    const approval = data.graphs.find((g) => g.id === 'approval');
    check('process lists every step', approval && (await page.locator('#proc-steps li').count()) === approval.nodes.length);
    const cards = await page.locator('[data-card]').count();
    check('every card is rendered', cards === data.cards.length, `${cards} of ${data.cards.length}`);

    // Filtering: the count and the table follow the filter, and agree with the data.
    await page.selectOption('#explorer [data-filter-key="class"]', 'valid');
    const want = Core.filterRows(rows, Core.readState(new URLSearchParams('class=valid'))).length;
    const shown = await page.locator('#explorer-table tr[data-row]').count();
    check('class filter: table rows match the data', shown === want, `${shown} shown, ${want} in the data`);
    check('class filter: kept in the URL', params(page.url()).get('class') === 'valid');
    const count = await page.textContent('#explorer-count');
    check('class filter: count says so', count.startsWith(`${want} of ${rows.length}`), count);

    // Table -> chart and detail: a row selects its record everywhere.
    const first = await page.getAttribute('#explorer-table tr[data-row]', 'data-row');
    await page.click(`#explorer-table tr[data-row="${first}"]`);
    check('row click selects the record (URL)', params(page.url()).get('sel') === first);
    check('row click opens the detail', (await page.locator(`[data-detail="${first}"]`).count()) === 1);
    const marked = await page.evaluate((id) => {
      const opt = window.surfaces.charts['explorer-chart'].getOption();
      return opt.series.some((s) => (s.data || []).some((d) => d && d.id === id && d.itemStyle && d.itemStyle.borderWidth === 3));
    }, first);
    check('row click marks the chart point', marked);

    // Chart -> table: a click on a mark selects its row.
    const other = await page.getAttribute('#explorer-table tr[data-row]:nth-child(2)', 'data-row');
    await page.evaluate((id) => window.surfaces.charts['explorer-chart'].trigger('click', { data: { id } }), other);
    check('chart click selects the row', (await page.getAttribute(`#explorer-table tr[data-row="${other}"]`, 'aria-selected')) === 'true');

    // Evidence: the write-up is a page of this site, the JSON a tracked file at the built commit.
    const hrefs = await page.$$eval(`[data-detail="${other}"] a[data-evidence]`, (as) => as.map((a) => a.getAttribute('href')));
    const rec = rows.find((r) => r.id === other);
    const writeup = hrefs[0].replace(/^\.?\//, '');
    check('evidence: write-up page exists', fs.existsSync(path.join(site, writeup)), writeup);
    check('evidence: record JSON is the tracked file', hrefs[1].endsWith('/' + rec.path) && fs.existsSync(path.join(root, rec.path)), hrefs[1]);

    // Maximize and Escape.
    await page.click('#explorer .maximize');
    check('maximize: card fills the viewport', await page.evaluate(() => document.getElementById('explorer').classList.contains('is-maximized')));
    check('maximize: kept in the URL', params(page.url()).get('max') === 'explorer');
    await page.keyboard.press('Escape');
    check('Escape restores', !(await page.evaluate(() => document.getElementById('explorer').classList.contains('is-maximized'))) && !params(page.url()).get('max'));

    // Deep link: the configured view, reopened elsewhere, is the same view.
    const seed = String(rows.find((r) => r.class === 'valid').seed);
    await page.selectOption('#explorer [data-filter-key="seed"]', seed);
    const link = page.url();
    const wantSeed = Core.filterRows(rows, Core.readState(new URLSearchParams(`class=valid&seed=${seed}`))).length;
    const again = await open(link, { viewport: { width: 1440, height: 900 } });
    const n2 = await again.page.locator('#explorer-table tr[data-row]').count();
    const sel2 = await again.page.inputValue('#explorer [data-filter-key="seed"]');
    check('deep link: same rows', n2 === wantSeed, `${n2} vs ${wantSeed}`);
    check('deep link: same controls', sel2 === seed && params(again.page.url()).get('sel') === other);
    check('deep link: same selection', (await again.page.locator(`[data-detail="${other}"]`).count()) === 1);
    check('deep link: no error', !again.errors.length, again.errors.join(' | '));
    await again.ctx.close();
    check('interactions: no page or console error', !errors.length, errors.slice(0, 3).join(' | '));
    await ctx.close();
  }

  // The graph: a node selects, highlights its neighbours and opens its page; kinds filter edges.
  {
    const g = data.graphs.find((x) => x.id === 'architecture');
    const node = g.nodes.find((n) => g.edges.some((e) => e.to === n.id) && g.edges.some((e) => e.from === n.id));
    const { ctx, page } = await open(url + `?view=graph&node=${node.id}`, { viewport: { width: 1440, height: 900 } });
    check('graph: deep-linked node is selected', (await page.locator(`[data-node-detail="${node.id}"]`).count()) === 1);
    const fed = Core.neighbours(g, node.id, null).in.length;
    const listed = await page.locator(`[data-node-detail="${node.id}"] [data-go]`).count();
    const nb = Core.neighbours(g, node.id, null);
    check('graph: detail lists every edge', listed === nb.in.length + nb.out.length, `${listed} vs ${nb.in.length + nb.out.length} (fed by ${fed})`);
    const href = await page.getAttribute(`[data-node-detail="${node.id}"] a[data-evidence]`, 'href');
    check('graph: node opens its page', href && fs.existsSync(path.join(site, href.replace(/^\.?\//, '').split('#')[0])), href);
    const kind = data.edge_kinds[0].id;
    await page.uncheck(`[data-kind="${kind}"]`);
    const links = await page.evaluate(() => window.surfaces.charts['graph-chart'].getOption().series[0].links.length);
    check('graph: unticking a kind hides its edges', links === g.edges.filter((e) => e.kind !== kind).length, `${links} edges`);
    check('graph: kinds kept in the URL', !(params(page.url()).get('kinds') || '').split(',').includes(kind) && params(page.url()).has('kinds'));
    await page.click('#graph-list li:first-child [data-node]');
    check('graph: the part list selects a node', params(page.url()).get('node') === g.nodes[0].id);
    await ctx.close();
  }

  // Reduced motion: Play jumps to the last step instead of animating.
  {
    const order = Core.walk(data.graphs.find((x) => x.id === 'approval'));
    const { ctx, page } = await open(url + '?view=process', { viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });
    await page.click('#proc-play');
    check('reduced motion: play shows the whole walk at once', (await page.getAttribute(`#proc-steps li[data-step="${order.length - 1}"]`, 'aria-current')) === 'step');
    await page.click('#proc-replay');
    await page.click('#proc-prev');
    check('process: step kept in the URL', Number(params(page.url()).get('step')) === order.length - 2);
    await ctx.close();
  }

  // Empty state: a filter nothing matches says so and offers a reset.
  {
    const { ctx, page } = await open(url + '?q=no-record-has-this-label-0000', { viewport: { width: 390, height: 844 } });
    check('empty state: shown', (await page.locator('[data-empty="explorer"]').count()) === 1);
    await page.click('[data-empty="explorer"] [data-reset]');
    check('empty state: reset restores every row', (await page.locator('#explorer-table tr[data-row]').count()) === rows.length);
    await ctx.close();
  }

  // Error state: the data does not load; the page says so and the static cards remain.
  {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.route('**/data/atlas.json', (r) => r.fulfill({ status: 500, body: 'no' }));
    await page.goto(url);
    await page.waitForSelector('#page-app[data-state="error"]', { timeout: 30000 });
    check('error state: an alert names the failure', (await page.locator('#page-app [role="alert"]').count()) === 1);
    check('error state: the cards stay', (await page.locator('[data-card]').count()) === data.cards.length);
    await ctx.close();
  }
}
