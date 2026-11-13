// The owner's status page: reads /api/admin/status with the admin key every
// fifteen seconds while shown. The key stays in this tab (sessionStorage) and
// goes only in the Authorization header. Everything is written as text.
'use strict';
(() => {
  const STORE = 'tradoor-admin-key';
  const EVERY_MS = 15_000;
  const $ = (id) => document.getElementById(id);
  /** Its text, and its colour: ok, bad, warn or none. */
  const set = (id, text, tone) => {
    const el = $(id);
    el.textContent = text;
    el.classList.remove('ok', 'bad', 'warn');
    if (tone) el.classList.add(tone);
  };
  const stored = () => {
    try {
      return sessionStorage.getItem(STORE) ?? '';
    } catch {
      return '';
    }
  };
  let key = stored();
  let timer = 0;

  const remember = (value) => {
    key = value;
    try {
      if (value) sessionStorage.setItem(STORE, value);
      else sessionStorage.removeItem(STORE);
    } catch {
      /* a private window: kept for this page alone */
    }
  };
  const n = (v) => (typeof v === 'number' ? v.toLocaleString('en-US') : '–');
  const clock = (ms) => new Date(ms).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  const span = (secs) => {
    const d = Math.floor(secs / 86400), h = Math.floor((secs % 86400) / 3600), m = Math.floor((secs % 3600) / 60);
    return d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`;
  };
  const pool = (p) => (p ? `${p.inUse} / ${p.max} in use${p.waiting ? ` · ${p.waiting} waiting` : ''}` : '–');

  function locked(note, bad) {
    clearTimeout(timer);
    $('gate').hidden = false;
    $('view').hidden = true;
    $('lock').hidden = true;
    set('gate-note', note, bad ? 'bad' : '');
    $('key').focus();
  }

  function show(s) {
    document.body.classList.remove('stale');
    $('gate').hidden = true;
    $('view').hidden = false;
    $('lock').hidden = false;
    const u = s.users ?? {};
    set('online', n(u.online));
    set('online-of', `${n(u.signedIn)} signed in`);
    set('recent', n(u.last15m));
    set('today', n(u.today));
    set('peak', typeof u.peakToday === 'number' ? `${n(u.peakToday)} at ${clock(u.peakAt)}` : '–');
    const t = s.telegram ?? {};
    set('bot', t.bot ? '● bot on' : '● no bot set up', t.bot ? 'ok' : 'warn');
    set('accounts', n(t.accounts));
    set('new-today', n(t.newToday));
    set('seen', n(t.seen24h));
    set('chats', n(t.alertChats));
    const v = s.server ?? {};
    const db = v.db ?? {};
    set('state', db.ok ? '● ok' : '● database not answering', db.ok ? 'ok' : 'bad');
    set('up', `${v.version ?? '?'} · ${span(v.upSecs ?? 0)}`);
    set('db', db.ok ? `${db.ms} ms` : 'no answer', db.ok ? (db.ms > 200 ? 'warn' : '') : 'bad');
    set('requests', pool(v.requests), v.requests?.waiting ? 'warn' : '');
    set('work', pool(v.work), v.work?.waiting ? 'warn' : '');
    set('memory', typeof v.memoryMb === 'number' ? `${v.memoryMb} MB` : '–');
    set('load', Array.isArray(v.load) ? v.load.map((x) => x.toFixed(2)).join('  ') : '–');
    set('when', `updated ${new Date().toLocaleTimeString('en-GB')}`);
    const fx = s.scripts ?? {};
    set('fx-total', n(fx.total));
    set('fx-authors', n(fx.authors));
    set('fx-public', n(fx.public));
    set('fx-new', n(fx.newToday));
    set('fx-removed', n(fx.removed), fx.removed ? 'warn' : '');
    if (!announced) void announcement();
    // Folded and not read yet: how many there are, from the counts.
    if (!scriptsRead) set('fx-summary', typeof fx.total === 'number' ? `${n(fx.total)} ${fx.total === 1 ? 'script' : 'scripts'}: open to read them` : 'The scripts');
  }

  // ---- Frost Script moderation: every script, the last changed first.
  let showing = 'all';
  let scriptsRead = 0;
  const admin = (path, init = {}) => fetch(path, { ...init, cache: 'no-store', headers: { authorization: `Bearer ${key}`, ...(init.body ? { 'content-type': 'application/json' } : {}) } });
  const el = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  };
  const when = (ms) => new Date(ms).toLocaleString('en-GB', { day: '2-digit', month: 'short', hour: '2-digit', minute: '2-digit' });

  async function scripts() {
    scriptsRead = Date.now();
    const list = $('fx-list');
    const q = showing === 'all' ? '' : `?removed=${showing === 'removed'}`;
    let res;
    try {
      res = await admin(`/api/admin/scripts${q}`);
    } catch {
      res = null;
    }
    if (!res?.ok) {
      list.replaceChildren(el('div', 'empty', res ? `The server answered ${res.status}.` : 'No answer from the server.'));
      return;
    }
    const { scripts: rows } = await res.json();
    const which = { all: 'scripts', live: 'live scripts', removed: 'removed scripts' }[showing];
    // The server gives the newest 300 at most.
    set('fx-summary', rows.length ? `${n(rows.length)} ${which}${rows.length >= 300 ? ', the newest 300' : ''}` : `No ${which}`);
    if (!rows.length) {
      list.replaceChildren(el('div', 'empty', showing === 'removed' ? 'None removed.' : 'No scripts yet.'));
      return;
    }
    list.replaceChildren(...rows.map(row));
    find();
  }

  /** Only the rows whose name or author holds what is typed. */
  function find() {
    const q = $('fx-find').value.trim().toLowerCase();
    let shown = 0;
    for (const box of $('fx-list').querySelectorAll('.script')) {
      box.hidden = q !== '' && !box.dataset.find.includes(q);
      if (!box.hidden) shown += 1;
    }
    const none = $('fx-list').querySelector('.none');
    if (q && shown === 0 && !none) $('fx-list').append(el('div', 'empty none', 'Nothing matches.'));
    else if ((!q || shown > 0) && none) none.remove();
  }

  function row(s) {
    const box = el('div', 'script');
    box.dataset.find = `${s.name} ${s.author} ${s.authorId} ${s.username ?? ''} ${s.about ?? ''}`.toLowerCase();
    const head = el('div', 'head');
    head.append(el('span', 'name', s.name), el('span', s.public ? 'pub' : 'meta', s.public ? `public as @${s.username ?? '?'}` : 'private'), el('span', 'meta', `by ${s.author} · ${s.authorId} · ${when(s.updated)} · ${(s.size / 1024).toFixed(1)} kB · ${s.checks ? 'checks' : 'has a mistake'}`));
    if (s.removed) head.append(el('span', 'gone', `removed${s.why ? `: ${s.why}` : ''}`));
    if (s.public) {
      const shown = el('a', 'meta', 'on the showcase ↗');
      shown.href = `/community.html#${encodeURIComponent(s.id)}`;
      shown.target = '_blank';
      shown.rel = 'noopener';
      head.append(shown);
    }
    const acts = el('span', 'acts');
    const view = el('button', '', 'Source');
    let pre = null;
    view.onclick = async () => {
      if (pre) {
        pre.remove();
        pre = null;
        return;
      }
      const r = await admin(`/api/admin/scripts/${encodeURIComponent(s.id)}`).catch(() => null);
      pre = el('pre', '', r?.ok ? (await r.json()).source : 'Not read.');
      box.append(pre);
    };
    acts.append(view);
    const act = async (action, why) => {
      const r = await admin(`/api/admin/scripts/${encodeURIComponent(s.id)}`, { method: 'POST', body: JSON.stringify({ action, why }) }).catch(() => null);
      if (!r?.ok) alert(`Not done: the server answered ${r ? r.status : 'nothing'}.`);
      void scripts();
      void read();
    };
    if (s.removed) {
      const back = el('button', '', 'Restore');
      back.onclick = () => act('restore');
      acts.append(back);
    } else {
      const off = el('button', 'danger', 'Remove');
      off.onclick = () => {
        const why = prompt(`Remove "${s.name}"? It is kept, and runs for no one. Why, for its author:`, '');
        if (why !== null) void act('remove', why);
      };
      acts.append(off);
    }
    const del = el('button', 'danger', 'Delete');
    del.onclick = () => {
      if (confirm(`Delete "${s.name}" for good? This cannot be undone.`)) void act('delete');
    };
    acts.append(del);
    head.append(acts);
    box.append(head);
    if (s.about) box.append(el('p', 'about', s.about));
    return box;
  }

  // ---- the announcement atop the app: what is up, put up anew, or taken down.
  let announced = false;
  async function announcement() {
    announced = true;
    const r = await admin('/api/admin/announcement').catch(() => null);
    if (!r?.ok) return set('an-state', '● not read', 'bad');
    const { current } = await r.json();
    if (current) {
      $('an-title').value = current.title;
      $('an-body').value = current.body;
      $('an-link').value = current.link ?? '';
      $('an-tone').value = current.tone;
      set('an-state', `● up since ${when(current.id)}`, 'ok');
    } else {
      set('an-state', '● none up', 'warn');
    }
  }
  $('an-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const body = { title: $('an-title').value, body: $('an-body').value, link: $('an-link').value || null, tone: $('an-tone').value };
    const r = await admin('/api/admin/announcement', { method: 'PUT', body: JSON.stringify(body) }).catch(() => null);
    if (r?.ok) {
      set('an-note', 'Published: every reader sees it within a few minutes.');
      void announcement();
    } else {
      const why = r ? (await r.json().catch(() => ({}))).error : null;
      set('an-note', `Not published: ${why ?? 'no answer from the server'}.`, 'bad');
    }
  });
  $('an-down').addEventListener('click', async () => {
    if (!confirm('Take the announcement down for everyone?')) return;
    const r = await admin('/api/admin/announcement', { method: 'DELETE' }).catch(() => null);
    set('an-note', r?.ok ? 'Taken down.' : 'Not taken down: no answer from the server.', r?.ok ? '' : 'bad');
    void announcement();
  });

  for (const b of document.querySelectorAll('[data-show]')) {
    b.addEventListener('click', () => {
      showing = b.dataset.show;
      for (const x of document.querySelectorAll('[data-show]')) x.classList.toggle('on', x === b);
      void scripts();
    });
  }
  $('fx-refresh').addEventListener('click', () => void scripts());
  $('fx-find').addEventListener('input', find);
  // Folded, the list is not read at all: opened, it is, and kept fresh while it stays open.
  $('fx-fold').addEventListener('toggle', () => {
    if ($('fx-fold').open && Date.now() - scriptsRead > 5_000) void scripts();
  });

  async function read() {
    clearTimeout(timer);
    if (!key) return locked('Kept in this tab only, until it is closed.');
    let res;
    try {
      res = await fetch('/api/admin/status', { headers: { authorization: `Bearer ${key}` }, cache: 'no-store' });
    } catch {
      res = null;
    }
    if (res?.status === 401) {
      remember('');
      return locked('That key is not the one the server holds.', true);
    }
    if (res?.status === 404) return locked('The status page is off on this server: set TRADOOR_ADMIN_KEY and restart it.', true);
    if (res?.status === 429) return locked('Too many wrong keys from here: wait a minute.', true);
    if (res?.ok) {
      show(await res.json());
    } else {
      // Kept on screen, faded, with when it was last read.
      document.body.classList.add('stale');
      set('state', res ? `● the server answered ${res.status}` : '● no answer', 'bad');
    }
    if (key && $('fx-fold').open && Date.now() - scriptsRead > 60_000) void scripts();
    if (document.visibilityState === 'visible') timer = setTimeout(read, EVERY_MS);
  }

  $('form').addEventListener('submit', (e) => {
    e.preventDefault();
    // Left in the field, so a password manager can offer to keep it.
    remember($('key').value.trim());
    void read();
  });
  $('lock').addEventListener('click', () => {
    remember('');
    $('key').value = '';
    locked('Locked. The key is gone from this tab.');
  });
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && key) void read();
    else clearTimeout(timer);
  });
  void read();
})();
