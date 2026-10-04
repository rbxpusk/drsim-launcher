// kit/view.js and the art come from deltarunesim.com, see scripts/prep-ui.mjs
import { textKit, frameDataURL, paintThumb, dateLabel, THUMB_W, THUMB_H, dpr } from './kit/view.js';

const T = window.__TAURI__;
const invoke = (c, a) => T.core.invoke(c, a);
const win = T.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);
const el = (tag, cls, parent) => { const e = document.createElement(tag); if (cls) e.className = cls; if (parent) parent.appendChild(e); return e; };
const C = { fg: '#ffffff', sel: '#ffff00', dim: '#9a9a9a', red: '#ff4040' };
const size = (b) => { b = Number(b) || 0; return b >= 1e9 ? (b / 1e9).toFixed(2) + ' GB' : b >= 1e6 ? (b / 1e6).toFixed(b >= 1e8 ? 0 : 1) + ' MB' : Math.max(1, Math.round(b / 1e3)) + ' KB'; };
const eta = (s) => (s < 60 ? Math.max(1, Math.round(s)) + ' sec' : Math.round(s / 60) + ' min');
const cap = (s) => { s = String(s); return s.charAt(0).toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? '' : '.'); };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

const S = {
  st: null, chk: null, checking: false, busy: null, prog: null, err: '', note: '', noteT: 0,
  notes: null, view: 'home', focus: 'home', selId: 'main', nsel: 0, csec: 0, crow: 0, after: null, verifyBad: false,
  splash: true, starting: false, launchDone: true,
};

let F = {}, kit = null, box = null;
const load = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = no; i.src = src; });
async function loadKit() {
  const tab = await (await fetch('kit/fonts.json')).json();
  for (const n of ['fnt_main', 'fnt_mainbig']) F[n] = { ...tab[n], img: await load('kit/' + n + '.png') };
  const [corner, top, left] = await Promise.all(['box_corner', 'box_top', 'box_left'].map((n) => load('kit/' + n + '.png')));
  box = { corner, top, left };
  kit = textKit(F);
  const fu = frameDataURL(box, dpr());
  for (const b of document.querySelectorAll('.box')) { b.style.borderImageSource = `url(${fu})`; b.style.borderImageSlice = (16 * dpr()) + ' fill'; }
}
function bt(parent, text, o = {}) { const w = kit.bt(parent, text, o); kit.paint(w); return w; }
function bt2(parent, text, font = 'fnt_main', s = 1) {
  const a = bt(parent, text, { font, s }); a.classList.add('w');
  const b = bt(parent, text, { font, s, col: C.sel }); b.classList.add('y'); b.setAttribute('aria-hidden', 'true');
  return [a, b];
}
function retext(w, text, col) { if (w._bt.text === text && (!col || w._bt.col === col)) return; w._bt.text = text; if (col) w._bt.col = col; w.querySelector('.sr').textContent = text; kit.paint(w); }
function flash(e) { if (!e) return; e.classList.remove('flash'); void e.offsetWidth; e.classList.add('flash'); }

const SND = {};
function sound(n, v = 0.4) { if (S.splash) return; try { let a = SND[n]; if (!a) a = SND[n] = new Audio('kit/' + n + '.wav'); a.volume = v; a.currentTime = 0; a.play().catch(() => {}); } catch (e) { } }
function note(msg) { S.note = msg; clearTimeout(S.noteT); S.noteT = setTimeout(() => { S.note = ''; renderStatus(); }, 5000); renderStatus(); }

function primary() {
  const st = S.st, c = S.chk, inst = !!st.installed;
  const need = !!(c && c.online && !c.error && c.need_files > 0);
  if (S.busy === 'sync') return { id: 'pause', label: 'PAUSE' };
  if (S.busy) return { id: 'none', label: S.busy === 'verify' ? 'CHECKING' : 'UPDATING', off: true };
  if (!inst) return { id: 'install', label: c && c.staged_bytes ? 'RESUME' : 'INSTALL', off: !need };
  if (need) return { id: 'update', label: S.verifyBad ? 'REPAIR' : 'UPDATE' };
  return { id: 'play', label: 'PLAY' };
}

