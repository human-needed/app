// SPDX-License-Identifier: GPL-3.0-or-later
// Screen-edge glow page (D-059): shows what the app sends ({state, color, light, level}).
(() => {
  const g = document.getElementById('g');
  const LIGHTS = ['solid', 'breathe', 'pulse', 'blink', 'heartbeat', 'flicker', 'spin'];
  let off = null;

  let shown = '';

  function apply(s) {
    clearTimeout(off);
    if (!s || !s.state) {
      shown = '';
      // Fade out from where the animation ended (a whole cycle), not into the next one.
      g.classList.add('hold');
      g.style.transitionDuration = '400ms';
      g.style.opacity = '0';
      // Stop the animation once faded out, so an idle screen costs nothing.
      off = setTimeout(() => { g.className = ''; }, 500);
      return;
    }
    g.style.setProperty('--c', /^#[0-9a-f]{6}$/i.test(s.color) ? s.color : '#ffffff');
    const light = LIGHTS.includes(s.light) ? s.light : 'solid';
    // Like the puck, every animation is a 1 s cycle counted from when the state started, so a
    // state of any duration (whole seconds) ends on a complete cycle. A page that starts late
    // (new window, full-screen app gone) joins at the right point.
    const key = `${s.state}/${light}/${s.since}`;
    if (key !== shown) {
      shown = key;
      g.className = '';
      void g.offsetWidth;
      const into = Math.max(0, Date.now() - (Number(s.since) || 0)) % 1000;
      g.style.setProperty('--d', `-${into}ms`);
      g.className = light;
    }
    // An animated light comes up quickly, so its first cycle looks like all the others.
    g.style.transitionDuration = light === 'solid' ? '400ms' : '120ms';
    g.dataset.t = ['thin', 'wide'].includes(s.thickness) ? s.thickness : 'mid';
    g.style.opacity = String(Math.max(0, Math.min(1, Number(s.level) || 0)));
  }

  const T = window.__TAURI__;
  if (!T) return;
  T.event.listen('glow', (e) => apply(e.payload));
  T.core.invoke('glow_spec').then(apply).catch(() => {});
})();
