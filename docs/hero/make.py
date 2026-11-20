# SPDX-License-Identifier: FSL-1.1-ALv2
"""Renders the README hero (docs/images/hero.gif) and the GitHub social preview
(docs/images/social-preview.png) from scene.html, with the real panel (app/ui) inside.

    python docs/hero/make.py            # needs Pillow and Chrome or Edge

Rerun it whenever the panel's design changes. Each frame is one headless-browser screenshot of
scene.html#t=<ms>, so the timing is exact and nothing on your own screen is captured.
"""
import functools, http.server, os, re, shutil, subprocess, sys, tempfile, threading
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
UI = ROOT / "app" / "ui"
OUT = ROOT / "docs" / "images"
W, H = 960, 540
FPS = 12
LOOP_MS = 8000

BROWSERS = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "google-chrome", "chromium", "msedge",
]

# A stand-in for the app behind the panel: a fixed status per state (#needs, #done, …).
MOCK = r"""<script>
(() => {
  const state = (location.hash || '#working').slice(1);
  const look = (on, light, color, brightness, duration) => ({ on, light, color, brightness, duration });
  const snd = (sound) => ({ on: true, sound, volume: 70 });
  const states = {
    needs: { screen: look(true, 'breathe', '#ffa62b', 100, 0), sound: snd('call'), puck: look(true, 'breathe', '#ffa62b', 100, 0), puck_sound: snd('call') },
    done: { screen: look(true, 'breathe', '#22e04a', 100, 10), sound: snd('bright'), puck: look(true, 'breathe', '#22e04a', 100, 3), puck_sound: snd('bright') },
    working: { screen: look(false, 'spin', '#3366ff', 60, 0), puck: look(true, 'spin', '#3366ff', 60, 0) },
    idle: { screen: look(false, 'solid', '#ffd2a0', 20, 0), puck: look(true, 'solid', '#ffd2a0', 12, 0) },
  };
  const now = Date.now();
  const sessions = state === 'needs' || state === 'working' ? [{ agent: 'claude', state, set_at: now - 20000 }] : [];
  const status = {
    app: { version: '1.0.0' }, http: { port: 24816, listening: true }, puck: { connected: false },
    agents: [
      { id: 'claude', state: 'connected', enabled: true, found: true, last_seen: now - 20000 },
      { id: 'codex', state: 'connected', enabled: true, found: true, last_seen: now - 3600e3 },
      { id: 'cursor', state: 'not_set_up', enabled: true, found: false },
    ],
    cloud: { enabled: true, state: 'working' }, cloud_last_seen: now - 2 * 3600e3,
    sessions, output: { shown: state, status: { muted: 'off' } }, quiet: {}, update: {}, now,
    glow: { enabled: true, screens: 'all', thickness: 'mid', skip_fullscreen: true },
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd) => cmd === 'status' ? status : cmd === 'alerts_get' ? { alerts: { version: 1, states }, defaults: states } : null },
    event: { listen: async () => {} },
  };
})();
</script>
"""


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def browser():
    for b in BROWSERS:
        p = shutil.which(b) or (b if os.path.exists(b) else None)
        if p:
            return p
    sys.exit("Chrome or Edge not found")


def panel_page():
    html = (UI / "index.html").read_text(encoding="utf8")
    html = html.replace("<head>", '<head>\n<base href="../../app/ui/">', 1)
    html = re.sub(r'(<script src="app\.js"></script>)', MOCK + r"\1", html, count=1)
    (HERE / "panel.html").write_text(html, encoding="utf8")


def shot(exe, url, path, w=W, h=H, scale=1.0, wait=2500):
    subprocess.run([exe, "--headless=new", "--disable-gpu", "--hide-scrollbars", f"--force-device-scale-factor={scale}",
                    f"--window-size={w},{h}", f"--virtual-time-budget={wait}", "--allow-file-access-from-files",
                    f"--screenshot={path}", url], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    exe = browser()
    panel_page()
    # Served over http (not file://): the browser blocks masked images (the agent logos) from files.
    handler = functools.partial(QuietHandler, directory=str(ROOT))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    scene = f"http://127.0.0.1:{server.server_port}/docs/hero/scene.html"
    tmp = Path(tempfile.mkdtemp())
    frames = []
    try:
        for i in range(LOOP_MS * FPS // 1000):
            p = tmp / f"f{i:03}.png"
            shot(exe, f"{scene}#t={i * 1000 // FPS}", p)
            frames.append(Image.open(p).convert("RGB"))
            print(f"\rframe {i + 1}/{LOOP_MS * FPS // 1000}", end="", flush=True)
        print()
        # One shared palette, from a working, a needs and a done frame, keeps every colour steady
        # and the file small.
        picks = [frames[int(len(frames) * x)] for x in (0.1, 0.33, 0.75)]
        sheet = Image.new("RGB", (W, H * len(picks)))
        for k, f in enumerate(picks):
            sheet.paste(f, (0, H * k))
        pal = sheet.quantize(colors=255, method=Image.Quantize.MEDIANCUT)
        gif = [f.quantize(palette=pal, dither=Image.Dither.NONE) for f in frames]
        OUT.mkdir(parents=True, exist_ok=True)
        gif[0].save(OUT / "hero.gif", save_all=True, append_images=gif[1:], duration=1000 // FPS, loop=0, optimize=True)
        # Social preview: GitHub's 1280×640 (the scene at 1120×560, scaled), the "needs you" moment.
        shot(exe, f"{scene}#t=2700", tmp / "social.png", 1120, 560, scale=1280 / 1120, wait=5000)
        Image.open(tmp / "social.png").convert("RGB").save(OUT / "social-preview.png", optimize=True)
        print("wrote", OUT / "hero.gif", "and", OUT / "social-preview.png")
    finally:
        server.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)
        (HERE / "panel.html").unlink(missing_ok=True)


if __name__ == "__main__":
    main()
