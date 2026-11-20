// SPDX-License-Identifier: FSL-1.1-ALv2
// HumanNeeded tray panel (D-059 app-first, WS-DESIGN rounds 14–16). Talks only to the app (Tauri
// commands). The app owns the alert settings: per state, the computer's screen edges and sound,
// and the puck's light and sound (pushed to the puck when it connects).
//
// Views: welcome (first run) · main (Connect your agents, Cloud agents, state cards, Puck, Settings) · state (Computer /
// Puck tabs) · Settings · Puck. Everything is drawn from two things: `st` (the app's status) and `A` (alert settings).
'use strict';
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const listen = (ev, fn) => window.__TAURI__.event.listen(ev, fn);

const STATES = [
  { id: 'working', name: 'Working' },
  { id: 'needs', name: 'Needs you' },
  { id: 'done', name: 'Done' },
  { id: 'idle', name: 'Idle' },
];
const STATE_NAME = Object.fromEntries(STATES.map((s) => [s.id, s.name]));
const LIGHTS = [['solid', 'Solid'], ['breathe', 'Breathe'], ['blink', 'Blink'], ['heartbeat', 'Heartbeat'], ['flicker', 'Candle flicker'], ['spin', 'Spin']];
const SOUNDS = [['ding', 'Ding'], ['chime', 'Chime'], ['rising', 'Rising'], ['arpeggio', 'Arpeggio'], ['bell', 'Bell'], ['beeps', 'Beeps'], ['bloop', 'Bloop'],
  ['bright', 'Bright (melody)'], ['clock', 'Clock (melody)'], ['doorbell', 'Doorbell (melody)'], ['fanfare', 'Fanfare (melody)'], ['ripple', 'Ripple (melody)'],
  ['call', 'Call (melody)'], ['lullaby', 'Lullaby (melody)'], ['harp', 'Harp (melody)']];
const DURATIONS = [[1, '1 second'], [2, '2 seconds'], [3, '3 seconds'], [5, '5 seconds'], [10, '10 seconds'], [30, '30 seconds'], [60, '1 minute'], [300, '5 minutes'], [600, '10 minutes']];
const UNTIL = { needs: 'Until you answer', working: 'While it works', idle: 'Always', done: '' };
const SWATCHES = ['#ffa62b', '#ff4d4d', '#ff5fb0', '#a26bff', '#3366ff', '#2ec4d6', '#22e04a', '#ffd2a0', '#ffffff'];
// The agents listed, in order. Cowork is the cloud link (claude-cloud).
const AGENTS = [
  { id: 'claude', name: 'Claude Code', logo: 'claude' },
  { id: 'codex', name: 'Codex', logo: 'openai' },
  { id: 'claude-cloud', name: 'Cowork', logo: 'claude' },
  { id: 'cursor', name: 'Cursor', logo: 'cursor' },
];
const LOGO = {
  claude: '<img src="logos/claude-color.svg" alt="">',
  openai: '<span class="mk" style="--u:url(logos/openai.svg)"></span>',
  cursor: '<span class="mk" style="--u:url(logos/cursor.svg)"></span>',
};
const CLOUD_TEXT = {
  no_account: 'Sign in to Claude on this computer first',
  connecting: 'Connecting…',
  connected: 'Checking it works…',
  key_full: 'Three other computers use this account',
  error: 'The relay refused this computer',
};

let st = null;     // app status
let A = null;      // alert settings
let D = null;      // default alert settings per state
let view = 'v-home';
let cur = null;    // state being edited
let tab = 'c';     // 'c' computer | 'p' puck

function esc(v) {
  return String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
let tm;
function toast(t) {
  $('toast').textContent = t;
  $('toast').classList.add('show');
  clearTimeout(tm);
  tm = setTimeout(() => $('toast').classList.remove('show'), 2400);
}
function fail(e) { toast(String(e)); }
const label = (list, v) => (list.find(([k]) => String(k) === String(v)) || [v, String(v)])[1];
function ago(ms) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 45) return 'now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} days ago`;
}
// Replaces an element's content only when the markup changed. A redraw replaces the
// controls inside, which closes an open dropdown and drops hover and focus, so status updates
// (every few seconds) must not redraw what hasn't changed.
const drawn = new WeakMap();
function put(el, html) {
  if (drawn.get(el) === html) return false;
  drawn.set(el, html);
  el.innerHTML = html;
  return true;
}
const sw = (attrs, on, dis) => `<button type="button" class="sw" role="switch" aria-checked="${!!on}" ${attrs}${dis ? ' disabled' : ''}></button>`;
const puckOn = () => !!(st && st.puck && st.puck.connected);