function extraItems() {
  const c = S.chk, list = [];
  if (S.st.installed && c && c.online && c.need_files > 0 && !S.busy && !S.verifyBad) list.push({ id: 'play', label: 'PLAY v' + S.st.installed });
  if (c && c.launcher_update && !S.busy) list.push({ id: 'lupdate', label: 'NEW LAUNCHER' });
  return list;
}
let items = [], extrasKey = '';
function renderControls() {
  const ex = extraItems(), key = ex.map((i) => i.id + i.label).join('|');
  if (key !== extrasKey) {
    extrasKey = key;
    const box = $('extras'); box.textContent = '';
    ex.forEach((it) => { const b = el('button', 'lk', box); b.dataset.id = it.id; b.setAttribute('aria-label', it.label); bt2(b, it.label); });
  }
  const setup = renderSetup();
  items = [
    ...setup,
    { id: 'notes', node: $('l-notes') }, { id: 'config', node: $('l-config') }, { id: 'discord', node: $('w-discord') },
    ...[...$('extras').children].map((n) => ({ id: n.dataset.id, node: n })),
    { id: 'main', node: $('play') },
  ];
  if (!items.some((r) => r.id === S.selId)) S.selId = 'main';
  mark();
}
function mark() {
  items.forEach((r) => r.node.classList.toggle('on', S.focus === 'home' && r.id === S.selId));
  if (S.view === 'notes') $('l-notes').classList.add('on');
  if (S.view === 'config') $('l-config').classList.add('on');
  const pb = $('play');
  if (playLbl) retext(playLbl, playLbl._bt.text, pb.disabled ? '#5a5a5a' : pb.classList.contains('on') ? C.sel : C.fg);
}
function itemAct(id, node) {
  if (id === 'sfolder') { sound('snd_select'); flash(node); return pickDir(); }
  if (id === 'sshort') { sound('snd_menumove', 0.3); return invoke('desktop_shortcut', { on: !S.st.desktop_shortcut }).then((on) => { S.st.desktop_shortcut = on; renderAll(); }).catch((e) => note(cap(e))); }
  if (id === 'slaunch') { sound('snd_menumove', 0.3); S.launchDone = !S.launchDone; return renderAll(); }
  if (id === 'main') { if ($('play').disabled) return; flash($('play')); sound('snd_select'); return act(primary().id); }
  flash(node); sound('snd_select');
  if (id === 'play') return play();
  if (id === 'lupdate') return launcherUpdate();
  if (id === 'notes' || id === 'config') return openView(S.view === id ? 'home' : id);
  if (id === 'discord') return invoke('open_link', { which: 'discord' });
}
function openView(v) {
  S.view = v;
  $('panel').hidden = v === 'home'; $('notes-view').hidden = v !== 'notes'; $('config-view').hidden = v !== 'config';
  document.body.classList.toggle('panel', v !== 'home');
  S.focus = v;
  if (v !== 'home') { $('p-title').textContent = ''; bt($('p-title'), v === 'notes' ? 'PATCH NOTES' : 'SETTINGS', { font: 'fnt_mainbig', col: C.sel }); }
  if (v === 'notes') renderNotes();
  if (v === 'config') renderConfig();
  mark();
}

const shortPath = (p) => (p.length > 46 ? p.slice(0, 18) + '...' + p.slice(-25) : p);
let setupKey = '';
function renderSetup() {
  const st = S.st, on = !S.splash && !st.installed && !S.busy && S.view === 'home';
  document.body.classList.toggle('setup', !st.installed && !S.busy);
  $('setup').hidden = !on;
  if (!on) return [];
  const rows = [['sfolder', 'GAME FOLDER', 'path'], ...(st.desktop_shortcut != null ? [['sshort', 'DESKTOP SHORTCUT', 'opt', !!st.desktop_shortcut]] : []), ['slaunch', 'PLAY WHEN DONE', 'opt', S.launchDone]];
  const key = JSON.stringify([rows, st.install_dir, st.portable]);
  const box = $('s-rows');
  if (key !== setupKey) {
    setupKey = key; box.textContent = '';
    for (const [id, label, kind, val] of rows) {
      const d = el('div', 'row', box); d.dataset.id = id;
      const c = el('img', 'cur', d); c.src = 'kit/heart.png'; c.alt = '';
      bt2(d, label);
      const v = el('div', 'val', d);
      if (kind === 'path') {
        const p = el('div', 'path', v); bt(p, shortPath(st.install_dir), { col: C.dim });
        if (!st.portable) { const o = el('span', 'opt pick', p); bt(o, 'CHANGE', { col: C.dim }).classList.add('d'); bt2(o, 'CHANGE'); }
      } else for (const [x, t] of [[true, 'ON'], [false, 'OFF']]) { const o = el('span', 'opt' + (x === val ? ' pick' : ''), v); bt(o, t, { col: C.dim }).classList.add('d'); bt2(o, t); }
      d.addEventListener('click', () => { S.focus = 'home'; S.selId = id; mark(); itemAct(id, d); });
      d.addEventListener('mouseenter', () => { if (S.focus === 'home' && S.selId !== id) { S.selId = id; mark(); sound('snd_menumove', 0.25); } });
    }
  }
  return [...box.children].map((n) => ({ id: n.dataset.id, node: n }));
}

