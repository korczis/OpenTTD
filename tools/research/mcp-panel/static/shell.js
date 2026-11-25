'use strict';
/* The one shell of every panel page (rule project.panel-items-are-routable-pages): the Flowbite Pro admin
   navbar, sidebar and page header, declared once here and never copied into a page. A page is only its <main>:
   - <body data-page="KEY"> names it (a key of PAGES); its sidebar entry is marked current;
   - the page header (breadcrumb from the registry, H1, description, #badges) opens <main>: the H1 is
     <body data-title> or the page's label, the description <body data-description> (both may be filled
     server-side, __TITLE__ / __DESCRIPTION__); a crumb a page script appends to #crumbs is redrawn as
     the shell's own (the bare "/" separators dropped);
   - UI holds the Pro class sets (card, card title, buttons, select, table, badge) every page uses;
   - elements in <main> carrying data-nav="Label" become its "On this page" entries (derived, not listed);
   - elements carrying data-nav-extra move into the navbar's right side (page stats, endpoints);
   - the connection badge (#conn) and the console buttons (#open-console, #side-console) exist on every
     page; a page with its own console (the home) binds them, every other page sends them to /#console.
   Load it as the first element of <body>: the navbar and sidebar are in place before Flowbite inits its
   drawer on DOMContentLoaded. The pure parts are exported for Node (test/shell.test.mjs). */

