// SPDX-License-Identifier: FSL-1.1-ALv2
// Screen-edge glow page (D-059): shows what the app sends ({state, color, light, level}).
(() => {
  const g = document.getElementById('g');
  const LIGHTS = ['solid', 'breathe', 'blink', 'heartbeat', 'flicker', 'spin'];
  // The light fades in, its effect runs for the state's duration, then it fades out. The fades
  // (glow.css, the engine's LIGHT_FADE_MS) don't count towards the duration.
  const FADE_MS = 250;
  let off = null;

  let shown = '';

  // One state's light: a layer (faded in on a change) holding the animated colour.
  function layer(light, color, since) {
    const l = document.createElement('div');
    l.className = 'l';
    const c = document.createElement('div');
    c.className = `c ${light}`;
    c.style.setProperty('--c', color);
    // Like the puck, every animation is a 1 s cycle, counted from the end of the fade-in, so a
    // state of any duration (whole seconds) plays that many complete cycles. A page that starts
    // late (new window, full-screen app gone) joins at the right point.
    const into = Date.now() - (Number(since) || 0) - FADE_MS;
    c.style.setProperty('--d', `${into < 0 ? -into : -(into % 1000)}ms`);
    l.appendChild(c);
    return l;
  }

  function apply(s) {
    clearTimeout(off);
    if (!s || !s.state) {
      shown = '';
      // Fade out from where the animation ended (a whole cycle), not into the next one.
      g.classList.add('hold');
      g.style.opacity = '0';
      // Stop the animation once faded out, so an idle screen costs nothing.
      off = setTimeout(() => { g.replaceChildren(); g.classList.remove('hold'); }, FADE_MS + 50);
      return;
    }
    const color = /^#[0-9a-f]{6}$/i.test(s.color) ? s.color : '#ffffff';
    const light = LIGHTS.includes(s.light) ? s.light : 'solid';
    const key = `${s.state}/${light}/${s.since}`;
    if (key !== shown) {
      shown = key;
      g.classList.remove('hold');
      const next = layer(light, color, s.since);
      const below = [...g.children];
      if (below.length) {
        // A change while a light shows (or is fading out): as on the puck, the light shown now
        // holds its frame and the new one fades in over it, then the old one goes.
        below.forEach((b) => b.classList.add('hold'));
        next.style.opacity = '0';
        g.appendChild(next);
        void next.offsetWidth;
        next.style.opacity = '1';
        setTimeout(() => below.forEach((b) => b.remove()), FADE_MS + 50);
      } else {
        g.appendChild(next);
      }
    } else {
      g.lastElementChild?.firstElementChild?.style.setProperty('--c', color);
    }
    g.dataset.t = ['thin', 'wide'].includes(s.thickness) ? s.thickness : 'mid';
    g.style.opacity = String(Math.max(0, Math.min(1, Number(s.level) || 0)));
  }

  const T = window.__TAURI__;
  if (!T) return;
  T.event.listen('glow', (e) => apply(e.payload));
  T.core.invoke('glow_spec').then(apply).catch(() => {});
})();
