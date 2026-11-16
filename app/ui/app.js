// HumanNeeded tray panel (D-055): a remote control for the puck. Talks only to the app (Tauri
// commands); the app owns the puck. Lists (lights, sounds, durations) come from the puck's SCHEMA.
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
// Firmware 2.0 defaults (settingsDefaults), for a state's "Default" button.
const DEFAULTS = {
  needs: { color: '#ffa62b', light: 'pulse', sound: 'call', duration: '0' },
  done: { color: '#22e04a', light: 'pulse', sound: 'bright', duration: '3' },
  working: { color: '#3366ff', light: 'spin', sound: 'none', duration: '0' },
  idle: { color: '#ffd2a0', light: 'solid' },
};
const SWATCHES = ['#ff8a1f', '#ffcf3a', '#ff4d4d', '#ff5fb0', '#a26bff', '#4a78ff', '#2ec4d6', '#26c25a', '#ffffff'];
// Every animation the puck's SCHEMA offers is shown (off = the page's switch). Known ones first,
// in this order; any new one from a later firmware is added after them with a plain dot.
const LIGHT_ORDER = ['solid', 'breathe', 'pulse', 'spin', 'blink', 'heartbeat', 'flicker'];
const LIGHT_SHORT = { breathe: 'Breathe', flicker: 'Flicker', spin: 'Spin' };
const FX_ICON = {
  solid: '<path d="M1 6h32" stroke="currentColor" stroke-width="2"/>',
  breathe: '<path d="M1 9c4 0 4-6 8-6s4 6 8 6 4-6 8-6 4 6 8 6" fill="none" stroke="currentColor" stroke-width="2"/>',
  pulse: '<path d="M1 10h4l2-8 2 8h6l2-8 2 8h6l2-8 2 8h3" fill="none" stroke="currentColor" stroke-width="2"/>',
  spin: '<circle cx="17" cy="6" r="4.5" fill="none" stroke="currentColor" stroke-width="2" stroke-dasharray="7 3.5"/>',
  blink: '<path d="M1 10h5V2h6v8h5V2h6v8h5V2h5" fill="none" stroke="currentColor" stroke-width="2"/>',
  heartbeat: '<path d="M1 8h7l2-6 2 6 2-3 2 3h18" fill="none" stroke="currentColor" stroke-width="2"/>',
  flicker: '<path d="M1 7l3-3 2 5 3-6 2 4 3-2 2 5 3-6 2 3 3-1 2 4 3-3 2 2" fill="none" stroke="currentColor" stroke-width="1.6"/>',
};
const AGENT_MARK = { claude: 'Cl', codex: 'Cx', cursor: 'Cu' };
const AGENT_NAME = { claude: 'Claude Code', codex: 'Codex', cursor: 'Cursor' };
const CLOUD_TEXT = {
  off: 'Through the HumanNeeded relay',
  no_account: 'Sign in to Claude on this computer first',
  connecting: 'Connecting to the relay…',
  connected: 'Connected; checking messages come through…',
  working: 'Working through the HumanNeeded relay',
  key_full: 'Three other computers use this account',
  error: 'The relay refused this computer',
};

let st = null;          // last status
let cfg = {};           // puck settings (GET all)
let schema = null;      // puck SCHEMA
let view = 'v-home';
let cur = null;         // state being edited
let busy = false;       // a slider is being dragged

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

// ---- the window follows its content ----
function fit() {
  requestAnimationFrame(() => {
    const h = Math.ceil($('panel').getBoundingClientRect().height);
    if (h > 0) invoke('panel_fit', { height: h }).catch(() => {});
  });
}
function go(id, back) {
  document.querySelectorAll('.view').forEach((v) => v.classList.remove('on', 'back'));
  const v = $(id);
  v.classList.add('on');
  if (back) v.classList.add('back');
  view = id;
  fit();
}
function homeView() { return st && st.puck && st.puck.connected ? 'v-home' : 'v-empty'; }