async function act(id) {
  if (!id || id === 'none') return;
  if (id === 'play') return play();
  if (id === 'install' || id === 'update') { S.after = (id === 'update' && !S.verifyBad) || (id === 'install' && S.launchDone) ? 'play' : null; return download(); }
  if (id === 'pause') return invoke('cancel_sync');
}
async function play() { S.err = ''; try { await invoke('play'); } catch (e) { S.err = cap(e); renderStatus(); } }
function maybePrewarm(delay = 0) {
  setTimeout(() => {
    const c = S.chk;
    if (!S.st || !S.st.installed || S.busy || S.st.game_open || S.starting) return;
    if (c && c.online && !c.error && c.need_files > 0) return;
    invoke('prewarm').catch(() => {});
  }, delay);
}
async function download() {
  if (S.busy) return false;
  S.busy = 'sync'; S.err = '';
  S.prog = { phase: 'download', done: 0, total: S.chk ? S.chk.need_bytes : 0, files_done: 0, files_total: S.chk ? S.chk.need_files : 0, bps: 0 };
  renderAll();
  let ok = false;
  try { await invoke('sync'); ok = true; }
  catch (e) {
    const m = String(e);
    S.err = m === 'cancelled' ? 'Paused. RESUME carries on where it stopped.' : /offline|network/.test(m) ? 'The connection dropped. RESUME carries on where it stopped.' : 'The download stopped: ' + m + '. Try again; SETTINGS > STORAGE > LOGS has the details.';
  }
  S.busy = null; S.prog = null;
  S.st = await invoke('launcher_state');
  if (ok) { if (S.chk) Object.assign(S.chk, { need_files: 0, need_bytes: 0, staged_bytes: 0 }); if (!S.splash) note(S.verifyBad ? 'Repaired.' : 'v' + S.st.installed + ' is installed.'); S.verifyBad = false; refreshNotes(false); }
  else if (/^Paused/.test(S.err)) { const e = S.err; await check(); S.err = e; }
  renderAll();
  if (ok && S.after === 'play') { S.after = null; play(); }
  else if (ok && !S.splash) maybePrewarm();
  return ok;
}
async function launcherUpdate() {
  if (S.st.portable) return invoke('open_link', { which: 'portable' });
  S.busy = 'lupdate'; S.prog = { done: 0, total: 0 }; renderAll();
  try { await invoke('install_launcher_update'); } catch (e) { S.busy = null; S.err = 'The launcher update failed: ' + e; renderAll(); }
}
let checkP = null;
function check() {
  if (S.checking) return checkP;
  if (S.busy) return Promise.resolve();
  S.checking = true; S.err = ''; renderStatus();
  return (checkP = (async () => {
    try { S.chk = await invoke('check'); } catch (e) { S.chk = { online: false, error: String(e), need_files: 0 }; }
    S.checking = false;
    renderAll();
    if (S.chk && S.chk.online) refreshNotes(true);
    if (!S.splash) maybePrewarm();
  })());
}
async function verify() {
  if (S.busy) return;
  S.busy = 'verify'; S.prog = { phase: 'verify', done: 0, total: 1, files_done: 0, files_total: 0 }; renderAll();
  try {
    const r = await invoke('verify');
    S.busy = null; S.prog = null; S.st = await invoke('launcher_state');
    if (r.bad) { S.verifyBad = true; await check(); note(r.bad + (r.bad > 1 ? ' files are' : ' file is') + ' broken. REPAIR downloads ' + (r.bad > 1 ? 'them' : 'it') + ' again.'); }
    else note('All ' + r.checked + ' files are fine.');
  } catch (e) { S.busy = null; S.prog = null; note(cap(e)); }
  renderAll();
}

