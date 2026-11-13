// HumanNeeded settings window. Talks only to the app (Tauri commands); the app owns the puck.
// The light/sound form is built from the puck's own SCHEMA (same format as firmware 0.3.0).
'use strict';
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const listen = (ev, fn) => window.__TAURI__.event.listen(ev, fn);

const COLORS = { needs: '#ff8800', working: '#3366ff', done: '#22cc55', idle: '#ffaa66' };

function esc(value) {
  return String(value ?? '').replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function showError(e) { $('error').textContent = e ? String(e) : ''; }

// ---- settings form from SCHEMA ----
function optionsOf(field, lists) {
  const list = typeof field.options === 'string' ? lists[field.options] : field.options;
  return (list || []).map((o) => (Array.isArray(o) ? o : [o, o]));
}
function controlHtml(f, lists) {
  const key = `data-key="${esc(f.key)}" aria-label="${esc(f.label)}"`;
  const unit = f.unit ? ` data-unit="${esc(f.unit)}"` : '';
  switch (f.type) {
    case 'range':
      return `<input type="range" ${key}${unit} min="${esc(f.min ?? 0)}" max="${esc(f.max ?? 100)}" step="${esc(f.step ?? 1)}"><span class="value"></span>`;
    case 'select':
      return `<select ${key}${unit}>${optionsOf(f, lists).map(([v, l]) => `<option value="${esc(v)}">${esc(l)}</option>`).join('')}</select>`;
    case 'color':
      return `<input type="color" ${key}><span class="hex"></span>`;
    case 'toggle':
      return `<input type="checkbox" ${key}>`;
    case 'number':
      return `<input type="number" ${key}${unit} min="${esc(f.min ?? '')}" max="${esc(f.max ?? '')}" step="${esc(f.step ?? 1)}">`;
    default:
      return `<input type="text" ${key} maxlength="${esc(f.maxlength ?? 64)}">`;
  }
}
function fieldHtml(f, lists) {
  const tip = f.tip ? ` <button class="tip" type="button" aria-label="About ${esc(f.label)}" data-tip="${esc(f.tip)}">?</button>` : '';
  return `<div class="field"><span class="label">${esc(f.label)}${tip}</span><div class="control">${controlHtml(f, lists)}</div></div>`;
}
function buildForm(schema) {
  const lists = schema.lists || {};
  $('groups').innerHTML = (schema.groups || []).map((g) => `
    <section class="card">
      <div class="card-head">
        ${g.swatch ? `<span class="swatch" data-swatch="${esc(g.swatch)}"></span>` : ''}
        <h2 class="grow">${esc(g.title)}</h2>
        ${g.preview ? `<button type="button" data-preview="${esc(g.preview)}">Preview</button>` : ''}
      </div>
      ${g.desc ? `<p class="card-desc">${esc(g.desc)}</p>` : ''}
      ${(g.fields || []).map((f) => fieldHtml(f, lists)).join('')}
    </section>`).join('');
}
function showValue(el) {
  const label = el.nextElementSibling;
  if (el.type === 'range' && label) label.textContent = el.value + (el.dataset.unit || '');
  if (el.type === 'color') {
    if (label) label.textContent = el.value.toUpperCase();
    for (const s of document.querySelectorAll('[data-swatch]')) {
      if (s.dataset.swatch === el.dataset.key) s.style.background = el.value;
    }
  }
}
function fillForm(settings) {
  for (const el of document.querySelectorAll('[data-key]')) {
    let value = settings[el.dataset.key];
    if (value === undefined) continue;
    if (el.type === 'checkbox') { el.checked = value === true || value === 1 || value === '1'; continue; }
    value = String(value);
    if (el.type === 'color') value = value.toLowerCase();
    if (el.tagName === 'SELECT' && ![...el.options].some((o) => o.value === value)) {
      el.add(new Option(el.dataset.unit ? `${value} ${el.dataset.unit}` : value, value));
    }
    el.value = value;
    showValue(el);
  }
}
let savedTimer;
async function onSettingChange(ev) {
  const el = ev.target;
  if (!el.dataset.key) return;
  const value = el.type === 'checkbox' ? (el.checked ? '1' : '0') : el.value;
  try {
    await invoke('puck_set', { key: el.dataset.key, value });
    $('saved').classList.add('show');
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => $('saved').classList.remove('show'), 1200);
    showError('');
  } catch (e) { showError(`Couldn't save: ${e}`); }
}
$('groups').addEventListener('input', (ev) => showValue(ev.target));
$('groups').addEventListener('change', onSettingChange);
$('groups').addEventListener('click', async (ev) => {
  const p = ev.target.dataset && ev.target.dataset.preview;
  if (!p) return;
  try { await invoke('puck_action', { action: 'test', arg: p }); } catch (e) { showError(e); }
});

