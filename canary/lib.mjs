// Shared pieces of the agent-compatibility canary (pm/contracts/testing.md §1).
// No npm packages: node >= 20 only.
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

export const PORT = 24816;
export const HERE = dirname(fileURLToPath(import.meta.url));
export const EXPECT = JSON.parse(readFileSync(join(HERE, "expect.json"), "utf8"));
const PATH_RE = /^\/v1\/event\/([a-z0-9-]{1,32})\/(working|needs|done|idle|end)(\?.*)?$/;

// Recording stand-in for the app's hub (app-events §1): records every request, replies 204.
export function startHub(port = PORT) {
  const events = [];
  const server = createServer((req, res) => {
    const chunks = [];
    req.on("data", (d) => chunks.push(d));
    req.on("end", () => {
      const raw = Buffer.concat(chunks).toString("utf8");
      let body = null;
      try { body = JSON.parse(raw); } catch { /* recorded as null */ }
      events.push({ ts: Date.now(), method: req.method, path: req.url,
                    ctype: req.headers["content-type"] ?? "", bytes: raw.length, body });
      res.writeHead(204); res.end();
    });
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve({ events, close: () => new Promise((r) => server.close(r)) }));
  });
}

export const sessionOf = (b) => (b && (b.session_id || b.conversation_id)) || "";

// Compare recorded posts with expect.json. Returns {failures, soft, seen}.
// Each failure: {id, msg}. id is stable (used for the issue fingerprint).
export function check(agent, events, exp = EXPECT[agent]) {
  const failures = [], soft = [];
  const posts = events.filter((e) => PATH_RE.test(e.path) && e.path.match(PATH_RE)[1] === agent);
  for (const e of events) {
    if (!PATH_RE.test(e.path)) failures.push({ id: `bad-path`, msg: `unexpected request ${e.method} ${e.path}` });
  }
  for (const e of posts) {
    const ev = e.body?.hook_event_name ?? "?";
    if (e.method !== "POST") failures.push({ id: `method:${ev}`, msg: `${ev}: method ${e.method}, want POST` });
    if (!/^application\/json/.test(e.ctype)) failures.push({ id: `ctype:${ev}`, msg: `${ev}: Content-Type '${e.ctype}', want application/json` });
    if (e.body === null) failures.push({ id: `json:${ev}`, msg: `${e.path}: body is not JSON` });
  }
  const seen = posts.map((e) => ({ state: e.path.match(PATH_RE)[2], event: e.body?.hook_event_name ?? null,
                                   session: sessionOf(e.body) }));
  const find = (x) => seen.filter((s) => s.state === x.state && (s.event === null || s.event === x.event));
  for (const [list, out] of [[exp.required, failures], [exp.soft, soft]]) {
    for (const x of list) {
      const hits = find(x);
      if (!hits.length) out.push({ id: `missing:${x.event}`, msg: `${x.event} -> ${x.state}: never posted` });
      else if (!hits.some((h) => h.session)) out.push({ id: `nosession:${x.event}`, msg: `${x.event}: posted without session_id/conversation_id` });
      else if (hits.some((h) => h.event === null)) out.push({ id: `noeventname:${x.event}`, msg: `${x.event}: body has no hook_event_name (matched by state only)` });
    }
  }
  const ids = new Set(seen.map((s) => s.session).filter(Boolean));
  if (ids.size > 1) soft.push({ id: "sessions", msg: `posts carry ${ids.size} different session ids` });
  return { failures: dedupe(failures), soft: dedupe(soft), seen };
}
const dedupe = (l) => [...new Map(l.map((f) => [f.id, f])).values()];

// D-022: ~/.claude/sessions/<pid>.json must keep sessionId, pid, status busy|idle|waiting.
export function readClaudeSessions(home) {
  const dir = join(home, ".claude", "sessions");
  if (!existsSync(dir)) return [];
  const out = [];
  for (const f of readdirSync(dir)) {
    if (!f.endsWith(".json")) continue;
    try { out.push({ file: f, json: JSON.parse(readFileSync(join(dir, f), "utf8")) }); } catch { /* mid-write */ }
  }
  return out;
}
export function checkClaudeSessions(snaps, hookSession) {
  const failures = [];
  if (!snaps.length) return [{ id: "sessions-file:missing", msg: "~/.claude/sessions/<pid>.json never appeared during the run" }];
  const good = snaps.filter(({ file, json: j }) => typeof j.sessionId === "string" && Number.isInteger(j.pid)
                                                   && file === `${j.pid}.json` && typeof j.status === "string");
  if (!good.length) {
    failures.push({ id: "sessions-file:fields", msg: `sessions file lacks sessionId/pid/status: ${JSON.stringify(snaps.at(-1).json).slice(0, 300)}` });
    return failures;
  }
  const statuses = [...new Set(good.map((s) => s.json.status))];
  const unknown = statuses.filter((s) => !["busy", "idle", "waiting"].includes(s));
  if (unknown.length) failures.push({ id: "sessions-file:status", msg: `unknown status value(s): ${unknown.join(", ")}` });
  if (hookSession && !good.some((s) => s.json.sessionId === hookSession))
    failures.push({ id: "sessions-file:id", msg: `sessionId in the file never matched the hook session_id ${hookSession}` });
  return failures;
}

// Same break -> same fingerprint (no duplicate issues). Version is NOT part of it.
export const fingerprint = (agent, failures) =>
  createHash("sha1").update(agent + "|" + failures.map((f) => f.id).sort().join(",")).digest("hex").slice(0, 12);