let playLbl = null, stMain = null, stSub = null;
function renderStatus() {
  const st = S.st, c = S.chk, p = S.prog, v = st.installed;
  let main = '', sub = '', subCol = C.dim, pct = -1;
  if (S.busy === 'sync' && p) {
    pct = p.total ? Math.min(100, (p.done / p.total) * 100) : 0;
    if (p.phase === 'install' || p.phase === 'done') { main = 'Installing...'; sub = 'Moving the new files into place.'; pct = 100; }
    else {
      main = (v ? 'Updating... ' : 'Downloading... ') + p.files_done + ' of ' + p.files_total + ' files';
      sub = size(p.done) + ' of ' + size(p.total) + (p.bps > 1e4 ? ',  ' + size(p.bps) + '/s,  ' + eta((p.total - p.done) / p.bps) + ' left' : '') + (S.splash ? '.  Please don\'t close this window.' : '');
    }
  } else if (S.busy === 'verify' && p) { pct = p.total ? Math.min(100, (p.done / p.total) * 100) : 0; main = 'Checking your files... ' + p.files_done + ' of ' + p.files_total; sub = 'Broken files can be downloaded again afterwards.'; }
  else if (S.busy === 'lupdate' && p) { pct = p.total ? (p.done / p.total) * 100 : 0; main = 'Updating the launcher...'; sub = 'It restarts by itself.'; }
  else if (S.starting) { main = 'Starting...'; sub = v ? 'Version ' + v : ''; pct = -2; }
  else if (S.checking) { main = 'Checking for updates...'; sub = v ? 'Version ' + v + ' is installed.' : 'This takes a moment the first time.'; pct = -2; }
  else if (S.err) { main = v ? 'Version ' + v : 'Not installed'; sub = S.err; subCol = C.red; }
  else if (c && c.error === 'not-published') {
    main = v ? 'Version ' + v : 'Not on the server yet';
    sub = v ? 'The update server has no desktop files right now. Your copy plays.' : 'deltarunesim.com has no desktop game files yet. They go up with the next site update.';
    if (!v) subCol = C.red;
  } else if (c && c.error === 'blocked') { main = 'Download blocked'; sub = "The site's bot check refused the launcher. Try again in a minute."; subCol = C.red; }
  else if (c && c.error === 'newer') { main = 'This launcher is too old'; sub = 'Choose NEW LAUNCHER to get the newest game.'; subCol = C.red; }
  else if (c && c.error) {
    const mode = c.error === 'offline-mode';
    main = v ? 'Playing offline' : mode ? 'Offline mode' : 'Offline';
    sub = v ? 'Version ' + v + '.' + (mode ? ' Offline mode is on (SETTINGS > UPDATES).' : ' Updates come when you are back online.') : (mode ? 'Turn offline mode off in SETTINGS > UPDATES to download the game.' : 'Connect to the internet to download the game (0.9 GB).');
    if (!v) subCol = C.red;
  } else if (c && c.online && c.need_files > 0) {
    main = S.verifyBad ? c.need_files + ' files to repair' : (v ? 'Update ready: v' + c.version : 'Ready to install');
    sub = (v ? '' : 'Version ' + c.version + ':  ') + c.need_files + ' files, ' + size(c.need_bytes) + (st.free_bytes != null ? '.  ' + size(st.free_bytes) + ' free on this drive.' : '.');
  } else if (c && c.online) { main = 'Up to date'; sub = 'Version ' + (v || c.version) + (c.title ? ':  ' + c.title : ''); }
  else if (v) { main = 'Version ' + v; sub = st.settings.auto_check ? '' : 'Update checks are off (SETTINGS > UPDATES).'; }
  else main = 'Not installed';
  if (S.note && !S.busy) { sub = S.note; subCol = C.fg; }
  if (!stMain) { stMain = bt($('st-main'), main, { font: 'fnt_mainbig' }); stSub = bt($('st-sub'), sub || ' ', { col: subCol, wrap: true }); }
  else { retext(stMain, main); retext(stSub, sub || ' ', subCol); }
  const pr = $('prog'), bar = pr.querySelector('i');
  pr.classList.toggle('run', pct === -2); pr.classList.toggle('idle', pct === -1);
  bar.style.width = pct >= 0 ? pct.toFixed(1) + '%' : '';
  bar.style.background = S.busy === 'verify' ? C.sel : '';
  const p0 = primary(), b = $('play');
  b.disabled = !!p0.off; b.dataset.act = p0.id;
  if (!playLbl) playLbl = bt($('play-lbl'), p0.label, { font: 'fnt_mainbig', s: 2 });
  retext(playLbl, p0.label, p0.off ? '#5a5a5a' : b.classList.contains('on') ? C.sel : C.fg);
  b.setAttribute('aria-label', p0.label);
}
function renderAll() { renderControls(); renderStatus(); if (S.view === 'config') renderConfig(); }

