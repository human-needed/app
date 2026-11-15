// Agent-compatibility canary, one agent per call (pm/contracts/testing.md §1).
//   node canary/run.mjs <claude|codex|cursor>
// Env: PLUGINS_DIR (checkout of human-needed/plugins main, required), OUT_DIR (default canary-out),
//      BREAK_HOOK (optional: drop this hook event from our integration to prove a break is caught),
//      CANARY_MODEL_<AGENT> (optional model override), agent keys (see SECRETS below).
// Installs our integration, runs one tiny headless prompt against a recording hub on
// 127.0.0.1:24816, checks expect.json. Writes OUT_DIR/<agent>.json; exit 1 on fail/error.
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, cpSync, existsSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join } from "node:path";
import { startHub, check, readClaudeSessions, checkClaudeSessions, sessionOf, EXPECT } from "./lib.mjs";

export const SECRETS = {
  claude: ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
  codex: ["OPENAI_API_KEY"],
  cursor: ["CURSOR_API_KEY"],
};
const agent = process.argv[2];
if (!EXPECT[agent]) { console.error("usage: run.mjs <claude|codex|cursor>"); process.exit(2); }
const PLUGINS = process.env.PLUGINS_DIR;
const OUT = process.env.OUT_DIR || "canary-out";
const BREAK = process.env.BREAK_HOOK || "";
const MODEL = process.env[`CANARY_MODEL_${agent.toUpperCase()}`] || "";
const HOME = homedir();
mkdirSync(OUT, { recursive: true });
const logFile = join(OUT, `${agent}.log`);
const log = (s) => { console.log(s); writeFileSync(logFile, s + "\n", { flag: "a" }); };
const result = { agent, version: "", status: "", break_hook: BREAK || null, failures: [], soft: [],
                 not_covered: EXPECT[agent].not_covered, seen: [], note: "" };
const finish = (status, note = "") => {
  result.status = status; if (note) result.note = note;
  writeFileSync(join(OUT, `${agent}.json`), JSON.stringify(result, null, 2));
  log(`RESULT ${agent} ${result.version} ${status}${note ? " - " + note : ""}`);
  for (const f of result.failures) log(`  FAIL ${f.msg}`);
  for (const f of result.soft) log(`  soft ${f.msg}`);
  process.exit(status === "fail" || status === "error" ? 1 : 0);
};

const key = SECRETS[agent].find((k) => process.env[k]);
if (!key) finish("skip", `no secret set (${SECRETS[agent].join(" or ")})`);
if (!PLUGINS || !existsSync(PLUGINS)) finish("error", "PLUGINS_DIR not set or missing");

const sh = (cmd, args, opts = {}) => {
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 120_000, ...opts });
  log(`$ ${cmd} ${args.join(" ")} -> ${r.status}${r.error ? " " + r.error.message : ""}`);
  if (r.stdout?.trim()) log(r.stdout.trim().slice(-2000));
  if (r.stderr?.trim()) log(r.stderr.trim().slice(-2000));
  return r;
};
const must = (r, what) => { if (r.status !== 0) finish("error", `${what} failed (exit ${r.status})`); return r; };

// Copy of the plugins checkout with BREAK_HOOK removed from <rel>.
function pluginsCopy(rel) {
  const dir = mkdtempSync(join(tmpdir(), "hn-plugins-"));
  cpSync(PLUGINS, dir, { recursive: true });
  if (BREAK) {
    const f = join(dir, rel), j = JSON.parse(readFileSync(f, "utf8"));
    if (!(BREAK in j.hooks)) finish("error", `BREAK_HOOK ${BREAK} not in ${rel}`);
    delete j.hooks[BREAK];
    writeFileSync(f, JSON.stringify(j, null, 2));
    log(`BREAK: removed ${BREAK} from ${rel}`);
  }
  return dir;
}

// Run the agent; resolve {code, ms}. Kills it after timeoutMs.
function runAgent(cmd, args, { env = {}, cwd, input, timeoutMs = 240_000, onTick } = {}) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    log(`$ ${cmd} ${args.map((a) => (a.includes(" ") ? JSON.stringify(a) : a)).join(" ")}`);
    const p = spawn(cmd, args, { cwd, env: { ...process.env, ...env }, stdio: ["pipe", "pipe", "pipe"] });
    let out = "";
    p.stdout.on("data", (d) => (out += d)); p.stderr.on("data", (d) => (out += d));
    p.stdin.end(input ?? "");
    const tick = onTick && setInterval(() => onTick(p.pid), 250);
    const kill = setTimeout(() => { log("TIMEOUT, killing agent"); p.kill("SIGTERM"); setTimeout(() => p.kill("SIGKILL"), 5000); }, timeoutMs);
    p.on("error", (e) => { clearTimeout(kill); clearInterval(tick); log(String(e)); resolve({ code: -1, ms: Date.now() - t0 }); });
    p.on("close", (code) => {
      clearTimeout(kill); clearInterval(tick);
      log(out.trim().slice(-4000));
      resolve({ code, ms: Date.now() - t0, timedOut: Date.now() - t0 >= timeoutMs });
    });
  });
}

const hub = await startHub().catch((e) => finish("error", `cannot listen on 127.0.0.1:24816: ${e.message}`));
const ws = mkdtempSync(join(tmpdir(), "hn-canary-ws-"));
spawnSync("git", ["init", "-q"], { cwd: ws });
const extraFailures = [];
let run;