// ---- the window follows its content ----
function fit() {
  requestAnimationFrame(() => {
    const h = Math.ceil($('panel').getBoundingClientRect().height);
    if (h > 0) invoke('panel_fit', { height: h }).catch(() => {});
  });
}
function go(id, back) {
  document.querySelectorAll('.view').forEach((v) => v.classList.remove('on', 'back'));
  $(id).classList.add('on');
  if (back) $(id).classList.add('back');
  view = id;
  render();
  fit();
}

// ---- alert settings ----
async function loadAlerts() {
  try {
    const r = await invoke('alerts_get');
    A = r.alerts;
    D = r.defaults;
  } catch (e) { /* the app isn't ready yet */ }
}
let saveTimer;
function save(now) {
  clearTimeout(saveTimer);
  const send = () => invoke('alerts_set', { alerts: A }).catch((e) => { fail(e); loadAlerts().then(render); });
  if (now) send(); else saveTimer = setTimeout(send, 150);
}

// ---- main view ----
function problem() {
  const h = st.http || {};
  if (h.error) {
    return { title: "Agents can't reach the app", text: `Another program is using port ${h.port || 24816} (${h.error}). Close it or restart your computer; HumanNeeded keeps trying.` };
  }
  if (st.proxy && st.proxy.problem) {
    return { title: 'A proxy blocks your agents', text: st.proxy.problem, code: st.proxy.fix, btn: 'Copy' };
  }
  return null;
}
// Cloud agents (Cowork and other cloud agents, through the relay): one switch on the main view.
function renderCloud() {
  const c = st.cloud || {};
  const on = !!c.enabled;
  const b = $('cloud-sw');
  if (!b.disabled) b.setAttribute('aria-checked', on);
  // Only a problem is worth a line here (the design has none otherwise).
  const msg = on && c.state && c.state !== 'working' && c.state !== 'connected' ? (c.error || CLOUD_TEXT[c.state] || '') : '';
  $('cloud-note').hidden = !msg;
  $('cloud-note').textContent = msg;
}
// The same rule on every card: what this computer does, else the Puck, else Off. No durations.
function tileText(id) {
  const a = A.states[id];
  const parts = [];
  if (a.screen.on) parts.push('Edges');
  if (a.sound && a.sound.on) parts.push('Sound');
  if (parts.length) return parts.join(' · ');
  return a.puck.on || (a.puck_sound && a.puck_sound.on) ? 'Puck only' : 'Off';
}
function renderHome() {
  const pr = problem();
  $('fix').hidden = !pr;
  if (pr) {
    $('fix-title').textContent = pr.title;
    $('fix-text').textContent = pr.text;
    $('fix-code').hidden = !pr.code;
    $('fix-code').textContent = pr.code || '';
    $('fix-btn').hidden = !pr.btn;
    $('fix-btn').textContent = pr.btn || '';
  }
  renderCloud();
  // A downloaded update: one click installs it and restarts (as Settings' "Restart to update").
  const ready = st.update && st.update.ready;
  $('upd').hidden = !ready;
  $('upd-v').textContent = ready || '';
  const m = (st.output && st.output.status && st.output.status.muted) || 'off';
  const q = st.quiet || {};
  let note = '';
  if (q.active) note = `Quiet hours until ${q.to}: no sounds, softer light.`;
  else if (m === 'on') note = 'Sounds muted.';
  else if (m !== 'off') note = `Sounds muted for ${m} more min.`;
  $('mute-note').hidden = !note;
  $('mute-note').textContent = note;
  if (A) {
    put($('tiles'), STATES.map((s) => {
      const a = A.states[s.id];
      const off = tileText(s.id) === 'Off';
      const c = a.screen.on || !a.puck.on ? a.screen.color : a.puck.color;
      return `<button type="button" class="tile${off ? ' off' : ''}" data-state="${s.id}" style="--c:${esc(c)}">
        <span class="lamp"></span><b>${esc(s.name)}</b><small>${esc(tileText(s.id))}</small></button>`;
    }).join(''));
  }
  const p = st.puck || {};
  const old = p.connected && !p.compatible;
  $('puck-ic').style.color = p.connected ? 'var(--hn-text)' : 'var(--hn-faint)';
  $('puck-s').style.setProperty('--c', old ? 'var(--hn-problem)' : p.connected ? 'var(--hn-ok)' : 'var(--hn-faint)');
  $('puck-s').lastElementChild.textContent = old ? 'Update needed' : p.connected ? 'Connected' : 'Not connected';
}