const updates = () => (S.notes && S.notes.data && Array.isArray(S.notes.data.updates) ? S.notes.data.updates.filter((e) => e && e.version) : []);
async function refreshNotes(online) {
  try { const n = await invoke('notes', { online }); if (n && n.data) { S.notes = n; if (S.view === 'notes') renderNotes(); } } catch (e) { }
}
function thumb(parent, e, k) {
  const cv = el('canvas', 'th', parent); cv.setAttribute('aria-hidden', 'true');
  paintThumb(cv, e, { F, spriteURL: (n, f) => (S.notes.sprites || {})[n + '_' + f] || 'data:,' }, k * dpr());
  cv.style.width = THUMB_W * k + 'px'; cv.style.height = THUMB_H * k + 'px';
}
function renderNotes() {
  const list = $('n-list'), det = $('n-detail'); list.textContent = ''; det.textContent = '';
  const U = updates();
  if (!U.length) { bt(det, S.chk && !S.chk.online && !S.st.installed ? 'The notes show once the game is downloaded.' : 'No notes yet.', { col: C.dim, wrap: true }); return; }
  S.nsel = Math.max(0, Math.min(S.nsel, U.length - 1));
  U.forEach((e, i) => {
    const b = el('button', 'ver' + (i === S.nsel ? ' on' : ''), list);
    const c = el('img', 'cur', b); c.src = 'kit/heart_small.png'; c.alt = '';
    bt2(b, 'v' + e.version);
    b.addEventListener('click', () => { S.focus = 'notes'; S.nsel = i; sound('snd_menumove', 0.25); renderNotes(); });
  });
  const on = list.children[S.nsel]; if (on) on.scrollIntoView({ block: 'nearest' });
  const e = U[S.nsel];
  thumb(det, e, 1);
  bt(det, e.title || '', { font: 'fnt_mainbig', col: C.sel, wrap: true });
  el('div', 'gap', det);
  bt(det, dateLabel(e.date, true).toUpperCase(), { col: C.dim });
  const ul = el('ul', '', det);
  for (const n of e.notes || []) bt(el('li', '', ul), '* ' + n, { wrap: true });
  det.scrollTop = 0;
}