// ---- puck data ----
const val = (k) => (cfg[k] === undefined ? undefined : String(cfg[k]));
function list(name) {
  const l = (schema && schema.lists && schema.lists[name]) || [];
  return l.map((o) => (Array.isArray(o) ? o : [o, o]));
}
function label(name, v) {
  const o = list(name).find(([k]) => k === v);
  return o ? o[1].replace(/\s*\(.*\)$/, '') : v;
}
async function loadPuck() {
  if (!st || !st.puck || !st.puck.compatible) return;
  try {
    if (!schema) schema = await invoke('puck_schema');
    cfg = await invoke('puck_settings');
  } catch (e) { return; }
  renderHome();
  if (view === 'v-state' && cur) renderState();
}
async function set(key, value) {
  cfg[key] = value;
  try { await invoke('puck_set', { key, value: String(value) }); } catch (e) { fail(e); }
}
async function act(action, arg) {
  try { await invoke('puck_action', { action, arg }); } catch (e) { fail(e); }
}

// ---- home ----
function problem() {
  const h = st.http || {};
  if (h.error) {
    return {
      text: "Agents can't reach the app",
      fix: `Another program is using port ${h.port || 24816} (${h.error}). Close it or restart your computer; HumanNeeded keeps trying.`,
    };
  }
  if (st.proxy && st.proxy.problem) {
    return { text: 'A proxy blocks your agents', fix: st.proxy.problem, code: st.proxy.fix, btn: 'Copy' };
  }
  if (st.puck.connected && !st.puck.compatible) {
    return { text: 'Firmware too old for this app', fix: 'Update the puck to use it with this app. Settings are kept.', btn: 'Update firmware' };
  }
  return null;
}
function paintRange(r) {
  const p = ((r.value - r.min) / (r.max - r.min)) * 100;
  r.style.setProperty('--p', `${p}%`);
  const v = r.parentNode.querySelector('.v');
  if (v) v.textContent = `${Math.round(r.value)}%`;
}
function tileText(id) {
  const light = val(`${id}.light`);
  if (light === undefined) return '';
  if (light === 'off') return 'Off';
  const snd = val(`${id}.sound`);
  return label('light', light) + (snd && snd !== 'none' ? ` · ${label('sound', snd).toLowerCase()}` : '');
}
function renderHome() {
  if (!st) return;
  const p = st.puck;
  const pr = problem();
  const shown = String((p && p.shown) || 'idle').replace(/^test:/, '');
  $('dev-ring').style.color = val(`${shown}.color`) || 'var(--hn-faint)';
  $('dev-st').style.setProperty('--c', pr ? 'var(--hn-problem)' : 'var(--hn-ok)');
  $('dev-text').textContent = pr ? pr.text : 'Connected';
  $('fix').hidden = !pr;
  if (pr) {
    $('fix-text').textContent = pr.fix;
    $('fix-code').hidden = !pr.code;
    $('fix-code').textContent = pr.code || '';
    $('fix-btn').hidden = !pr.btn;
    $('fix-btn').textContent = pr.btn || '';
  }
  if (!busy) {
    if (val('brightness') !== undefined) $('bright').value = val('brightness');
    if (val('volume') !== undefined) $('vol').value = val('volume');
  }
  paintRange($('bright'));
  paintRange($('vol'));
  const m = (p.status && p.status.muted) || 'off';
  const muted = m !== 'off';
  $('mute').setAttribute('aria-pressed', muted);
  $('mute').setAttribute('aria-label', muted ? 'Unmute the puck' : 'Mute the puck');
  $('mute-ic').setAttribute('href', muted ? '#i-volx' : '#i-vol');
  $('volrow').classList.toggle('off', muted);
  const q = st.quiet || {};
  let note = '';
  if (q.active) note = `Quiet hours until ${q.to}: dimmed and muted.`;
  else if (muted && m !== 'on') note = `Muted for ${m} more min.`;
  $('lv-note').hidden = !note;
  $('lv-note').textContent = note;
  $('tiles').innerHTML = STATES.map((s) => {
    const off = val(`${s.id}.light`) === 'off';
    return `<button type="button" class="tile${off ? ' off' : ''}" data-state="${s.id}" style="--c:${esc(val(`${s.id}.color`) || 'var(--hn-' + s.id + ')')}">
      <span class="lamp"></span><b>${esc(s.name)}</b><small>${esc(tileText(s.id))}</small></button>`;
  }).join('');
}
let levelTimer;
function level(key, el) {
  paintRange(el);
  busy = true;
  clearTimeout(levelTimer);
  levelTimer = setTimeout(async () => { await set(key, el.value); busy = false; }, 120);
}
$('bright').addEventListener('input', (e) => level('brightness', e.target));
$('vol').addEventListener('input', (e) => level('volume', e.target));
$('mute').onclick = async () => {
  const muted = $('mute').getAttribute('aria-pressed') === 'true';
  await act('mute', muted ? '0' : 'on');
  refresh();
};
$('fix-btn').onclick = async () => {
  const pr = problem();
  if (!pr) return;
  if (pr.btn === 'Copy') {
    try { await navigator.clipboard.writeText(pr.code); toast('Copied'); } catch (e) { fail(e); }
  } else {
    go('v-settings');
    updateFirmware();
  }
};