// Its own scope: a classic script's top-level const is global, and app.js declares esc too.
(function shell() {
  // The panel's pages (one game); the hub's (every instance) are APPS.hub. A page picks its app with
  // <body data-app>, the panel by default.
  // icon: an outline icon's path (24x24, stroke), shown alone when the sidebar is a rail.
  const PAGES = [
    { key: 'home', href: '/', label: 'Overview', title: 'Panel', icon: 'M3 12l9-9 9 9M5 10v10h5v-6h4v6h5V10' },
    { key: 'world', href: '/world', label: 'World: map, plan, yards', title: 'World', icon: 'M9 20l-6-3V4l6 3m0 13 6-3m-6 3V7m6 10 6 3V7l-6-3m0 13V4M9 7l6-3' },
    { key: 'play', href: '/play', label: 'Play: the game in your browser', title: 'Play', icon: 'M7 5l12 7-12 7z' },
    { key: 'battle', href: '/battle', label: 'Battle: every LAN arena', title: 'Battle', icon: 'M4 20h16M6 16V9m4 7V5m4 11v-5m4 5V7' },
    { key: 'mesh', href: '/mesh', label: 'Mesh: Majordomus peers and nodes', title: 'Mesh', icon: 'M5 12a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm14 0a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM12 21a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM6.7 11l4.2 6.2M17.3 11l-4.2 6.2M7 10h10' },
    { key: 'ui', href: '/ui', label: 'Interface: every window', title: 'Interface', icon: 'M4 5h16v14H4zM4 9h16M8 5v4' },
    { key: 'tools', href: '/tools', label: 'MCP catalogue', title: 'Catalogue', icon: 'M4 6h16M4 12h16M4 18h10' },
    { key: 'results', href: '/results', label: 'Duels', title: 'Duels', icon: 'M7 4h10v4a5 5 0 0 1-10 0zM5 5H3v2a4 4 0 0 0 4 4m12-6h2v2a4 4 0 0 1-4 4m-5 2v4m-4 3h8' },
    { key: 'data', href: '/data', label: 'Data: query the event store', title: 'Data', icon: 'M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zm0 0v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3' },
    { key: 'bench', href: '/bench', label: 'Benchmarks: Rust performance', title: 'Benchmarks', icon: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18zm0-13v4l3 2M9 2h6' },
    { key: 'scripts', href: '/scripts', label: 'Scripts: Rhai editor and services', title: 'Scripts', icon: 'M8 9l-3 3 3 3m8-6 3 3-3 3M14 5l-4 14' },
    { key: 'towns', href: '/towns', label: 'Towns: authority model and advice', title: 'Towns', icon: 'M3 21h18M5 21V10l4-3 4 3v11m4 0V6l3-2 3 2v15' },
    { key: 'strategy', href: '/strategy', label: 'Strategy: strategist and blackboard', title: 'Strategy', icon: 'M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2M9 5a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2M9 5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2m-6 9 2 2 4-4' },
    { key: 'realworld', href: '/realworld', label: 'Real world: places as maps', title: 'Real world', icon: 'M12 21s-7-6.2-7-12a7 7 0 0 1 14 0c0 5.8-7 12-7 12zm0-9a3 3 0 1 0 0-6 3 3 0 0 0 0 6z' },
    { key: 'docs', href: '/docs', label: 'API (OpenAPI, Swagger UI)', title: 'API', icon: 'M10 20l4-16M6 16l-4-4 4-4m12 0 4 4-4 4' },
  ];

  const APPS = {
    panel: { brand: 'OpenTTD MCP', status: '/api/status', console: '/#console', doc: 'research/mcp-control.md', pages: PAGES },
    // The hub sets #conn from its own WebSocket and has no developer console.
    hub: { brand: 'MCP Hub', status: null, console: null, doc: 'research/mcp-hub.md', pages: [
      { key: 'hub', href: '/', label: 'Hub: every instance', title: 'Hub' },
      { key: 'viz', href: '/viz', label: 'World view', title: 'World view' },
      { key: 'docs', href: '/docs', label: 'API (OpenAPI)', title: 'API' },
    ] },
  };

  /** The page a path belongs to: a tool, client or resource page is part of the catalogue, a run of the duels. */
  function pageOf(path, app = 'panel') {
    if (app === 'hub') return path === '/' || path === '/hub' ? 'hub' : path.startsWith('/viz') ? 'viz' : path.startsWith('/docs') ? 'docs' : '';
    if (path === '/' || path === '/index.html') return 'home';
    if (/^\/(tools|tool|client|resource)(\/|$)/.test(path)) return 'tools';
    if (/^\/(results|result)(\/|$)/.test(path)) return 'results';
    const hit = PAGES.find((p) => p.href !== '/' && (path === p.href || path.startsWith(p.href + '/')));
    return hit ? hit.key : '';
  }

  /* ---- the one page kit: every page script takes these from here (PanelKit, components.js), never
     a copy of its own (rule project.web-ui-is-covered-end-to-end, ADR 0038) ------------------------- */
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const $ = (id) => document.getElementById(id);
  /** A Flowbite toast in the corner (#toasts, made on the first one): what a control did, or why it failed. Errors
   *  stay until dismissed; the rest go after 6 s. The toast carries data-toast="<tone>", which the
   *  generic browser crawl reads (tools/research/mcp-ui-crawl-check). */
  function notify(text, tone = 'red') {
    if (typeof document === 'undefined' || !document.body) return;
    let box = document.getElementById('toasts');
    if (!box) {
      document.body.insertAdjacentHTML('beforeend', '<div id="toasts" aria-live="polite" class="fixed bottom-4 end-4 z-50 flex flex-col gap-2 w-[calc(100%-2rem)] max-w-sm"></div>');
      box = document.getElementById('toasts');
    }
    const t = document.createElement('div');
    t.setAttribute('role', tone === 'red' ? 'alert' : 'status');
    t.dataset.toast = tone;
    t.className = 'flex items-start w-full max-w-sm p-4 gap-3 text-gray-700 bg-white rounded-lg shadow-sm border border-gray-200 dark:text-gray-300 dark:bg-gray-800 dark:border-gray-700';
    t.innerHTML = `<span class="${UI.badge} ${TONE[tone] || TONE.gray}">${tone === 'red' ? 'error' : 'ok'}</span><div class="text-sm min-w-0 break-words">${esc(text)}</div><button type="button" aria-label="Dismiss" class="ms-auto -mx-1.5 -my-1.5 p-1.5 rounded-lg text-gray-400 hover:text-gray-900 hover:bg-gray-100 dark:hover:text-white dark:hover:bg-gray-700">&times;</button>`;
    t.querySelector('button').onclick = () => t.remove();
    box.append(t);
    if (tone !== 'red') setTimeout(() => t.remove(), 6000);
  }
  /** Where a failure is shown: a toast, or, for a page that writes the state into an element itself
   *  (a poll, a card that says "no GUI client"), that element, marked data-failure="<METHOD> <path>:
   *  <why>" so the crawl can check the failure really is on the page. Cleared on the next success. */
  // One element may show several failures (the windows and the screenshot of one card): one line
  // each, keyed by "<METHOD> <path>", replaced on the next failure and dropped on the next success.
  const elementOf = (shownIn) => (typeof shownIn === 'string' ? (typeof document !== 'undefined' && document.getElementById(shownIn)) : shownIn);
  const withoutKey = (marks, key) => (marks || '').split('\n').filter((m) => m && !m.startsWith(`${key}:`));
  function failed(shownIn, what) {
    const el = elementOf(shownIn);
    if (!el) return notify(what);
    el.dataset.failure = [...withoutKey(el.dataset.failure, what.split(':')[0]), what].join('\n');
  }
  function succeeded(shownIn, key) {
    const el = elementOf(shownIn);
    if (!el || !el.dataset || !el.dataset.failure) return;
    const rest = withoutKey(el.dataset.failure, key);
    if (rest.length) el.dataset.failure = rest.join('\n'); else delete el.dataset.failure;
  }
  /** The owner's sign-in, a Flowbite modal: what an owner's route (Approve, auto-approve on) asks for
   *  instead of a raw 401. `file` is where mcp-panel keeps the secret (it names it when it starts). */
  function signInHtml(file) {
    return `<div id="owner-signin" role="dialog" aria-modal="true" aria-labelledby="owner-signin-title" tabindex="-1" class="fixed inset-0 z-50 flex items-center justify-center w-full h-full p-4 bg-gray-900/50 dark:bg-gray-900/80">
  <div class="relative w-full max-w-md bg-white rounded-lg shadow-sm dark:bg-gray-700">
    <div class="flex items-center justify-between p-4 border-b border-gray-200 rounded-t dark:border-gray-600">
      <h3 id="owner-signin-title" class="text-lg font-semibold text-gray-900 dark:text-white">Owner sign-in</h3>
      <button type="button" data-signin-cancel aria-label="Close" class="inline-flex items-center justify-center w-8 h-8 text-sm text-gray-400 bg-transparent rounded-lg hover:bg-gray-200 hover:text-gray-900 dark:hover:bg-gray-600 dark:hover:text-white">&times;</button>
    </div>
    <form id="owner-signin-form" class="p-4 space-y-4">
      <p class="text-sm text-gray-500 dark:text-gray-400">Approving and auto-approve are the owner's. Paste the owner secret from <code class="font-mono text-gray-900 break-all dark:text-white">${esc(file || '~/.config/prismatic/panel-owner-secret')}</code>; this browser then stays signed in, across panel restarts too.</p>
      <div><label for="owner-secret" class="block mb-2 text-sm font-medium text-gray-900 dark:text-white">Owner secret</label>
        <input type="password" id="owner-secret" name="secret" autocomplete="current-password" required class="block w-full ${UI.input}"></div>
      <p id="owner-signin-error" role="alert" class="hidden text-sm font-medium text-red-600 dark:text-red-500"></p>
      <div class="flex justify-end gap-2"><button type="button" data-signin-cancel class="${UI.secondary}">Cancel</button><button type="submit" class="${UI.primary}">Sign in</button></div>
    </form>
  </div>
</div>`;
  }
  let signing = null;
  /** Ask the owner to sign in: true once POST /api/owner/login took the secret (the panel then keeps
   *  this browser's HttpOnly cookie), false when the owner closed the box. One box at a time. */
  function signIn(file) {
    if (signing) return signing;
    signing = new Promise((resolve) => {
      document.body.insertAdjacentHTML('beforeend', signInHtml(file));
      const box = document.getElementById('owner-signin');
      const input = document.getElementById('owner-secret');
      const error = document.getElementById('owner-signin-error');
      const done = (ok) => { box.remove(); signing = null; resolve(ok); };
      box.querySelectorAll('[data-signin-cancel]').forEach((b) => { b.onclick = () => done(false); });
      box.onkeydown = (e) => { if (e.key === 'Escape') done(false); };
      document.getElementById('owner-signin-form').onsubmit = async (e) => {
        e.preventDefault();
        const r = await fetch('/api/owner/login', { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ secret: input.value }) }).catch(() => null);
        if (r && r.ok) return done(true);
        error.textContent = r ? ((await r.json().catch(() => ({}))).error || `${r.status} ${r.statusText}`) : 'the panel is unreachable';
        error.classList.remove('hidden');
        input.select();
      };
      input.focus();
    });
    return signing;
  }
  /** The one API client: GET without a body, POST JSON with one. Answers the parsed body (or {status,
   *  text} for a non-JSON one) with `ok`; a non-2xx answer or a network failure is shown to the person
   *  (a toast, or the element named by `shownIn`, see failed()), so no control fails silently: a 403
   *  only the console saw is how every button of a page opened as localhost went dead (2026-11-22).
   *  An owner's route refused for want of the owner secret asks the owner to sign in, then is sent once more. */
  async function api(path, body, { shownIn, retried } = {}) {
    const opts = body === undefined ? {} : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) };
    const where = `${opts.method || 'GET'} ${path.split('?')[0]}`;
    let r;
    try { r = await fetch(path, { credentials: 'same-origin', ...opts }); } catch (e) {
      failed(shownIn, `${where}: ${e.message || 'the panel is unreachable'}`);
      return { ok: false, status: 0, error: String(e.message || e) };
    }
    const text = await r.text();
    let out;
    try { out = JSON.parse(text); } catch { out = { status: r.status, text }; }
    if (r.status === 401 && out && out.refused === 'owner_secret' && !retried && typeof document !== 'undefined' && await signIn(out.secret_file)) {
      return api(path, body, { shownIn, retried: true });
    }
    if (!r.ok) failed(shownIn, `${where}: ${r.status} ${(out && (out.error || out.text)) || r.statusText}`);
    else succeeded(shownIn, where);
    // `ok` and the HTTP `status` ride along unlisted (a body's own `status`, a debug session's, wins).
    if (out && typeof out === 'object' && !Array.isArray(out)) {
      if (!('ok' in out)) Object.defineProperty(out, 'ok', { value: r.ok, enumerable: false });
      if (!('status' in out)) Object.defineProperty(out, 'status', { value: r.status, enumerable: false });
    }
    return out;
  }
  /** One image from the server (a screenshot): `done(image)` when it loaded, `done(null)` when not,
   *  the failure shown as api() shows one (`shownIn`, else a toast). */
  function image(src, shownIn, done = () => {}) {
    const next = new Image();
    next.onload = () => { succeeded(shownIn, `GET ${src.split('?')[0]}`); done(next); };
    next.onerror = () => { failed(shownIn, `GET ${src.split('?')[0]}: did not load`); done(null); };
    next.src = src;
  }
  /** The live view of the owner's game window: `img` follows the GUI client's screenshot every
   *  `every` ms while the page is visible, `msg` says when it was updated or why it is not. */
  function liveImage(img, msg, every = 1200) {
    if (!img || !msg) return;
    let busy = false;
    setInterval(() => {
      if (busy || document.hidden) return;
      busy = true;
      image(`/api/view/screenshot?t=${Date.now()}`, msg, (next) => {
        busy = false;
        if (next) { img.src = next.src; msg.textContent = `updated ${new Date().toLocaleTimeString()}`; } else msg.textContent = 'no GUI client attached yet';
      });
    }, every);
  }
  const KIT = Object.freeze({ esc, $, api, notify, image, liveImage });
  /** The frame every page's <main> gets: under the navbar, beside the sidebar, one gutter. It fills
   *  the width beside the sidebar at every size: a cap (max-w-screen-2xl) left a third of a 2560 px
   *  screen empty, and the cards and grids already wrap by their own breakpoints. */
  const FRAME = ['pt-20', 'px-4', 'sm:px-6', 'lg:ms-[var(--sidebar-w)]'];
  const LINK = 'flex items-center gap-3 p-2 text-sm text-gray-900 rounded-lg hover:bg-gray-100 dark:text-gray-200 dark:hover:bg-gray-700';
  /** The sidebar's two states on <html data-sidebar>: full (16rem, labels) or rail (3.5rem, icons
   *  only, a title as tooltip). input.css maps them to --sidebar-w, which the sidebar's width, the
   *  page frame and the console drawer all read; localStorage keeps the owner's choice. */
  const SIDEBAR_KEY = 'panel.sidebar';
  const SIDEBAR_STATES = ['full', 'rail'];
  const icon = (d, cls = 'w-5 h-5') => `<svg class="${cls} shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" fill="none" viewBox="0 0 24 24"><path stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.75" d="${d}"/></svg>`;
  const CONSOLE_ICON = 'M8 9l3 3-3 3m5 0h3M4 19h16a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1H4a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1z';
  const SUB = 'flex items-center p-2 ps-4 text-sm text-gray-700 rounded-lg hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700';

  /** The Pro class sets of the admin dashboard (reference.md sections 1, 6, 9, 13), one per role. A page's
      markup carries every class of its role's set (test/shell.test.mjs enforces it), plus layout only. */
  const FIELD = 'bg-gray-50 border border-gray-300 text-gray-900 text-sm rounded-lg focus:ring-primary-500 focus:border-primary-500 dark:bg-gray-700 dark:border-gray-600 dark:placeholder-gray-400 dark:text-white dark:focus:ring-primary-500 dark:focus:border-primary-500';
  const UI = Object.freeze({
    card: 'p-4 bg-white border border-gray-200 rounded-lg shadow-sm sm:p-6 dark:border-gray-700 dark:bg-gray-800',
    cardTitle: 'text-xl font-bold leading-none text-gray-900 dark:text-white',
    cardSub: 'text-sm font-normal text-gray-500 dark:text-gray-400',
    h1: 'text-xl font-semibold text-gray-900 sm:text-2xl dark:text-white',
    primary: 'text-white bg-primary-700 hover:bg-primary-800 focus:ring-4 focus:ring-primary-300 font-medium rounded-lg text-sm px-4 py-2 whitespace-nowrap focus:outline-none disabled:opacity-50 dark:bg-primary-600 dark:hover:bg-primary-700 dark:focus:ring-primary-800',
    secondary: 'text-gray-900 bg-white border border-gray-300 hover:bg-gray-100 focus:ring-4 focus:ring-gray-100 font-medium rounded-lg text-sm px-4 py-2 whitespace-nowrap focus:outline-none dark:bg-gray-800 dark:text-white dark:border-gray-600 dark:hover:bg-gray-700 dark:focus:ring-gray-700',
    // A field is an input or a select, as tall as a button (38 px, mcp-overview-check); the select keeps
    // room on its right for the arrow (pe-9).
    input: `${FIELD} px-3 py-2`,
    select: `${FIELD} py-2 ps-3 pe-9`,
    check: 'w-4 h-4 text-primary-600 bg-gray-100 border-gray-300 rounded-sm focus:ring-primary-500 focus:ring-2 dark:bg-gray-700 dark:border-gray-600 dark:focus:ring-primary-600 dark:ring-offset-gray-800',
    tableWrap: 'relative overflow-x-auto rounded-lg',
    table: 'w-full text-sm text-left text-gray-500 dark:text-gray-400',
    thead: 'text-xs text-gray-700 uppercase bg-gray-50 dark:bg-gray-700 dark:text-gray-400',
    badge: 'text-xs font-medium px-2.5 py-0.5 rounded-sm',
    // A card's header row and its title block. From sm up the row is a grid: the title in the first
    // column, the actions (the row's second child) in the second on the title's line, and a title
    // block with a note dissolves (contents) so the note spans both columns below. A long note no
    // longer pushes the actions under it at any width (mcp-overview-check, 1440 and 1280 px). On a
    // phone the row stacks: title, note, actions.
    cardHead: 'flex flex-col items-start gap-3 sm:grid sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-x-3 sm:gap-y-0 sm:*:nth-2:col-start-2 sm:*:nth-2:row-start-1',
    cardHeadTitle: 'min-w-0 sm:has-[p]:contents sm:*:last:col-span-2',
  });
  /** Flowbite badge tones, one per state; a badge is UI.badge plus one of them. */
  const TONE = Object.freeze({
    green: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300',
    yellow: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300',
    blue: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-300',
    gray: 'bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-300',
    red: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300',
    purple: 'bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-300',
  });
  /** The connection badge and the navbar's buttons, built from the sets above (compact on a phone). */
  const CONN = `shrink-0 whitespace-nowrap ${UI.badge}`;
  const BTN = UI.secondary.replace('px-4 py-2', 'px-3 py-1.5 sm:px-4 sm:py-2');

  const CHEVRON = '<svg class="w-6 h-6 text-gray-400 rtl:rotate-180" aria-hidden="true" fill="currentColor" viewBox="0 0 20 20"><path fill-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" clip-rule="evenodd"></path></svg>';
  const HOME = '<svg class="w-5 h-5 me-2.5" aria-hidden="true" fill="currentColor" viewBox="0 0 20 20"><path d="M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z"></path></svg>';
  const CRUMB_LINK = 'ms-1 text-sm font-medium text-gray-700 hover:text-gray-900 md:ms-2 dark:text-gray-300 dark:hover:text-white';
  const CRUMB_HERE = 'ms-1 text-sm font-medium text-gray-400 md:ms-2 dark:text-gray-500';

  /** One breadcrumb after the home one: a link, or the current page (no href). */
  function crumbHtml(label, href) {
    const inner = href ? `<a href="${esc(href)}" class="${CRUMB_LINK}">${esc(label)}</a>`
      : `<span class="${CRUMB_HERE}" aria-current="page">${esc(label)}</span>`;
    return `<li data-crumb><div class="flex items-center">${CHEVRON}${inner}</div></li>`;
  }

  /** The breadcrumb, derived from the app's registry: its home page, then the current page (a link
      while the path is below it, as /tool/<name> is below /tools). */
  function crumbsHtml(current, path = '/', app = APPS.panel) {
    const home = app.pages[0];
    const page = app.pages.find((p) => p.key === current);
    const out = [`<li data-crumb class="inline-flex items-center"><a href="${home.href}" class="inline-flex items-center text-sm font-medium text-gray-700 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white">${HOME}${esc(home.title)}</a></li>`];
    if (page && page !== home) out.push(crumbHtml(page.title, path === page.href ? null : page.href));
    return out.join('');
  }

  /** The Pro page header (users/settings.html variant B): breadcrumb, H1, optional description, badges.
      #crumbs, #title, #description and #badges are what tool.js and results.js write to. */
  function headerHtml({ current, path = '/', app = APPS.panel, title = '', description = '' } = {}) {
    const page = app.pages.find((p) => p.key === current);
    return `<div id="page-header" class="mb-4">
      <nav class="flex mb-5" aria-label="Breadcrumb"><ol id="crumbs" class="inline-flex flex-wrap items-center gap-y-1 space-x-1 md:space-x-2 rtl:space-x-reverse">${crumbsHtml(current, path, app)}</ol></nav>
      <h1 id="title" class="${UI.h1} break-words">${esc(title || page?.label || '')}</h1>
      <p id="description" class="mt-1 ${UI.cardSub} empty:hidden">${esc(description)}</p>
      <div id="badges" class="flex flex-wrap gap-2 mt-2 empty:hidden"></div>
    </div>`;
  }

  /** A crumb a page script appended (<li>/</li><li>Name</li>, or a link) becomes the shell's own. */
  function adoptCrumbs(ol) {
    for (const li of [...ol.children]) {
      if (li.hasAttribute('data-crumb')) continue;
      const text = li.textContent.trim();
      if (text === '/' || !text) { li.remove(); continue; }
      const a = li.querySelector('a[href]');
      li.insertAdjacentHTML('beforebegin', crumbHtml(text, a ? a.getAttribute('href') : null));
      li.remove();
    }
  }

  /** The sidebar's page list, the current one marked. */
  function sidebarHtml(current, pages = PAGES) {
    return pages.map((p) => `<li><a href="${p.href}"${p.key === current ? ' aria-current="page"' : ''} title="${esc(p.label)}" class="${LINK}${p.key === current ? ' bg-gray-100 dark:bg-gray-700' : ''}">${p.icon ? icon(p.icon) : ''}<span data-label class="truncate">${esc(p.label)}</span></a></li>`).join('');
  }

  /** "On this page": one entry per [data-nav] element with an id ({id, label}), in document order. */
  function sectionsHtml(sections) {
    if (!sections.length) return '';
    return `<p data-label class="px-2 pt-4 pb-1 text-xs font-semibold tracking-wide text-gray-500 uppercase dark:text-gray-400">On this page</p>`
      + sections.map((s) => `<li><a href="#${esc(s.id)}" class="${SUB}">${esc(s.label)}</a></li>`).join('');
  }

  function navbarHtml(app = APPS.panel) {
    return `<nav id="shell-nav" class="fixed top-0 z-30 w-full bg-white border-b border-gray-200 dark:bg-gray-800 dark:border-gray-700">
    <div class="px-3 py-3 lg:px-5 lg:ps-3"><div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-3 min-w-0">
        <button data-drawer-target="sidebar" data-drawer-toggle="sidebar" aria-controls="sidebar" type="button" class="p-2 text-gray-600 rounded-sm cursor-pointer lg:hidden hover:text-gray-900 hover:bg-gray-100 focus:bg-gray-100 dark:focus:bg-gray-700 focus:ring-2 focus:ring-gray-100 dark:focus:ring-gray-700 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white"><span class="sr-only">Toggle sidebar</span><svg class="w-6 h-6" aria-hidden="true" fill="currentColor" viewBox="0 0 20 20"><path fill-rule="evenodd" d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zM3 10a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zM3 15a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z" clip-rule="evenodd"></path></svg></button>
        <button id="sidebar-collapse" type="button" aria-controls="sidebar" aria-expanded="true" title="Collapse or expand the sidebar ([)" class="hidden lg:inline-flex p-2 text-gray-600 rounded-lg hover:text-gray-900 hover:bg-gray-100 focus:ring-2 focus:ring-gray-100 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white dark:focus:ring-gray-700"><span class="sr-only">Collapse or expand the sidebar</span>${icon('M4 5h16v14H4zM9 5v14', 'w-5 h-5')}</button>
        <a href="/" class="self-center text-lg font-semibold tracking-tight whitespace-nowrap dark:text-white">${esc(app.brand)}</a>
        <span class="hidden sm:inline bg-yellow-100 text-yellow-800 text-xs font-medium px-2.5 py-0.5 rounded-sm dark:bg-yellow-900 dark:text-yellow-300">RESEARCH-ONLY</span>
        <span id="conn" class="shrink-0 whitespace-nowrap bg-gray-100 text-gray-800 text-xs font-medium px-2.5 py-0.5 rounded-sm dark:bg-gray-700 dark:text-gray-300">connecting</span>
      </div>
      <div id="nav-extra" class="flex items-center gap-2 min-w-0 text-sm text-gray-500 dark:text-gray-400">
        <a href="/docs" class="hidden sm:inline-block ${BTN}">API</a>
        ${app.console ? `<button id="open-console" type="button" class="${BTN}">Console <kbd class="px-1.5 py-0.5 text-xs font-semibold text-gray-800 bg-gray-100 border border-gray-200 rounded-lg dark:bg-gray-600 dark:text-gray-100 dark:border-gray-500">~</kbd></button>` : ''}
      </div>
    </div></div></nav>`;
  }

  function asideHtml(current, app = APPS.panel) {
    return `<aside id="sidebar" class="fixed top-0 left-0 z-20 flex flex-col flex-shrink-0 w-64 lg:w-[var(--sidebar-w)] h-full pt-16 transition-[transform,width] duration-200 ease-out -translate-x-full bg-white border-r border-gray-200 lg:translate-x-0 dark:bg-gray-800 dark:border-gray-700" aria-label="Sidebar">
    <div class="flex-1 px-3 pt-5 pb-4 overflow-y-auto">
      <ul class="space-y-1 font-medium">${sidebarHtml(current, app.pages)}
        ${app.console ? `<li><button type="button" id="side-console" title="Developer console (~)" class="${LINK} w-full">${icon(CONSOLE_ICON)}<span data-label class="truncate">Developer console (~)</span></button></li>` : ''}
      </ul>
      <ul id="shell-sections" class="space-y-1 font-medium"></ul>
      <div data-label class="pt-4 mt-4 space-y-2 text-sm text-gray-500 border-t border-gray-200 dark:border-gray-700 dark:text-gray-400">
        <p>Docs: <code>${esc(app.doc)}</code></p>
      </div>
    </div></aside>`;
  }

  /* ---- in the browser ------------------------------------------------------------------------- */
  /** The sidebar state to start with: the owner's saved choice, else full. */
  function sidebarState(saved) { return SIDEBAR_STATES.includes(saved) ? saved : 'full'; }

  if (typeof document !== 'undefined' && document.body) {
    // Before anything is drawn, so the frame never jumps from full to rail.
    let saved = null;
    try { saved = localStorage.getItem(SIDEBAR_KEY); } catch { /* storage blocked: full */ }
    document.documentElement.dataset.sidebar = sidebarState(saved);
    const app = APPS[document.body.dataset.app] || APPS.panel;
    const current = document.body.dataset.page || pageOf(location.pathname, document.body.dataset.app);
    document.body.insertAdjacentHTML('afterbegin', navbarHtml(app) + asideHtml(current, app));
    // The page header opens <main> as soon as the parser has created it, before any page script runs.
    const header = () => {
      const main = document.querySelector('body > main');
      if (!main || document.getElementById('page-header')) return !!main;
      const { title, description } = document.body.dataset;
      main.insertAdjacentHTML('afterbegin', headerHtml({ current, path: location.pathname, app, title, description }));
      const ol = document.getElementById('crumbs');
      new MutationObserver(() => adoptCrumbs(ol)).observe(ol, { childList: true });
      return true;
    };
    if (!header()) {
      const watch = new MutationObserver(() => { if (header()) watch.disconnect(); });
      watch.observe(document.body, { childList: true });
      document.addEventListener('DOMContentLoaded', () => { watch.disconnect(); header(); });
    }
    const setSidebar = (state) => {
      document.documentElement.dataset.sidebar = state;
      const b = document.getElementById('sidebar-collapse');
      if (b) b.setAttribute('aria-expanded', String(state === 'full'));
      try { localStorage.setItem(SIDEBAR_KEY, state); } catch { /* not kept */ }
    };
    const toggleSidebar = () => setSidebar(document.documentElement.dataset.sidebar === 'rail' ? 'full' : 'rail');
    document.getElementById('sidebar-collapse').onclick = toggleSidebar;
    setSidebar(document.documentElement.dataset.sidebar);
    document.addEventListener('keydown', (e) => {
      const t = e.target; const typing = t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName || ''));
      if (e.key === '[' && !typing && !e.metaKey && !e.ctrlKey && !e.altKey) { e.preventDefault(); toggleSidebar(); }
    });
    globalThis.PanelSidebar = { set: setSidebar, toggle: toggleSidebar };
    globalThis.PanelUI = UI;
    globalThis.PanelTONE = TONE;
    globalThis.PanelKit = KIT;
    if (app.console) {
      const toConsole = () => { location.href = app.console; };
      document.getElementById('open-console').onclick = toConsole;
      document.getElementById('side-console').onclick = toConsole;
    }
    document.addEventListener('DOMContentLoaded', () => {
      const main = document.querySelector('main');
      if (main) {
        // One frame for every page's content: under the navbar, beside the sidebar, the same gutter.
        main.classList.add(...FRAME);
        if (!/(^|\s)pb-/.test(main.className)) main.classList.add('pb-16');
        const sections = [...main.querySelectorAll('[data-nav][id]')].map((el) => ({ id: el.id, label: el.dataset.nav }));
        document.getElementById('shell-sections').innerHTML = sectionsHtml(sections);
        const extra = document.getElementById('nav-extra');
        for (const el of main.querySelectorAll('[data-nav-extra]')) extra.prepend(el);
      }
      // The connection badge on every page, from the app's own status (the hub's script sets it itself).
      const conn = document.getElementById('conn');
      if (!app.status) return;
      const poll = async () => {
        try {
          const s = await (await fetch(app.status)).json();
          conn.textContent = s.mcp_up ? 'MCP up' : (s.server_running ? 'server starting' : 'server down');
          conn.className = `${CONN} ${s.mcp_up ? TONE.green : TONE.red}`;
        } catch { conn.textContent = 'panel unreachable'; }
      };
      poll();
      setInterval(poll, 5000);
    });
  }

  if (typeof module !== 'undefined') {
    module.exports = { PAGES, APPS, FRAME, UI, TONE, KIT, SIDEBAR_KEY, SIDEBAR_STATES, sidebarState, pageOf, sidebarHtml, sectionsHtml, navbarHtml, asideHtml, crumbsHtml, headerHtml };
  }
})();