async function setting(key, value) {
  try { S.st.settings = await invoke('set_setting', { key, value }); sound('snd_menumove', 0.3); }
  catch (e) { note(cap(e)); }
  renderAll();
}
const ONOFF = [[false, 'OFF'], [true, 'ON']];
function sections() {
  const st = S.st, s = st.settings, k = S.chk;
  const opt = (key, opts, help) => ({ kind: 'opt', key, opts, help, cur: s[key] });
  return [
    ['GAME', [
      { label: 'WINDOW', ...opt('window_mode', [['windowed', 'WINDOWED'], ['borderless', 'BORDERLESS'], ['fullscreen', 'FULLSCREEN']], 'How the game window opens. F11 switches fullscreen while you play.') },
      { label: 'WINDOW SIZE', ...opt('window_scale', [[0, 'AUTO'], [1, '1X'], [2, '2X'], [3, '3X'], [4, '4X']], 'In whole multiples of 640x480. AUTO is the largest 4:3 window that fits the screen.') },
      { label: 'MUSIC', kind: 'vol', key: 'music', help: 'Set in the game the next time it starts. The game\'s own CONFIG can change it again.' },
      { label: 'SOUND EFFECTS', kind: 'vol', key: 'sfx', help: 'Set in the game the next time it starts.' },
      { label: 'PERFORMANCE MODE', ...opt('perf', ONOFF, 'Tells the game to go easy on slower PCs.') },
      { label: 'START IN THE GAME', ...opt('start_in_game', ONOFF, 'Opens the game as soon as the launcher starts. Updates still come in the background.') },
    ]],
    ['LAUNCHER', [
      { label: 'WHILE PLAYING', ...opt('on_play', [['hide', 'HIDE'], ['minimize', 'MINIMIZE'], ['keep', 'KEEP OPEN']], 'What the launcher does when the game opens. It comes back when the game closes.') },
      { label: 'CLOSE TO TRAY', ...opt('close_to_tray', ONOFF, 'The X button keeps the launcher in the system tray instead of quitting.') },
      { label: /windows/i.test(st.os) ? 'START WITH WINDOWS' : 'START ON LOGIN', ...opt('autostart', ONOFF, 'Starts quietly in the tray when you log in.') },
      { label: 'DISCORD STATUS', ...opt('discord', ONOFF, 'Shows what you are fighting on your Discord profile while the game is open.') },
    ]],
    ['UPDATES', [
      { label: 'CHECK FOR UPDATES', ...opt('auto_check', [[true, 'ON START'], [false, 'MANUAL']], 'When the launcher starts, or only when you ask.') },
      { label: 'CHANNEL', ...opt('channel', [['stable', 'STABLE'], ['beta', 'BETA']], 'BETA gets test builds first when there is one; otherwise it is the same as STABLE.'), after: () => check() },
      { label: 'OFFLINE MODE', ...opt('offline', ONOFF, 'Never uses the internet: no update checks, no stats. The game plays from your copy.'), after: (v) => { if (v) { S.chk = { online: false, error: 'offline-mode', need_files: 0 }; renderAll(); } else check(); } },
      { label: 'CHECK NOW', kind: 'act', value: S.checking ? 'CHECKING' : k && k.online && !k.error ? (k.need_files ? 'v' + k.version + ' READY' : 'UP TO DATE') : k ? (k.error || 'OFFLINE').toUpperCase() : '', run: () => check(), help: 'Looks for a new version of the game and of the launcher.' },
    ]],
    ['STORAGE', [
      { label: 'INSTALL FOLDER', kind: 'act', value: st.portable ? 'PORTABLE' : 'CHANGE', run: pickDir, help: (st.installed ? 'The game uses ' + size(st.installed_bytes) : 'The game needs 0.9 GB') + (st.free_bytes != null ? '. ' + size(st.free_bytes) + ' free on this drive.' : '.') },
      { label: st.install_dir, kind: 'path' },
      { label: 'OPEN THE FOLDER', kind: 'act', value: 'OPEN', run: () => invoke('open_link', { which: 'data' }), help: 'Opens the install folder.' },
      { label: 'VERIFY FILES', kind: 'act', value: S.busy === 'verify' ? 'CHECKING' : 'VERIFY', run: verify, help: 'Checks every file against the update list. Broken files are downloaded again with REPAIR.' },
      { label: 'CLEAR CACHE', kind: 'act', value: 'CLEAR', run: async () => { try { const n = await invoke('clear_cache'); note('Cleared ' + size(n) + ' now. The web caches clear at the next start; saves stay.'); } catch (e) { note(cap(e)); } }, help: 'Removes unfinished downloads now and the web caches at the next start. Your saves stay.' },
      { label: 'LOGS', kind: 'act', value: 'OPEN', run: () => invoke('open_link', { which: 'logs' }), help: 'Opens the folder with launcher.log. Send that file with a bug report.' },
    ]],
    ['ABOUT', [
      { label: 'LAUNCHER', kind: 'info', value: 'v' + st.launcher_version + (st.portable ? ' PORTABLE' : '') },
      { label: 'GAME', kind: 'info', value: st.installed ? 'v' + st.installed : 'NOT INSTALLED' },
      { label: 'WEBVIEW', kind: 'info', value: st.webview_version || '?' },
      { label: 'SYSTEM', kind: 'info', value: st.os.toUpperCase() },
      { label: 'WEBSITE', kind: 'act', value: 'OPEN', run: () => invoke('open_link', { which: 'site' }), help: 'deltarunesim.com in your browser.' },
      { label: 'ALL PATCH NOTES', kind: 'act', value: 'OPEN', run: () => invoke('open_link', { which: 'updates' }), help: 'deltarunesim.com/updates in your browser.' },
      { label: 'FAN-MADE', kind: 'info', value: '', help: 'An unofficial fan-made project. DELTARUNE and UNDERTALE (c) Toby Fox. Not affiliated with or endorsed by Toby Fox.' },
    ]],
  ];
}
async function pickDir() {
  if (S.st.portable) return;
  try { const d = await invoke('pick_install_dir'); if (d) { S.st = await invoke('launcher_state'); note('The game is now in ' + d + '.'); } } catch (e) { note(cap(e)); }
  renderAll();
}
let crow = [];
function renderConfig() {
  const secs = sections(), tabs = $('c-tabs'), rows = $('c-rows'), help = $('c-help');
  S.csec = (S.csec + secs.length) % secs.length;
  const keep = rows.scrollTop;
  tabs.textContent = ''; rows.textContent = ''; help.textContent = '';
  secs.forEach(([name], i) => {
    const b = el('button', i === S.csec ? 'on' : '', tabs); bt2(b, name);
    b.addEventListener('click', () => { S.csec = i; S.crow = 0; S.focus = 'config'; sound('snd_menumove', 0.3); renderConfig(); });
  });
  const list = secs[S.csec][1];
  crow = [];
  list.forEach((r, i) => {
    if (r.kind === 'path') { const d = el('div', 'row sub', rows); el('span', '', d); bt(d, r.label, { col: C.dim }); return; }
    const d = el('div', 'row', rows); const idx = crow.length;
    const c = el('img', 'cur', d); c.src = 'kit/heart.png'; c.alt = '';
    bt2(d, r.label);
    const v = el('div', 'val', d);
    if (r.kind === 'opt') r.opts.forEach(([val, label]) => {
      const o = el('button', 'opt' + (val === r.cur ? ' pick' : ''), v);
      bt(o, label, { col: C.dim }).classList.add('d'); bt2(o, label);
      o.addEventListener('click', (ev) => { ev.stopPropagation(); S.focus = 'config'; S.crow = idx; if (val !== r.cur) setting(r.key, val).then(() => r.after && r.after(val)); });
    });
    else if (r.kind === 'vol') {
      const n = S.st.settings[r.key], cells = el('div', 'cells', v);
      for (let k = 1; k <= 10; k++) { const ci = el('i', k * 10 <= n ? 'f' : '', cells); ci.addEventListener('click', (ev) => { ev.stopPropagation(); S.crow = idx; setting(r.key, k * 10 === n ? k * 10 - 10 : k * 10); }); }
      bt(v, String(n) + '%', { col: C.dim });
    } else if (r.kind === 'act') { const o = el('span', 'opt pick', v); bt(o, r.value || ' ', { col: C.dim }).classList.add('d'); bt2(o, r.value || ' '); }
    else if (r.kind === 'info') bt(v, r.value || ' ', { col: C.dim });
    d.addEventListener('mouseenter', () => { if (S.crow !== idx) { S.crow = idx; S.focus = 'config'; cmark(); sound('snd_menumove', 0.2); } });
    d.addEventListener('click', () => { S.crow = idx; S.focus = 'config'; cmark(); cact(0); });
    crow.push({ ...r, node: d });
  });
  S.crow = Math.max(0, Math.min(S.crow, crow.length - 1));
  rows.scrollTop = keep;
  cmark();
}
function cmark() {
  crow.forEach((r, i) => r.node.classList.toggle('on', i === S.crow && S.focus === 'config'));
  const r = crow[S.crow], h = $('c-help'); h.textContent = '';
  if (r && r.help) bt(h, r.help, { col: C.dim, wrap: true });
  if (r) r.node.scrollIntoView({ block: 'nearest' });
}
function cact(d) {
  const r = crow[S.crow]; if (!r) return;
  if (r.kind === 'opt') {
    const i = r.opts.findIndex(([v]) => v === r.cur), n = r.opts.length;
    const j = d === 0 ? (i + 1) % n : Math.max(0, Math.min(n - 1, i + d));
    if (j !== i) { const v = r.opts[j][0]; setting(r.key, v).then(() => r.after && r.after(v)); }
  } else if (r.kind === 'vol') { const n = S.st.settings[r.key], v = Math.max(0, Math.min(100, n + (d || 1) * 10)); if (v !== n) setting(r.key, v); }
  else if (r.kind === 'act' && d === 0) { sound('snd_select'); flash(r.node); r.run(); }
}

