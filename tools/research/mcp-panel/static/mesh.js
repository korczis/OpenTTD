'use strict';
/* The Majordomus mesh inside the panel (src/panel_mesh_page.rs): the machines, runtimes and sessions this
   repository's Majordomus server cooperates with, and every node it has discovered. Everything comes
   from the panel's GET /api/mesh, which answers Majordomus's /api/v1/mesh, /peers and /nodes
   unchanged; providers, tallies, trust states and sessions are read from the answer, never typed
   here. Read-only: the page claims, beats and enrols nothing.

   The pure render functions are exported for Node (test/mesh.test.mjs). */
(function mesh() {
  const esc = (v) => String(v ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const num = (v) => (v === null || v === undefined || Number.isNaN(Number(v)) ? '–' : Number(v).toLocaleString('en-US'));
  const shell = typeof module !== 'undefined' ? require('./shell.js') : { UI: globalThis.PanelUI, TONE: globalThis.PanelTONE };
  const badge = (text, tone) => `<span class="${shell.UI.badge} ${shell.TONE[tone] || shell.TONE.gray}">${esc(text)}</span>`;
  const TD = 'px-3 py-2 whitespace-nowrap';
  const ROW = 'border-t border-gray-100 dark:border-gray-700';
  const empty = (cols, text) => `<tr><td class="px-3 py-3 text-sm text-gray-500" colspan="${cols}">${esc(text)}</td></tr>`;

  /** A state word's tone: good states green, bad red, waiting yellow, anything else gray. */
  function tone(state) {
    const s = String(state || '').toLowerCase();
    if (/^(running|trusted|present|active|own|alive|up|linked)$/.test(s)) return 'green';
    if (/^(stopped|failed|error|denied|refused|absent|dead|down|revoked)$/.test(s)) return 'red';
    if (/^(observed|pending|stale|idle|starting|degraded)$/.test(s)) return 'yellow';
    return 'gray';
  }

  /** How long ago an ISO time was, short ("12 s", "3 min", "2 h"), or '' when unreadable. */
  function ago(iso, now = Date.now()) {
    const t = Date.parse(iso);
    if (!Number.isFinite(t)) return '';
    const s = Math.max(0, Math.round((now - t) / 1000));
    return s < 90 ? `${s} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${Math.round(s / 3600)} h`;
  }

  /** The headline figures: whether the mesh runs, this node, the trust policy, and every tally the
      server reports (its keys, in its order). */
  function kpis(m) {
    const mesh = (m && m.mesh) || {};
    return {
      active: mesh.active === true,
      node: mesh.identity?.display_name || mesh.identity?.node_id || '–',
      policy: mesh.trust_policy || '–',
      tallies: Object.entries(mesh.tallies || {}),
      since: mesh.started_at || '',
    };
  }

  /** One row per discovery provider (tailscale, multicast, ...), as the server lists them. */
  function providersHtml(providers) {
    return (providers || []).map((p) => `<tr class="${ROW}">
      <td class="${TD} font-medium text-gray-900 dark:text-white">${esc(p.id)}</td>
      <td class="${TD}">${badge(p.state, tone(p.state))}</td>
      <td class="${TD} text-right tabular-nums">${num(p.sent)}</td>
      <td class="${TD} text-right tabular-nums">${num(p.received)}</td>
      <td class="px-3 py-2 text-xs text-gray-500 dark:text-gray-400">${esc(p.detail)}</td>
    </tr>`).join('');
  }

  /** Every session of every runtime of every machine, flattened: machine, runtime, liveness, session,
      who runs it, what it does, and its claims. */
  function sessionRows(peers) {
    const rows = [];
    for (const m of (peers && peers.machines) || []) {
      for (const r of m.runtimes || []) {
        const sessions = r.sessions && r.sessions.length ? r.sessions : [null];
        for (const s of sessions) rows.push({ machine: m.name || m.node, local: !!m.local, runtime: r.runtime, mine: !!r.this_runtime, liveness: r.liveness, beat: r.last_beat_ms, session: s });
      }
    }
    return rows;
  }

  function peersHtml(peers) {
    return sessionRows(peers).map((x) => {
      const s = x.session;
      const info = (s && s.info) || {};
      const claims = s ? (s.claims || []).map((c) => esc(c.scope || c.path || c.key || JSON.stringify(c))).join(', ') : '';
      return `<tr class="${ROW}">
        <td class="${TD} font-medium text-gray-900 dark:text-white">${esc(x.machine)}${x.local ? ` ${badge('this machine', 'blue')}` : ''}</td>
        <td class="${TD} font-mono text-[11px] text-gray-500">${esc(String(x.runtime || '').split('-').pop())}${x.mine ? ` ${badge('this runtime', 'blue')}` : ''}</td>
        <td class="${TD}">${badge(x.liveness, tone(x.liveness))}${x.beat !== undefined ? ` <span class="text-xs text-gray-400 tabular-nums">${num(Math.round(x.beat / 1000))} s</span>` : ''}</td>
        <td class="${TD}">${s ? `${esc(info.session || s.key)} ${badge(s.state, tone(s.state))}` : '<span class="text-gray-400">no session</span>'}</td>
        <td class="${TD} text-gray-500 dark:text-gray-400">${esc(info.client || info.worker || '')}</td>
        <td class="px-3 py-2 text-xs text-gray-600 dark:text-gray-300 max-w-md break-words">${esc(info.intent || '')}</td>
        <td class="px-3 py-2 font-mono text-[11px] text-gray-500 break-all">${claims || (s ? `${num(s.claims_held || 0)}` : '')}</td>
      </tr>`;
    }).join('');
  }

  /** Every discovered node: name and runtime, trust, presence, version, sources, endpoints, last seen. */
  function nodesHtml(nodes, now = Date.now()) {
    const list = [...((nodes && nodes.nodes) || [])].sort((a, b) =>
      String(a.display_name).localeCompare(String(b.display_name)) || String(a.runtime).localeCompare(String(b.runtime)));
    return list.map((n) => {
      const trust = n.trust?.state || n.trust;
      return `<tr class="${ROW}">
        <td class="${TD} font-medium text-gray-900 dark:text-white">${esc(n.display_name || n.node_id)} <span class="font-mono text-[11px] text-gray-500">${esc(n.runtime)}</span></td>
        <td class="${TD}">${badge(trust, tone(trust))}${n.trust?.detail ? ` <span class="text-xs text-gray-400">${esc(n.trust.detail)}</span>` : ''}</td>
        <td class="${TD}">${badge(n.presence, tone(n.presence))}</td>
        <td class="${TD} font-mono text-[11px] text-gray-500">${esc(n.version)}</td>
        <td class="${TD} text-xs text-gray-500 dark:text-gray-400">${(n.sources || []).map((s) => esc(s.source)).join(', ')}</td>
        <td class="px-3 py-2 font-mono text-[11px] text-gray-500 break-all">${(n.endpoints || []).map(esc).join(' ')}</td>
        <td class="${TD} text-right tabular-nums text-gray-500">${esc(ago(n.last_seen, now))}</td>
      </tr>`;
    }).join('');
  }

  if (typeof module !== 'undefined') { module.exports = { tone, ago, kpis, providersHtml, sessionRows, peersHtml, nodesHtml }; }
  if (typeof document === 'undefined') return;

  const $ = (id) => document.getElementById(id);
  let lastAt = 0;
  const setState = (cls, text) => { $('m-live').className = `inline-block w-2 h-2 rounded-full ${cls}`; $('m-state').textContent = text; };
  const render = (m) => {
    lastAt = Date.now();
    $('m-source').textContent = m.source || '';
    const k = kpis(m);
    $('k-active').innerHTML = m.reachable ? badge(k.active ? 'active' : 'inactive', k.active ? 'green' : 'red') : badge('unreachable', 'red');
    $('k-node').textContent = k.node;
    $('k-policy').textContent = k.policy;
    $('k-since').textContent = k.since ? `since ${k.since}` : '';
    $('k-tallies').innerHTML = k.tallies.map(([key, v]) =>
      `<span class="me-2 whitespace-nowrap"><span class="text-gray-500 dark:text-gray-400">${esc(key)}</span> <span class="font-semibold tabular-nums text-gray-900 dark:text-white">${num(v)}</span></span>`).join('');
    $('providers-body').innerHTML = providersHtml(m.mesh?.providers) || empty(5, 'No provider reported.');
    $('peers-body').innerHTML = peersHtml(m.peers) || empty(7, 'No peer reported.');
    $('nodes-body').innerHTML = nodesHtml(m.nodes) || empty(7, 'No node discovered.');
    const errs = Object.entries(m.errors || {});
    $('m-errors').textContent = errs.map(([p, e]) => `${p}: ${e}`).join(' · ');
    $('m-errors').hidden = !errs.length;
    if (m.reachable) setState('bg-green-500', `live · ${num(m.elapsed_ms)} ms`);
    else setState('bg-red-500', 'Majordomus does not answer (majordomus serve ensure)');
  };
  const poll = async () => {
    if (document.hidden) return;
    try {
      const r = await fetch('/api/mesh');
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      render(await r.json());
    } catch (e) {
      setState('bg-red-500', lastAt ? `stale: ${e.message}` : `GET /api/mesh failed: ${e.message}`);
    }
  };
  poll();
  setInterval(poll, 5000);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });
})();