// ---- state page ----
function lookBody(key, look, id) {
  const durs = (id === 'done' ? [] : [[0, UNTIL[id]]]).concat(DURATIONS);
  const color = (look.color || '').toLowerCase();
  return `<div class="fld"><span class="lab">Light</span><select class="sel" data-k="${key}.light">${LIGHTS.map(([k, l]) => `<option value="${k}"${k === look.light ? ' selected' : ''}>${esc(l)}</option>`).join('')}</select></div>
    <div class="fld"><span class="lab">Colour</span><div class="swatches" data-color="${key}">${SWATCHES.map((c) => `<button type="button" style="--c:${c}" data-c="${c}" aria-label="Colour ${c}" aria-pressed="${c === color}"></button>`).join('')}<button type="button" class="custom" aria-label="Pick any colour" aria-pressed="${!SWATCHES.includes(color)}"></button><input type="color" value="${esc(color || '#ffffff')}" tabindex="-1"></div></div>
    <div class="fld"><span class="lab">Brightness</span><div class="lvrow compact"><input type="range" min="1" max="100" value="${look.brightness}" data-k="${key}.brightness" aria-label="Brightness"><span class="v"></span></div></div>
    <div class="fld"><span class="lab">Duration</span><select class="sel" data-k="${key}.duration" data-num>${durs.map(([k, l]) => `<option value="${k}"${Number(k) === look.duration ? ' selected' : ''}>${esc(l)}</option>`).join('')}</select></div>`;
}
function soundBody(key, s) {
  return `<div class="fld"><span class="lab">Sound</span><select class="sel" data-k="${key}.sound">${SOUNDS.map(([k, l]) => `<option value="${k}"${k === s.sound ? ' selected' : ''}>${esc(l)}</option>`).join('')}</select></div>
    <div class="fld"><span class="lab">Volume</span><div class="lvrow compact"><input type="range" min="0" max="100" value="${s.volume}" data-k="${key}.volume" aria-label="Volume"><span class="v"></span></div></div>`;
}
function section(title, key, on, body, locked) {
  return `<div class="ssec${on ? '' : ' off'}${locked ? ' locked' : ''}"><div class="row"><span class="gr"><b>${title}</b></span>${sw(`data-on="${key}" aria-label="${esc(title)}"`, on, locked)}</div><div class="body">${body}</div></div>`;
}
function renderState() {
  if (!cur || !A) return;
  const id = cur;
  const a = A.states[id];
  $('st-title').textContent = STATE_NAME[id];
  $('st-dot').style.setProperty('--c', a.screen.color);
  const pk = puckOn();
  if (!pk && tab === 'p') tab = 'c';
  $('tab-c').setAttribute('aria-selected', tab === 'c');
  $('tab-p').setAttribute('aria-selected', tab === 'p');
  $('tab-p').disabled = !pk;
  $('tab-p').textContent = pk ? 'Puck' : 'Puck · not connected';
  let html;
  if (tab === 'c') {
    html = section('Screen edges', 'screen', a.screen.on, lookBody('screen', a.screen, id));
    if (a.sound) html += section('Sound', 'sound', a.sound.on, soundBody('sound', a.sound));
  } else {
    html = section('Light', 'puck', a.puck.on, lookBody('puck', a.puck, id));
    if (a.puck_sound) html += section('Sound', 'puck_sound', a.puck_sound.on, soundBody('puck_sound', a.puck_sound));
  }
  if (put($('st-body'), html)) {
    $('st-body').querySelectorAll('input[type=range]').forEach(paintRange);
    fit();
  }
}
function paintRange(r) {
  const p = ((r.value - r.min) / (r.max - r.min)) * 100;
  r.style.setProperty('--p', `${p}%`);
  const v = r.parentNode.querySelector('.v');
  if (v) v.textContent = `${Math.round(r.value)}%`;
}
function setField(path, value) {
  const [part, field] = path.split('.');
  A.states[cur][part][field] = value;
}
$('st-body').addEventListener('click', (e) => {
  const s = e.target.closest('[data-on]');
  if (s && !s.disabled) {
    const part = s.dataset.on;
    A.states[cur][part].on = s.getAttribute('aria-checked') !== 'true';
    save(true);
    renderState();
    return;
  }
  const sw2 = e.target.closest('[data-color]');
  const b = e.target.closest('button');
  if (sw2 && b) {
    if (b.classList.contains('custom')) { sw2.querySelector('input[type=color]').click(); return; }
    setField(`${sw2.dataset.color}.color`, b.dataset.c);
    save(true);
    renderState();
  }
});
$('st-body').addEventListener('change', (e) => {
  const t = e.target;
  if (t.type === 'color') {
    setField(`${t.closest('[data-color]').dataset.color}.color`, t.value.toLowerCase());
    save(true);
    renderState();
  } else if (t.dataset.k && t.tagName === 'SELECT') {
    setField(t.dataset.k, 'num' in t.dataset ? Number(t.value) : t.value);
    save(true);
  }
});
$('st-body').addEventListener('input', (e) => {
  const t = e.target;
  if (t.type === 'range' && t.dataset.k) {
    paintRange(t);
    setField(t.dataset.k, Number(t.value));
    save(false);
  }
});
$('tab-c').onclick = () => { tab = 'c'; renderState(); };
$('tab-p').onclick = () => { if (puckOn()) { tab = 'p'; renderState(); } };
$('st-prev').onclick = async () => {
  try { await invoke('puck_action', { action: 'test', arg: cur }); toast(`Previewing "${STATE_NAME[cur]}"`); } catch (e) { fail(e); }
};
$('st-reset').onclick = () => {
  if (!D || !D[cur]) return;
  A.states[cur] = JSON.parse(JSON.stringify(D[cur]));
  save(true);
  renderState();
  toast('Back to default');
};

