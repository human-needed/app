// Screen-edge glow page (D-059): shows what the app sends ({state, color, light, level}).
(() => {
  const g = document.getElementById('g');
  const LIGHTS = ['solid', 'breathe', 'pulse', 'blink', 'heartbeat', 'flicker', 'spin'];
  let off = null;

  function apply(s) {
    clearTimeout(off);
    if (!s || !s.state) {
      g.style.opacity = '0';
      // Stop the animation once faded out, so an idle screen costs nothing.
      off = setTimeout(() => { g.className = ''; }, 700);
      return;
    }
    g.style.setProperty('--c', /^#[0-9a-f]{6}$/i.test(s.color) ? s.color : '#ffffff');
    const light = LIGHTS.includes(s.light) ? s.light : 'solid';
    if (g.className !== light) g.className = light;
    g.style.opacity = String(Math.max(0, Math.min(1, Number(s.level) || 0)));
  }

  const T = window.__TAURI__;
  if (!T) return;
  T.event.listen('glow', (e) => apply(e.payload));
  T.core.invoke('glow_spec').then(apply).catch(() => {});
})();