function move(d) {
  if (S.splash) return;
  if (S.focus === 'home') { const i = items.findIndex((r) => r.id === S.selId); S.selId = items[(i + d + items.length) % items.length].id; mark(); }
  else if (S.focus === 'notes') { const n = updates().length; if (!n) return; S.nsel = (S.nsel + d + n) % n; renderNotes(); }
  else if (S.focus === 'config') { S.crow = (S.crow + d + crow.length) % crow.length; cmark(); }
  sound('snd_menumove', 0.25);
}
function side(dx) {
  if (S.focus === 'config') return cact(dx);
  if (S.focus === 'home') return move(dx);
}
function ok() {
  if (S.splash) return;
  if (S.focus === 'home') { const r = items.find((x) => x.id === S.selId); return r && itemAct(r.id, r.node); }
  if (S.focus === 'config') return cact(0);
}
function back() { if (!S.splash && S.view !== 'home') { openView('home'); sound('snd_menumove'); } }
function tab(d) { if (S.splash) return; if (S.view !== 'config') openView('config'); else { S.csec += d; S.crow = 0; S.focus = 'config'; renderConfig(); } sound('snd_menumove', 0.3); }
addEventListener('keydown', (e) => {
  const k = e.key;
  if (k === 'ArrowUp' || k === 'ArrowLeft') (S.focus === 'config' && k === 'ArrowLeft' ? side(-1) : move(-1));
  else if (k === 'ArrowDown' || k === 'ArrowRight') (S.focus === 'config' && k === 'ArrowRight' ? side(1) : move(1));
  else if (k === 'z' || k === 'Z' || k === 'Enter' || k === ' ') { if (e.repeat) return; ok(); }
  else if (k === 'x' || k === 'X' || k === 'Escape' || k === 'Backspace') back();
  else if (k === 'q' || k === 'Q') tab(-1); else if (k === 'e' || k === 'E') tab(1);
  else return;
  e.preventDefault();
});
addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('wheel', (e) => { const t = e.target.closest('#n-list, #n-detail, #c-rows'); if (!t) e.preventDefault(); }, { passive: false });
// only read the gamepad while this window is focused
let winFocused = false;
const pad = { held: {}, next: {} };
function pollPad(t) {
  const gp = (navigator.getGamepads ? [...navigator.getGamepads()] : []).find((g) => g && g.connected);
  if (gp && winFocused && document.hasFocus()) {
    const ax = gp.axes || [], b = (i) => !!(gp.buttons[i] && gp.buttons[i].pressed);
    const st = { up: b(12) || ax[1] < -0.5, down: b(13) || ax[1] > 0.5, left: b(14) || ax[0] < -0.5, right: b(15) || ax[0] > 0.5, a: b(0), b: b(1), lb: b(4), rb: b(5), start: b(9) };
    for (const [k, on] of Object.entries(st)) {
      const was = pad.held[k], dir = k === 'up' || k === 'down' || k === 'left' || k === 'right';
      if (on && (!was || (dir && t >= pad.next[k]))) {
        pad.next[k] = t + (was ? 110 : 340);
        if (k === 'up') move(-1); else if (k === 'down') move(1);
        else if (k === 'left') (S.focus === 'config' ? side(-1) : move(-1)); else if (k === 'right') (S.focus === 'config' ? side(1) : move(1));
        else if (k === 'a') ok(); else if (k === 'b') back(); else if (k === 'lb') tab(-1); else if (k === 'rb') tab(1);
        else if (k === 'start' && !S.splash) act(primary().id);
      }
      pad.held[k] = on;
    }
  }
  requestAnimationFrame(pollPad);
}

