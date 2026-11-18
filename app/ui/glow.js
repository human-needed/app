// SPDX-License-Identifier: GPL-3.0-or-later
// Screen-edge glow page (D-059): shows what the app sends ({state, color, light, level}).
(() => {
  const g = document.getElementById('g');
  const LIGHTS = ['solid', 'breathe', 'pulse', 'blink', 'heartbeat', 'flicker', 'spin'];
  // The light fades in, its effect runs for the state's duration, then it fades out. The fades
  // (glow.css, the engine's LIGHT_FADE_MS) don't count towards the duration.
  const FADE_MS = 250;
  let off = null;

  let shown = '';

  function apply(s) {
    clearTimeout(off);
    if (!s || !s.state) {
      shown = '';
      // Fade out from where the animation ended (a whole cycle), not into the next one.
      g.classList.add('hold');
      g.style.opacity = '0';
      // Stop the animation once faded out, so an idle screen costs nothing.
      off = setTimeout(() => { g.className = ''; }, FADE_MS + 50);
      return;
    }
    g.style.setProperty('--c', /^#[0-9a-f]{6}$/i.test(s.color) ? s.color : '#ffffff');
    const light = LIGHTS.includes(s.light) ? s.light : 'solid';
    // Like the puck, every animation is a 1 s cycle, counted from the end of the fade-in, so a
    // state of any duration (whole seconds) plays that many complete cycles. A page that starts
    // late (new window, full-screen app gone) joins at the right point.
    const key = `${s.state}/${light}/${s.since}`;
    if (key !== shown) {
      shown = key;
      g.className = '';
      void g.offsetWidth;
      const into = Date.now() - (Number(s.since) || 0) - FADE_MS;
      g.style.setProperty('--d', `${into < 0 ? -into : -(into % 1000)}ms`);
      g.className = light;
    }
    g.dataset.t = ['thin', 'wide'].includes(s.thickness) ? s.thickness : 'mid';
    g.style.opacity = String(Math.max(0, Math.min(1, Number(s.level) || 0)));
  }

  const T = window.__TAURI__;
  if (!T) return;
  T.event.listen('glow', (e) => apply(e.payload));
  T.core.invoke('glow_spec').then(apply).catch(() => {});
})();