// ---- settings ----
function renderSettings() {
  const g = st.glow || {};
  $('glow-paused').hidden = g.enabled !== false;
  $('g-screens').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', b.dataset.v === (g.screens || 'all')));
  $('g-thick').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', b.dataset.v === (g.thickness || 'mid')));
  $('g-full').setAttribute('aria-checked', g.skip_fullscreen !== false);
  const q = st.quiet || {};
  $('q-on').setAttribute('aria-checked', !!q.enabled);
  if (document.activeElement !== $('q-from') && q.from) $('q-from').value = q.from;
  if (document.activeElement !== $('q-to') && q.to) $('q-to').value = q.to;
  const u = st.update || {};
  $('auto-update').setAttribute('aria-checked', !!u.auto);
  let ver = st.app.version;
  if (u.checking) ver += ' · checking…';
  else if (u.last) ver += ` · ${u.last}`;
  $('app-ver').textContent = ver;
  $('app-restart').hidden = !u.ready;
  $('app-restart').textContent = u.ready ? `Restart to update to ${u.ready}` : '';
  put($('kv'), [['Agent port', (st.http && st.http.port) || ''], ['Version', st.app.version]]
    .map(([k, v]) => `<span>${esc(k)}</span><span class="mono">${esc(v)}</span>`).join(''));
  if (!$('danger').innerHTML) resetButton();
}
async function setGlow(patch) {
  const options = { ...(st.glow || {}), ...patch };
  try { await invoke('glow_set', { options }); } catch (e) { fail(e); }
  refresh();
}
$('g-screens').addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) setGlow({ screens: b.dataset.v }); });
$('g-thick').addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) setGlow({ thickness: b.dataset.v }); });
$('g-full').onclick = () => setGlow({ skip_fullscreen: $('g-full').getAttribute('aria-checked') !== 'true' });
$('glow-resume').onclick = () => setGlow({ enabled: true });
async function saveQuiet() {
  const enabled = $('q-on').getAttribute('aria-checked') === 'true';
  try { await invoke('quiet_set', { enabled, from: $('q-from').value || '22:00', to: $('q-to').value || '08:00' }); } catch (e) { fail(e); }
  refresh();
}
$('q-on').onclick = () => {
  $('q-on').setAttribute('aria-checked', $('q-on').getAttribute('aria-checked') !== 'true');
  saveQuiet();
};
$('q-from').onchange = saveQuiet;
$('q-to').onchange = saveQuiet;
$('autostart').onclick = async () => {
  const on = $('autostart').getAttribute('aria-checked') !== 'true';
  try { await invoke('autostart_set', { on }); $('autostart').setAttribute('aria-checked', on); } catch (e) { fail(e); }
};
$('auto-update').onclick = async () => {
  const on = $('auto-update').getAttribute('aria-checked') !== 'true';
  try { await invoke('update_auto', { on }); } catch (e) { fail(e); }
  refresh();
};
$('app-restart').onclick = (e) => { e.target.disabled = true; invoke('update_install').catch(fail); };
$('upd-go').onclick = (e) => {
  e.target.disabled = true;
  e.target.textContent = 'Updating…';
  invoke('update_install').catch((err) => { e.target.disabled = false; e.target.textContent = 'Update now'; fail(err); });
};
$('clear').onclick = async () => { await invoke('clear_sessions'); toast('All alerts cleared'); refresh(); };
$('log').onclick = () => invoke('open_log').catch(fail);
// For a bug report: versions, agents, agent port, puck, last log lines (no keys or ids).
$('diag').onclick = async () => {
  try {
    const text = await invoke('diagnostics');
    try { await navigator.clipboard.writeText(text); } catch (_) {
      const t = document.createElement('textarea');
      t.value = text; document.body.appendChild(t); t.select();
      const ok = document.execCommand('copy'); t.remove();
      if (!ok) throw new Error("couldn't copy");
    }
    toast('Diagnostics copied. Paste them into your bug report');
  } catch (e) { fail(e); }
};
$('adv').addEventListener('toggle', fit);
function resetButton() {
  $('danger').innerHTML = '<div class="btns"><button type="button" class="btn red" data-reset>Reset lights and sounds</button><button type="button" class="btn red" data-uninstall>Uninstall HumanNeeded…</button></div>';
  fit();
}
$('danger').addEventListener('click', async (e) => {
  if (e.target.closest('[data-reset]')) {
    $('danger').innerHTML = '<p class="muted" style="margin:0 0 8px;font-size:12px">Put every state\'s lights and sounds back to their defaults, on this computer and the puck?</p><div class="btns"><button type="button" class="btn red" data-yes>Reset</button><button type="button" class="btn" data-no>Cancel</button></div>';
    fit();
  } else if (e.target.closest('[data-yes]')) {
    A.states = JSON.parse(JSON.stringify(D));
    save(true);
    toast('Lights and sounds reset');
    resetButton();
    render();
  } else if (e.target.closest('[data-uninstall]')) {
    // R-05: removes the login item, the cloud link's credential, logs and caches; settings
    // only if the user doesn't keep them. Windows: the uninstaller takes over from here.
    $('danger').innerHTML = '<p class="muted" style="margin:0 0 8px;font-size:12px">Remove HumanNeeded from this computer? Your agents\' plugins and hooks stay; remove them in each agent.</p><div class="row"><span class="gr">Keep my settings<small>For when you install it again</small></span><button type="button" class="sw" role="switch" aria-checked="true" id="un-keep" aria-label="Keep my settings"></button></div><div class="btns"><button type="button" class="btn red" data-un-yes>Uninstall</button><button type="button" class="btn" data-no>Cancel</button></div>';
    fit();
  } else if (e.target.closest('#un-keep')) {
    const k = $('un-keep');
    k.setAttribute('aria-checked', k.getAttribute('aria-checked') !== 'true');
  } else if (e.target.closest('[data-un-yes]')) {
    const keepSettings = $('un-keep').getAttribute('aria-checked') === 'true';
    e.target.disabled = true;
    try { await invoke('uninstall', { keepSettings }); } catch (err) { fail(err); resetButton(); }
  } else if (e.target.closest('[data-no]')) resetButton();
});