function endSplash() { S.splash = false; S.starting = false; document.body.classList.remove('splash', 'out'); renderAll(); }
async function boot() {
  await loadKit();
  bt($('sub'), 'FIGHT SIMULATOR', { font: 'fnt_mainbig', s: 2 });
  bt2($('l-notes'), 'PATCH NOTES'); bt2($('l-config'), 'SETTINGS'); bt2($('p-close'), 'CLOSE');
  $('w-min').addEventListener('click', () => win.minimize());
  $('w-close').addEventListener('click', () => win.close());
  for (const [id, node] of [['notes', $('l-notes')], ['config', $('l-config')], ['discord', $('w-discord')], ['main', $('play')]]) {
    node.addEventListener('click', () => { S.focus = 'home'; S.selId = id; itemAct(id, node); });
    node.addEventListener('mouseenter', () => { if (S.focus === 'home' && S.selId !== id) { S.selId = id; mark(); sound('snd_menumove', 0.25); } });
  }
  $('extras').addEventListener('click', (e) => { const n = e.target.closest('.lk'); if (n) { S.selId = n.dataset.id; itemAct(n.dataset.id, n); } });
  $('p-close').addEventListener('click', () => { sound('snd_menumove'); openView('home'); });
  S.st = await invoke('launcher_state');
  renderAll();
  T.event.listen('sync-progress', (ev) => { S.prog = ev.payload; renderStatus(); });
  T.event.listen('launcher-progress', (ev) => { const [done, total] = ev.payload || [0, 0]; S.prog = { done, total }; renderStatus(); });
  T.event.listen('game-closed', async () => { S.st = await invoke('launcher_state'); renderAll(); if (S.st.settings.auto_check && !S.st.settings.offline) check(); else maybePrewarm(1500); });
  addEventListener('resize', () => kit.paintAll(true));
  try { winFocused = await win.isFocused(); win.onFocusChanged((e) => { winFocused = !!e.payload; pad.held = {}; }); } catch (e) { }
  requestAnimationFrame(pollPad);
  refreshNotes(false);

  const s = S.st.settings, hidden = S.st.start_hidden, t0 = performance.now();
  if (!hidden) await win.show();
  if (!hidden && S.st.first_run) { try { await win.setAlwaysOnTop(true); await win.setFocus(); await win.setAlwaysOnTop(false); } catch (e) { } }
  if (s.offline) S.chk = { online: false, error: 'offline-mode', need_files: 0 };
  else if (s.auto_check || !S.st.installed) {
    await Promise.race([check(), wait(S.st.installed ? 1500 : 6000)]);
    const c = S.chk;
    if (!hidden && S.st.installed && !S.checking && c && c.online && !c.error && c.need_files > 0) await download();
  }
  await wait(Math.max(0, 600 - (performance.now() - t0)));
  if (!hidden && s.start_in_game && S.st.installed && !S.busy && !S.err) {
    S.starting = true; renderStatus();
    document.body.classList.add('out');
    await wait(350);
    await play();
    await wait(400);
    endSplash();
    return;
  }
  endSplash();
  if (!S.checking) maybePrewarm();
}
boot().catch((e) => {
  try { const p = document.createElement('p'); p.style.cssText = 'position:fixed;left:28px;bottom:120px;color:#ff4040;z-index:9;font:16px monospace'; p.textContent = 'The launcher did not start: ' + e; document.body.appendChild(p); } catch (x) { }
  document.body.classList.remove('splash');
  win.show();
});