// ---- status ----
let formFor = null; // serial+fw the form was built for
async function loadForm(puck) {
  const id = `${puck.serial}/${puck.fw}`;
  if (formFor === id) return;
  try {
    const [schema, settings] = await Promise.all([invoke('puck_schema'), invoke('puck_settings')]);
    buildForm(schema);
    fillForm(settings);
    formFor = id;
    $('settings').classList.remove('hidden');
  } catch (e) { showError(`Couldn't read the puck's settings: ${e}`); }
}
function ago(ms) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${Math.round(s / 3600)} h ago`;
}
const AGENT_NAMES = { claude: 'Claude', codex: 'Codex', cursor: 'Cursor' };
function renderSessions(sessions) {
  const shown = sessions.filter((s) => s.state === 'working' || s.state === 'needs' || Date.now() - s.last_seen < 3600e3)
    .filter((s) => s.state !== 'ended');
  $('sessions').innerHTML = shown.map((s) => `
    <li><span class="pill ${esc(s.state)}">${esc(s.state === 'needs' ? 'needs you' : s.state)}</span>
      <span class="grow">${esc(AGENT_NAMES[s.agent] || s.agent)} <span class="muted mono">${esc(s.session.slice(0, 8))}</span></span>
      <span class="muted" title="${esc(s.reason)}">${esc(ago(s.last_seen))}</span></li>`).join('');
}
async function refresh() {
  let st;
  try { st = await invoke('status'); } catch (e) { showError(e); return; }
  const p = st.puck;
  $('app-version').textContent = `App ${st.app.version}`;
  let shown = 'off';
  if (!p.connected) {
    $('puck-title').textContent = 'Puck not connected';
    $('puck-detail').textContent = 'Plug your puck into a USB port.';
    $('device').textContent = 'Not connected.';
    $('settings').classList.add('hidden');
    formFor = null;
  } else if (!p.compatible) {
    $('puck-title').textContent = 'Your puck needs a firmware update';
    $('puck-detail').textContent = `Firmware ${p.fw} is too old for this app.`;
    $('device').textContent = `Firmware ${p.fw} · ${p.serial} · ${p.port}`;
  } else {
    const snap = st.snapshot;
    const needs = snap.filter((e) => e.state === 'needs').length;
    const working = snap.filter((e) => e.state === 'working').length;
    shown = p.shown || (needs ? 'needs' : working ? 'working' : 'idle');
    const words = [];
    if (needs) words.push(`${needs} need${needs > 1 ? '' : 's'} you`);
    if (working) words.push(`${working} working`);
    $('puck-title').textContent = words.length ? words.join(', ') : 'All quiet';
    const muted = p.status && p.status.muted;
    $('puck-detail').textContent = muted && muted !== 'off'
      ? (muted === 'on' ? 'Sounds muted.' : `Sounds muted for ${muted} more min.`) : 'Connected.';
    $('device').textContent = `Firmware ${p.fw} · ${p.board} · ${p.serial} · ${p.port}`;
    loadForm(p);
  }
  if (p.updating) $('puck-detail').textContent = `Updating firmware: ${p.updating}`;
  const base = String(shown).replace(/^test:/, '');
  $('light').style.background = COLORS[base] || 'transparent';
  $('light').style.boxShadow = COLORS[base] ? `0 0 18px ${COLORS[base]}` : '';
  renderSessions(st.sessions || []);
}

async function refreshAgents() {
  try {
    const r = await invoke('integrations_list');
    $('agents').innerHTML = r.agents.map((a) => {
      let right;
      if (a.installed) right = '<span class="ok">Installed</span>';
      else if (!a.detected) right = '<span class="muted">Not found on this computer</span>';
      else if (a.can_install) right = `<button type="button" class="primary" data-install="${esc(a.id)}">Install</button>`;
      else right = `<span class="muted">${esc(a.manual)}</span>`;
      const manual = a.detected && !a.installed && a.can_install
        ? `<button class="tip" type="button" aria-label="Install by hand" data-tip="${esc(a.manual)}">?</button>` : '';
      return `<li><span class="grow">${esc(a.name)}</span>${manual}${right}</li>`;
    }).join('');
    const hn = r.hn;
    $('hn-note').classList.toggle('hidden', !!hn.on_path);
    $('hn-note').textContent = hn.on_path ? '' : 'The hn command is not on your PATH yet; agents can\'t reach the app until it is.';
  } catch (e) { showError(e); }
}
$('agents').addEventListener('click', async (ev) => {
  const id = ev.target.dataset && ev.target.dataset.install;
  if (!id) return;
  ev.target.disabled = true;
  ev.target.textContent = 'Installing…';
  try { showError(''); alert(await invoke('integration_install', { id, remove: false })); } catch (e) { showError(e); }
  refreshAgents();
});

// ---- buttons ----
async function act(action, arg) {
  try { showError(''); await invoke('puck_action', { action, arg }); } catch (e) { showError(e); }
  refresh();
}
$('test').onclick = () => act('test', 'needs');
$('mute').onchange = (ev) => act('mute', ev.target.value || '0');
$('restart').onclick = () => act('reboot');
$('reset').onclick = async () => {
  if (!confirm('Reset all light and sound settings to their defaults?')) return;
  await act('reset');
  formFor = null;
  refresh();
};
$('clear').onclick = async () => { await invoke('clear_sessions'); refresh(); };
$('log').onclick = () => invoke('open_log').catch(showError);
$('autostart').onchange = (ev) => invoke('autostart_set', { on: ev.target.checked }).catch(showError);

$('fw-check').onclick = async () => {
  $('update-note').textContent = 'Checking…';
  try {
    const a = await invoke('fw_check');
    if (!a.latest) $('update-note').textContent = 'No firmware is published for this puck yet.';
    else if (a.newer) {
      $('update-note').textContent = `Firmware ${a.latest.version} is available (you have ${a.current}).`;
      $('fw-update').classList.remove('hidden');
    } else $('update-note').textContent = `Up to date (${a.current}).`;
  } catch (e) { $('update-note').textContent = `Couldn't check: ${e}`; }
};
$('fw-update').onclick = async () => {
  $('fw-update').disabled = true;
  try {
    const v = await invoke('fw_update');
    $('update-note').textContent = `Updated to ${v}. Your settings are kept.`;
    $('fw-update').classList.add('hidden');
    formFor = null;
  } catch (e) { $('update-note').textContent = `Update failed: ${e}`; }
  $('fw-update').disabled = false;
  refresh();
};
listen('fw-progress', (ev) => { $('update-note').textContent = ev.payload; });
listen('changed', () => refresh());

invoke('autostart_get').then((on) => { $('autostart').checked = on; });
refresh();
refreshAgents();
setInterval(refresh, 5000);