// ---- puck page ----
let fwNote = '';
let fwBusy = false;
function renderPuck() {
  const p = st.puck || {};
  $('puck-on').hidden = !p.connected;
  $('puck-off').hidden = !!p.connected;
  if (!p.connected) return;
  const shown = String(p.shown || 'idle').replace(/^test:/, '');
  const a = A && A.states[shown];
  $('dev-ring').style.color = a && a.puck.on ? a.puck.color : 'var(--hn-faint)';
  const old = !p.compatible;
  $('dev-st').style.setProperty('--c', old ? 'var(--hn-problem)' : 'var(--hn-ok)');
  $('dev-text').textContent = old ? 'Firmware too old for this app' : 'Connected';
  $('puck-test').disabled = old;
  if (!fwBusy) {
    $('fw-s').textContent = fwNote ? `${p.fw} · ${fwNote}` : p.fw || '';
    $('fw-check').hidden = false;
    if (old) { $('fw-go').hidden = false; $('fw-go').textContent = 'Update firmware'; }
  }
  put($('pkv'), [['Port', p.port], ['Board', p.board], ['Serial', p.serial]]
    .filter(([, v]) => v).map(([k, v]) => `<span>${esc(k)}</span><span class="mono">${esc(v)}</span>`).join(''));
}
$('puck-test').onclick = async () => {
  try { await invoke('puck_action', { action: 'puck_test' }); toast('Testing the puck'); } catch (e) { fail(e); }
};
$('fw-check').onclick = async () => {
  fwBusy = true;
  $('fw-s').textContent = `${st.puck.fw} · checking…`;
  $('fw-go').hidden = true;
  try {
    const a = await invoke('fw_check');
    if (!a.latest) fwNote = 'no firmware published yet';
    else if (a.newer) {
      fwNote = `${a.latest.version} is available`;
      $('fw-go').hidden = false;
      $('fw-go').textContent = `Update to ${a.latest.version}`;
    } else fwNote = 'up to date';
  } catch (e) { fwNote = `couldn't check: ${e}`; }
  fwBusy = false;
  renderPuck();
  fit();
};
$('fw-go').onclick = async () => {
  fwBusy = true;
  $('fw-go').disabled = true;
  $('fw-check').disabled = true;
  $('fw-s').textContent = 'Updating firmware, about a minute. Settings are kept.';
  try {
    const v = await invoke('fw_update');
    fwNote = '';
    $('fw-go').hidden = true;
    toast(`Updated to ${v}`);
  } catch (e) { fwNote = `update failed: ${e}`; }
  fwBusy = false;
  $('fw-go').disabled = false;
  $('fw-check').disabled = false;
  refresh();
};
listen('fw-progress', (ev) => { if (fwBusy) $('fw-s').textContent = ev.payload; });
$('restart').onclick = async () => { try { await invoke('puck_action', { action: 'reboot' }); toast('Restarting the puck'); } catch (e) { fail(e); } };
$('pclear').onclick = async () => { await invoke('clear_sessions'); toast('Light cleared'); refresh(); };
$('padv').addEventListener('toggle', fit);
$('see-puck').onclick = () => invoke('open_puck_page').catch(fail);

