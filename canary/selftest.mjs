// Key-free self-test of the canary (runs on every PR): fires the REAL hook entries from a
// human-needed/plugins checkout the way each agent would, then checks the checker.
//   PLUGINS_DIR=../plugins node canary/selftest.mjs        (needs sh + curl)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { startHub, check, checkClaudeSessions, fingerprint, EXPECT } from "./lib.mjs";

const PLUGINS = process.env.PLUGINS_DIR;
if (!PLUGINS) { console.error("set PLUGINS_DIR"); process.exit(2); }
const FILES = { claude: "plugins/claude/hooks/hooks.json", codex: "plugins/codex/hooks.json", cursor: "plugins/cursor/hooks.json" };
let fails = 0;
const ok = (c, name, extra = "") => { console.log(`  ${c ? "ok  " : "FAIL"} ${name}${!c && extra ? "  -- " + extra : ""}`); if (!c) fails++; };

// Fire every hook entry for each expected event, like the agent would.
async function simulate(agent, drop = null) {
  const hooks = JSON.parse(readFileSync(join(PLUGINS, FILES[agent]), "utf8")).hooks;
  const hub = await startHub();
  const events = [...EXPECT[agent].required, ...EXPECT[agent].soft].map((x) => x.event);
  for (const ev of events) {
    if (ev === drop) continue;
    const body = JSON.stringify(agent === "cursor"
      ? { conversation_id: "sim-1", hook_event_name: ev, ...(ev === "sessionEnd" ? { session_id: "sim-1" } : {}) }
      : { session_id: "sim-1", hook_event_name: ev, cwd: "/tmp" });
    const entries = agent === "cursor" ? hooks[ev] ?? [] : (hooks[ev] ?? []).flatMap((g) => g.hooks);
    for (const h of entries) {
      if (h.type === "http") {
        await fetch(h.url, { method: "POST", headers: { "Content-Type": "application/json" }, body });
      } else if (h.command && h.command !== "sh") {
        const r = spawnSync("sh", ["-c", h.command], { input: body, encoding: "utf8" });
        if (r.stdout) ok(false, `${agent} ${ev}: hook printed to stdout`, r.stdout);
      }
    }
  }
  await new Promise((r) => setTimeout(r, 300));
  await hub.close();
  return hub.events;
}

for (const agent of Object.keys(FILES)) {
  console.log(agent);
  const good = check(agent, await simulate(agent));
  ok(good.failures.length === 0 && good.soft.length === 0, "all mapped hooks post with a session id",
     JSON.stringify([...good.failures, ...good.soft]));
  const victim = EXPECT[agent].required.at(-2).event;
  const bad = check(agent, await simulate(agent, victim));
  ok(bad.failures.some((f) => f.id === `missing:${victim}`), `dropping ${victim} is caught`, JSON.stringify(bad.failures));
  ok(fingerprint(agent, bad.failures) === fingerprint(agent, [...bad.failures].reverse()), "fingerprint ignores order");
  ok(fingerprint(agent, bad.failures) !== fingerprint(agent, good.failures), "fingerprint differs per break");
}

console.log("checker edge cases");
const ev = (path, body, ctype = "application/json") => ({ method: "POST", path, ctype, body });
let r = check("claude", [ev("/v1/event/claude/done", { hook_event_name: "Stop" })]);
ok(r.failures.some((f) => f.id === "nosession:Stop"), "post without session id fails");
r = check("claude", [ev("/v1/event/claude/done", { session_id: "a", hook_event_name: "Stop" }, "text/plain")]);
ok(r.failures.some((f) => f.id === "ctype:Stop"), "wrong content-type fails");
r = check("claude", [ev("/v1/event/claude/working", { session_id: "a", hook_event_name: "Stop" })]);
ok(r.failures.some((f) => f.id === "missing:Stop"), "Stop mapped to the wrong state fails");
ok(checkClaudeSessions([], "a")[0]?.id === "sessions-file:missing", "missing sessions file fails");
ok(checkClaudeSessions([{ file: "12.json", json: { sessionId: "a", pid: 12, status: "busy" } }], "a").length === 0, "good sessions file passes");
ok(checkClaudeSessions([{ file: "12.json", json: { session: "a", pid: 12, status: "busy" } }], "a")[0]?.id === "sessions-file:fields", "renamed sessionId fails");
ok(checkClaudeSessions([{ file: "12.json", json: { sessionId: "a", pid: 12, status: "thinking" } }], "a")[0]?.id === "sessions-file:status", "new status value fails");

console.log(fails ? `\n${fails} FAILED` : "\nall ok");
process.exit(fails ? 1 : 0);