// ---- state page ----
function renderState() {
  const s = cur;
  const g = schema && (schema.groups || []).find((x) => x.id === s.id);
  $('st-title').textContent = s.name;
  $('st-desc').textContent = g ? g.desc : '';
  const light = val(`${s.id}.light`) || 'solid';
  const on = light !== 'off';
  $('st-on').setAttribute('aria-checked', on);
  $('st-body').classList.toggle('dim', !on);
  const offered = list('light').map(([k]) => k).filter((k) => k !== 'off');
  const lights = LIGHT_ORDER.filter((l) => offered.includes(l)).concat(offered.filter((l) => !LIGHT_ORDER.includes(l)));
  if (!lights.length) lights.push(...LIGHT_ORDER);
  if (on && !lights.includes(light)) lights.push(light);
  $('fx').innerHTML = lights.map((l) => `<button type="button" data-fx="${esc(l)}" aria-pressed="${l === light}">
    <svg viewBox="0 0 34 12">${FX_ICON[l] || '<circle cx="17" cy="6" r="4" fill="currentColor"/>'}</svg>${esc(LIGHT_SHORT[l] || label('light', l))}</button>`).join('');
  const color = (val(`${s.id}.color`) || '').toLowerCase();
  $('swatches').innerHTML = SWATCHES.map((c) => `<button type="button" style="--c:${c}" data-c="${c}" aria-label="Colour ${c}" aria-pressed="${c === color}"></button>`).join('')
    + `<button type="button" class="custom" id="custom" aria-label="Pick any colour" aria-pressed="${!SWATCHES.includes(color)}"></button><input type="color" id="picker" value="${esc(color || '#ffffff')}" tabindex="-1">`;
  const hasSound = val(`${s.id}.sound`) !== undefined;
  $('st-sound-f').hidden = !hasSound;
  if (hasSound) {
    $('st-sound').innerHTML = list('sound').map(([k, l]) => `<option value="${esc(k)}">${esc(l)}</option>`).join('');
    $('st-sound').value = val(`${s.id}.sound`);
  }
  $('st-dur-f').hidden = s.id !== 'done';
  if (s.id === 'done') {
    $('st-dur').innerHTML = list('duration').filter(([k]) => k !== '0').map(([k, l]) => `<option value="${esc(k)}">${esc(l)}</option>`).join('');
    $('st-dur').value = val('done.duration');
  }
  fit();
}
function lastLight(id) {
  try { return localStorage.getItem(`hn.light.${id}`); } catch (e) { return null; }
}
function rememberLight(id, l) {
  try { localStorage.setItem(`hn.light.${id}`, l); } catch (e) { /* not kept */ }
}
$('st-on').onclick = async () => {
  const id = cur.id;
  const light = val(`${id}.light`);
  if (light && light !== 'off') {
    rememberLight(id, light);
    await set(`${id}.light`, 'off');
  } else {
    await set(`${id}.light`, lastLight(id) || DEFAULTS[id].light);
  }
  renderState();
};
$('fx').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-fx]');
  if (!b) return;
  await set(`${cur.id}.light`, b.dataset.fx);
  renderState();
});
$('swatches').addEventListener('click', async (e) => {
  const b = e.target.closest('button');
  if (!b) return;
  if (b.id === 'custom') { $('picker').click(); return; }
  await set(`${cur.id}.color`, b.dataset.c);
  renderState();
});
$('swatches').addEventListener('change', async (e) => {
  if (e.target.id !== 'picker') return;
  await set(`${cur.id}.color`, e.target.value);
  renderState();
});
$('st-sound').onchange = (e) => set(`${cur.id}.sound`, e.target.value);
$('st-dur').onchange = (e) => set('done.duration', e.target.value);
$('st-prev').onclick = async () => {
  await act('test', cur.id);
  toast(`Playing "${cur.name}" on your puck`);
};
$('st-reset').onclick = async () => {
  const d = DEFAULTS[cur.id];
  for (const [k, v] of Object.entries(d)) {
    if (val(`${cur.id}.${k}`) !== undefined && val(`${cur.id}.${k}`) !== v) await set(`${cur.id}.${k}`, v);
  }
  renderState();
  toast('Back to default');
};