// ---- main view actions ----
$('cloud-sw').onclick = async () => {
  const b = $('cloud-sw');
  if (b.disabled) return;
  const on = b.getAttribute('aria-checked') !== 'true';
  b.disabled = true;
  b.setAttribute('aria-checked', on);
  try { await invoke('cloud_set', { on }); } catch (err) { fail(err); }
  b.disabled = false;
  refresh();
};
$('fix-btn').onclick = async () => {
  const pr = problem();
  if (pr && pr.code) {
    try { await navigator.clipboard.writeText(pr.code); toast('Copied'); } catch (e) { fail(e); }
  }
};

// ---- welcome (R-01, first run) ----
// The first run opens here until Done: a demo of the glow and chime, each agent (found on this
// computer or not, connected once it sends a signal) and start at sign-in.
const home = () => (st && st.welcome ? 'v-welcome' : 'v-home');
function welcomeRow(a) {
  const s = (st.agents || []).find((x) => x.id === a.id) || {};
  let line; let right = '<button type="button" class="btn pri setup" data-guide>Set up</button>';
  let off = false;
  if (s.state === 'connected') {
    line = '<i style="--c:var(--hn-ok)"></i>Connected';
    right = '';
  } else if (s.found) line = 'Found on this computer';
  else { line = 'Not found on this computer'; off = true; right = '<button type="button" class="btn setup" data-guide>Set up</button>'; }
  return `<div class="agrow${off ? ' off' : ''}"><span class="agentmark">${LOGO[a.logo]}</span><span class="gr">${esc(a.name)}<small>${line}</small></span>${right}</div>`;
}
function renderWelcome() {
  put($('w-agents'), AGENTS.filter((a) => a.id !== 'claude-cloud').map(welcomeRow).join(''));
}
let demo = null;
$('w-demo').onclick = async () => {
  // "Needs you", then "Done", as a real task would: the same previews as the state pages.
  clearTimeout(demo);
  try {
    await invoke('puck_action', { action: 'test', arg: 'needs' });
    demo = setTimeout(() => invoke('puck_action', { action: 'test', arg: 'done' }).catch(fail), 3000);
    $('w-demo').lastChild.textContent = 'Show again';
  } catch (e) { fail(e); }
};
$('w-auto').onclick = async () => {
  const on = $('w-auto').getAttribute('aria-checked') !== 'true';
  try {
    await invoke('autostart_set', { on });
    $('w-auto').setAttribute('aria-checked', on);
    $('autostart').setAttribute('aria-checked', on);
  } catch (e) { fail(e); }
};
$('w-done').onclick = async () => {
  try { await invoke('welcome_done'); } catch (e) { fail(e); }
  if (st) st.welcome = false;
  go('v-home');
};