if (agent === "claude") {
  const env = { DISABLE_AUTOUPDATER: "1", DISABLE_TELEMETRY: "1", DISABLE_ERROR_REPORTING: "1",
                NO_PROXY: "127.0.0.1,localhost" };
  result.version = must(sh("claude", ["--version"]), "claude --version").stdout.trim();
  // Install exactly as the app does (integrations/manifest.json): marketplace from GitHub.
  const source = BREAK ? pluginsCopy("plugins/claude/hooks/hooks.json") : "human-needed/plugins";
  must(sh("claude", ["plugin", "marketplace", "add", source], { env: { ...process.env, ...env } }), "claude plugin marketplace add");
  must(sh("claude", ["plugin", "install", "humanneeded@humanneeded", "--scope", "user"], { env: { ...process.env, ...env } }), "claude plugin install");
  // Manifest "installed" detection: ~/.claude/settings.json /enabledPlugins/humanneeded@humanneeded == true
  try {
    const s = JSON.parse(readFileSync(join(HOME, ".claude", "settings.json"), "utf8"));
    if (s.enabledPlugins?.["humanneeded@humanneeded"] !== true)
      extraFailures.push({ id: "install:detect", msg: "settings.json /enabledPlugins/humanneeded@humanneeded is not true after install" });
  } catch (e) { extraFailures.push({ id: "install:detect", msg: `~/.claude/settings.json unreadable after install: ${e.message}` }); }

  const snaps = new Map();
  const prompt = "This is an automated hook test. Do exactly this: 1) Use the Bash tool to run: echo hn-canary " +
                 "2) Use the Write tool to create the file hn-needs.txt containing ok (it may be refused; that is fine). " +
                 "3) Reply with the single word DONE.";
  const args = ["-p", prompt, "--allowedTools", "Bash(echo hn-canary)", "--max-turns", "6"];
  args.push("--model", MODEL || "haiku");
  run = await runAgent("claude", args, { cwd: ws, env, onTick: () => {
    for (const s of readClaudeSessions(HOME)) snaps.set(JSON.stringify(s), s);
  } });
  await new Promise((r) => setTimeout(r, 1500));
  const hookSession = hub.events.map((e) => sessionOf(e.body)).find(Boolean);
  result.sessions_file = [...snaps.values()].slice(-3).map((s) => s.json);
  extraFailures.push(...checkClaudeSessions([...snaps.values()], hookSession));
}

if (agent === "codex") {
  result.version = must(sh("codex", ["--version"]), "codex --version").stdout.trim();
  must(sh("codex", ["login", "--with-api-key"], { input: process.env.OPENAI_API_KEY }), "codex login");
  const src = pluginsCopy("plugins/codex/hooks.json");
  mkdirSync(join(HOME, ".codex"), { recursive: true });
  cpSync(join(src, "plugins/codex/hooks.json"), join(HOME, ".codex", "hooks.json"));
  const prompt = "This is an automated hook test. Do exactly this: 1) run the shell command: echo hn-canary " +
                 "2) run the shell command: curl -sI https://example.com (it needs approval; if refused that is fine) " +
                 "3) reply with the single word DONE.";
  const args = ["exec", "--dangerously-bypass-hook-trust", "--skip-git-repo-check", "-s", "workspace-write",
                "-c", "approval_policy=\"on-request\""];
  if (MODEL) args.push("-m", MODEL);
  args.push(prompt);
  run = await runAgent("codex", args, { cwd: ws });
  await new Promise((r) => setTimeout(r, 2500)); // async curl hooks
}

if (agent === "cursor") {
  const bin = ["agent", "cursor-agent"].find((b) => spawnSync(b, ["--version"]).status === 0);
  if (!bin) finish("error", "Cursor CLI (agent / cursor-agent) not on PATH after install");
  result.version = sh(bin, ["--version"]).stdout.trim();
  const rel = "plugins/cursor/hooks.json";
  const src = pluginsCopy(rel);
  mkdirSync(join(HOME, ".cursor"), { recursive: true });
  cpSync(join(src, rel), join(HOME, ".cursor", "hooks.json"));
  const help = spawnSync(bin, ["--help"], { encoding: "utf8" }).stdout ?? "";
  const args = ["-p", "--force", "--output-format", "text"];
  if (/--trust\b/.test(help)) args.push("--trust");
  if (MODEL) args.push("--model", MODEL);
  args.push("This is an automated hook test. Run the shell command: echo hn-canary, then reply with the single word DONE.");
  run = await runAgent(bin, args, { cwd: ws });
  await new Promise((r) => setTimeout(r, 2500));
}

await hub.close();
writeFileSync(join(OUT, `${agent}.events.jsonl`), hub.events.map((e) => JSON.stringify(e)).join("\n") + "\n");
const c = check(agent, hub.events);
result.seen = c.seen;
result.failures = [...c.failures, ...extraFailures];
result.soft = c.soft;
if (run.code !== 0 && !hub.events.length) finish("error", `agent exited ${run.code}${run.timedOut ? " (timeout)" : ""} and posted nothing: auth, install or CLI change? See log`);
if (run.code !== 0) result.note = `agent exited ${run.code}${run.timedOut ? " (timeout)" : ""}`;
finish(result.failures.length ? "fail" : "pass", result.note);
