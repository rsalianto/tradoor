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
  }

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