// ---- navigation ----
document.addEventListener('click', (e) => {
  const t = e.target;
  const g = t.closest('[data-go]');
  if (g) { go(g.dataset.go); return; }
  if (t.closest('[data-back]')) { go('v-home', true); return; }
  if (t.closest('[data-guide]')) { invoke('open_setup_guide').catch(fail); return; }
  const tile = t.closest('[data-state]');
  if (tile) {
    cur = tile.dataset.state;
    tab = 'c';
    go('v-state');
  }
});
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (view === 'v-home' || view === 'v-welcome') invoke('panel_hide');
  else go('v-home', true);
});

// ---- status ----
function render() {
  if (!st) return;
  if (view === 'v-home') renderHome();
  else if (view === 'v-welcome') renderWelcome();
  else if (view === 'v-state') renderState();
  else if (view === 'v-settings') renderSettings();
  else if (view === 'v-puck') renderPuck();
  fit();
}
let wasPuck = null;
async function refresh() {
  try { st = await invoke('status'); } catch (e) { return; }
  // A puck plugged in or out: the Puck tab and page change.
  if (puckOn() !== wasPuck) {
    wasPuck = puckOn();
    if (view === 'v-state' && !wasPuck) tab = 'c';
  }
  // Never redraw under a control in use: an open dropdown, a slider being dragged.
  const f = document.activeElement;
  if (f && (f.tagName === 'SELECT' || f.type === 'range')) return;
  render();
}

listen('changed', () => refresh());
listen('panel-shown', async () => {
  // Opening the panel starts from the main view, with fresh values.
  await loadAlerts();
  await refresh();
  go(home());
});
invoke('autostart_get').then((on) => {
  $('autostart').setAttribute('aria-checked', !!on);
  $('w-auto').setAttribute('aria-checked', !!on);
}).catch(() => {});
loadAlerts().then(refresh).then(() => go(home()));
setInterval(() => { if (!document.hidden) refresh(); }, 5000);