// ---- settings ----
function sw(id, on, disabled) {
  return `<button type="button" class="sw" role="switch" aria-checked="${!!on}" data-agent="${esc(id)}" aria-label="${esc(AGENT_NAME[id] || id)}"${disabled ? ' disabled' : ''}></button>`;
}
function renderSettings() {
  if (!st) return;
  const rows = (st.agents || []).map((a) => {
    const right = a.state === 'connected'
      ? sw(a.id, a.enabled)
      : '<button type="button" class="btn small" data-guide>Set up<svg class="i" style="width:13px;height:13px"><use href="#i-ext"/></svg></button>';
    const small = a.state === 'connected' ? (a.enabled ? '' : "Doesn't light the puck") : 'Not set up yet';
    return `<div class="row"><span class="agentmark">${esc(AGENT_MARK[a.id] || a.id.slice(0, 2))}</span><span class="gr">${esc(AGENT_NAME[a.id] || a.name)}${small ? `<small>${esc(small)}</small>` : ''}</span>${right}</div>`;
  });
  const c = st.cloud;
  if (c) {
    const text = c.enabled ? (c.error || CLOUD_TEXT[c.state] || '') : CLOUD_TEXT.off;
    rows.splice(2, 0, `<div class="row"><span class="agentmark"><svg class="i" style="width:14px;height:14px"><use href="#i-cloud"/></svg></span><span class="gr">Cowork<small>${esc(text)}</small></span>${sw('claude-cloud', c.enabled)}</div>`);
  }
  $('agents').innerHTML = rows.join('');

  const q = st.quiet || {};
  $('q-on').setAttribute('aria-checked', !!q.enabled);
  if (document.activeElement !== $('q-from') && q.from) $('q-from').value = q.from;
  if (document.activeElement !== $('q-to') && q.to) $('q-to').value = q.to;

  if (!fwBusy) {
    const p = st.puck;
    $('fw-s').textContent = p.connected ? (fwNote ? `${p.fw} · ${fwNote}` : p.fw) : 'Plug in your puck to check its firmware';
    $('fw-check').disabled = !p.connected;
  }

  const u = st.update || {};
  $('auto-update').setAttribute('aria-checked', !!u.auto);
  let ver = `Version ${st.app.version}`;
  if (u.checking) ver += ' · checking…';
  else if (u.last) ver += ` · ${u.last}`;
  $('app-ver').textContent = ver;
  $('app-restart').hidden = !u.ready;
  $('app-restart').textContent = u.ready ? `Restart to update to ${u.ready}` : '';

  const p = st.puck || {};
  const kv = [];
  if (p.serial) kv.push(['Serial', p.serial]);
  if (p.board) kv.push(['Board', p.board]);
  if (p.port) kv.push(['USB', p.port]);
  kv.push(['Agent port', (st.http && st.http.port) || '']);
  $('kv').innerHTML = kv.map(([k, v]) => `<span>${esc(k)}</span><span class="mono">${esc(v)}</span>`).join('');
  $('restart').disabled = !p.connected;
  if (!$('danger').innerHTML) resetButton();
  fit();
}
$('agents').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-agent]');
  if (!b || b.disabled) return;
  const on = b.getAttribute('aria-checked') !== 'true';
  b.disabled = true;
  try {
    if (b.dataset.agent === 'claude-cloud') await invoke('cloud_set', { on });
    else await invoke('agent_set', { id: b.dataset.agent, on });
  } catch (err) { fail(err); }
  refresh();
});
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

let fwNote = '';
let fwBusy = false;
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
  renderSettings();
};
async function updateFirmware() {
  fwBusy = true;
  $('fw-go').disabled = true;
  $('fw-check').disabled = true;
  $('fw-s').textContent = 'Updating firmware, about a minute. Settings are kept.';
  try {
    const v = await invoke('fw_update');
    fwNote = '';
    $('fw-go').hidden = true;
    toast(`Updated to ${v}`);
    schema = null;
  } catch (e) { fwNote = `update failed: ${e}`; }
  fwBusy = false;
  $('fw-go').disabled = false;
  refresh();
}
$('fw-go').onclick = updateFirmware;
listen('fw-progress', (ev) => { if (fwBusy) $('fw-s').textContent = ev.payload; });

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
$('restart').onclick = async () => { await act('reboot'); toast('Restarting the puck'); };
$('clear').onclick = async () => { await invoke('clear_sessions'); toast('Light cleared'); refresh(); };
$('log').onclick = () => invoke('open_log').catch(fail);
$('adv').addEventListener('toggle', fit);
function resetButton() {
  $('danger').innerHTML = '<button type="button" class="btn red" data-reset>Reset all settings</button>';
  fit();
}
$('danger').addEventListener('click', async (e) => {
  if (e.target.closest('[data-reset]')) {
    $('danger').innerHTML = '<p class="muted" style="margin:0 0 8px;font-size:12px">Reset the puck\'s lights and sounds to their defaults? Agents, quiet hours and app settings stay.</p><div class="btns"><button type="button" class="btn red" data-yes>Reset</button><button type="button" class="btn" data-no>Cancel</button></div>';
    fit();
  } else if (e.target.closest('[data-yes]')) {
    await act('reset');
    toast('Settings reset');
    resetButton();
    loadPuck();
  } else if (e.target.closest('[data-no]')) resetButton();
});

// ---- navigation ----
document.addEventListener('click', (e) => {
  const t = e.target;
  const g = t.closest('[data-go]');
  if (g) { if (g.dataset.go === 'v-settings') renderSettings(); go(g.dataset.go); return; }
  if (t.closest('[data-back]')) { renderHome(); go(homeView(), true); return; }
  if (t.closest('[data-guide]')) { invoke('open_setup_guide').catch(fail); return; }
  const tile = t.closest('[data-state]');
  if (tile) {
    cur = STATES.find((s) => s.id === tile.dataset.state);
    go('v-state');
    renderState();
  }
});
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (view === 'v-home' || view === 'v-empty') invoke('panel_hide');
  else { renderHome(); go(homeView(), true); }
});

// ---- status ----
let wasConnected = null;
async function refresh() {
  try { st = await invoke('status'); } catch (e) { return; }
  const connected = !!(st.puck && st.puck.connected);
  if (connected !== wasConnected) {
    wasConnected = connected;
    if (connected) await loadPuck();
    if (view === 'v-home' || view === 'v-empty') go(homeView());
    if (view === 'v-state' && !connected) go(homeView(), true);
  }
  renderHome();
  if (view === 'v-settings') renderSettings();
}

listen('changed', () => refresh());
listen('panel-shown', () => {
  // Opening the panel starts from the puck, with fresh values.
  go(homeView());
  refresh().then(loadPuck);
});
invoke('autostart_get').then((on) => $('autostart').setAttribute('aria-checked', !!on)).catch(() => {});
refresh().then(() => go(homeView()));
setInterval(() => { if (!document.hidden) refresh(); }, 5000);
