// SPDX-License-Identifier: GPL-3.0-or-later
// Automated part of the release check (pm/RELEASE.md R-09). Node >= 18, no packages.
//
//   node qa/release-check.mjs release [tag]   release assets: names, SHA256SUMS, latest.json,
//                                             updater signatures against the app's public key
//   node qa/release-check.mjs installed [--expect <version>] [--preview]
//                                             the installed, running app: hub, guards, agents,
//                                             plugins, login item, updater. --preview also runs
//                                             a Needs preview (lights the screen and the puck!)
//   node qa/release-check.mjs removed         after uninstall: nothing left but settings
//
// Exit 1 when any check fails. "info" lines never fail.
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { homedir, platform } from "node:os";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const REPO = "human-needed/app";
const HUB = "http://127.0.0.1:24816";
const WIN = platform() === "win32", MAC = platform() === "darwin";
const HOME = homedir();
const DATA = process.env.HN_DATA_DIR ||
  (WIN ? join(process.env.LOCALAPPDATA || join(HOME, "AppData/Local"), "HumanNeeded")
       : MAC ? join(HOME, "Library/Application Support/HumanNeeded") : join(HOME, ".local/share/humanneeded"));

let failed = 0;
const ok = (c, name, extra = "") => {
  console.log(`${c ? "  ok  " : "  FAIL"} ${name}${extra ? "  -- " + extra : ""}`);
  if (!c) failed++;
  return c;
};
const info = (name, v) => console.log(`  info ${name}: ${v}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sha256 = (b) => createHash("sha256").update(b).digest("hex");
async function get(url, opts = {}) {
  const r = await fetch(url, { redirect: "follow", ...opts });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r;
}

// ---- minisign (Tauri updater) -------------------------------------------------------------
// Public key / signature are base64 of a minisign text file; line 2 is base64 of
// alg(2) + key id(8) + key(32) / alg(2) + key id(8) + signature(64). "ED" = BLAKE2b-512 prehash.
const mlines = (b64) => Buffer.from(b64, "base64").toString("utf8").split("\n");
export function minisignVerify(pubB64, sigB64, data) {
  const pk = Buffer.from(mlines(pubB64)[1], "base64");
  const lines = mlines(sigB64);
  const sig = Buffer.from(lines[1], "base64");
  const trusted = lines[2].replace(/^trusted comment: /, "");
  const global = Buffer.from(lines[3], "base64");
  if (!pk.subarray(2, 10).equals(sig.subarray(2, 10))) return { ok: false, why: "key id differs" };
  const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), pk.subarray(10, 42)]),
                                format: "der", type: "spki" });
  const alg = sig.subarray(0, 2).toString();
  const msg = alg === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, msg, key, sig.subarray(10, 74))) return { ok: false, why: "file signature invalid" };
  if (!verify(null, Buffer.concat([sig.subarray(10, 74), Buffer.from(trusted)]), key, global))
    return { ok: false, why: "trusted comment signature invalid" };
  return { ok: true, trusted };
}

// ---- release ------------------------------------------------------------------------------
async function release(tag) {
  const rel = await (await get(`https://api.github.com/repos/${REPO}/releases/${tag ? "tags/" + tag : "latest"}`,
    { headers: { accept: "application/vnd.github+json", ...(process.env.GH_TOKEN ? { authorization: `Bearer ${process.env.GH_TOKEN}` } : {}) } })).json();
  tag = rel.tag_name;
  const version = tag.replace(/^v/, "");
  const stable = !version.includes("-");
  console.log(`release ${tag} (${stable ? "stable" : "beta"})`);
  ok(stable ? !/test build/i.test(rel.name) : true, "stable release title has no '(test build)'", rel.name);
  ok(!rel.draft, "not a draft");
  ok(stable ? !rel.prerelease : true, "stable release is not marked pre-release");
  const assets = Object.fromEntries(rel.assets.map((a) => [a.name, a.browser_download_url]));
  const want = ["HumanNeeded-windows-x64-setup.exe", "HumanNeeded-windows-x64-setup.exe.sig",
                "HumanNeeded-macos-universal.dmg", "HumanNeeded-macos-universal.app.tar.gz",
                "HumanNeeded-macos-universal.app.tar.gz.sig", "latest.json", "SHA256SUMS.txt"];
  for (const n of want) ok(n in assets, `asset ${n}`);
  // The site links to releases/latest/download/<stable name> (STATE 11-14).
  if (!tag || rel.tag_name === (await (await get(`https://api.github.com/repos/${REPO}/releases/latest`)).json()).tag_name) {
    for (const n of ["HumanNeeded-windows-x64-setup.exe", "HumanNeeded-macos-universal.dmg"]) {
      const r = await fetch(`https://github.com/${REPO}/releases/latest/download/${n}`, { method: "HEAD", redirect: "follow" });
      ok(r.ok, `site download link works: releases/latest/download/${n}`, String(r.status));
    }
  } else info("latest", "this tag is not the latest release; site links not checked");

  const files = {};
  const sums = (await (await get(assets["SHA256SUMS.txt"])).text()).trim().split(/\r?\n/)
    .map((l) => l.trim().split(/\s+\*?/)).filter((p) => p.length === 2);
  for (const [hash, name] of sums) {
    if (!assets[name]) { ok(false, `SHA256SUMS lists ${name}`, "no such asset"); continue; }
    files[name] = Buffer.from(await (await get(assets[name])).arrayBuffer());
    ok(sha256(files[name]) === hash.toLowerCase(), `sha256 ${name}`);
  }
  for (const n of ["HumanNeeded-windows-x64-setup.exe", "HumanNeeded-macos-universal.dmg"])
    ok(sums.some(([, name]) => name === n), `SHA256SUMS covers ${n}`);

  const latest = JSON.parse((await (await get(assets["latest.json"])).text()));
  ok(latest.version === version, "latest.json version = tag", latest.version);
  const conf = JSON.parse(await (await get(`https://raw.githubusercontent.com/${REPO}/${tag}/app/tauri.conf.json`)).text());
  ok(conf.version === version, "tauri.conf.json version = tag", conf.version);
  const pub = conf.plugins.updater.pubkey;
  for (const p of ["windows-x86_64", "darwin-aarch64", "darwin-x86_64"]) {
    const e = latest.platforms?.[p];
    if (!ok(!!e, `latest.json has ${p}`)) continue;
    ok(e.url.includes(`/download/${tag}/`), `${p} url points at ${tag}`, e.url);
    const name = e.url.split("/").pop();
    const data = files[name] ?? Buffer.from(await (await get(e.url)).arrayBuffer());
    files[name] = data;
    const v = minisignVerify(pub, e.signature, data);
    ok(v.ok, `${p} updater signature valid for the app's public key`, v.why ?? "");
    const bad = Buffer.from(data); bad[bad.length >> 1] ^= 1;
    ok(!minisignVerify(pub, e.signature, bad).ok, `${p} a changed file is rejected (checker sanity)`);
    if (v.ok) ok(v.trusted.includes(`version:${version}`), `${p} signed version is ${version}`, v.trusted);
  }
}

// ---- installed app ------------------------------------------------------------------------
async function raw(path, { method = "GET", headers = {}, body } = {}) {
  try {
    const r = await fetch(HUB + path, { method, headers, body });
    return { status: r.status, text: await r.text() };
  } catch (e) { return { status: 0, text: String(e.cause?.code || e) }; }
}
const readJson = (f) => { try { return JSON.parse(readFileSync(f, "utf8").replace(/^﻿/, "")); } catch { return null; } };
const tail = (f, n = 4000) => { try { return readFileSync(f, "utf8").slice(-n * 100); } catch { return ""; } };

async function installed(expect, preview) {
  console.log(`installed app (data: ${DATA})`);
  let h;
  for (let i = 0; i < 40 && !(h = await raw("/v1/health")).status; i++) await sleep(500);
  if (!ok(h.status === 200, "hub answers /v1/health", `${h.status} ${h.text}`)) return;
  const health = JSON.parse(h.text);
  ok(health.app === "humanneeded", "it is the HumanNeeded app");
  if (expect) ok(health.version === expect, `version ${expect}`, health.version); else info("version", health.version);
  const st = JSON.parse((await raw("/v1/status")).text);
  ok(st.ok === true && st.http?.listening && st.http.port === 24816, "status ok, hub on 24816");
  ok(!st.http?.error, "no hub error", st.http?.error ?? "");
  ok(st.proxy == null, "no proxy warning (D-049)", JSON.stringify(st.proxy));
  for (const a of st.agents ?? []) info(`agent ${a.id}`, `${a.state}${a.enabled ? "" : " (switched off)"}`);
  info("puck", st.puck?.connected ? `connected, fw ${st.puck.fw}, ${st.puck.board}` : "not connected");
  info("cloud link", st.cloud?.state);

  // Guards (app-events §1), all without touching the puck.
  const j = { "content-type": "application/json" };
  ok((await raw("/v1/status", { headers: { origin: "https://evil.example" } })).status === 403, "Origin header -> 403");
  ok((await raw("/v1/event/qa/working")).status === 405, "GET on an event -> 405");
  ok((await raw("/v1/event/qa/working", { method: "POST", headers: { "content-type": "text/plain" }, body: "x" })).status === 405, "non-JSON POST -> 405");
  ok((await raw("/v1/event/qa/busy", { method: "POST", headers: j, body: "{}" })).status === 400, "unknown state -> 400");
  // idle is kept but never shown, end removes it: the round trip lights nothing.
  const sid = "release-check-" + Date.now();
  ok((await raw("/v1/event/qa-check/idle", { method: "POST", headers: j, body: JSON.stringify({ session_id: sid }) })).status === 204, "event accepted (204)");
  ok((await raw("/v1/event/qa-check/end", { method: "POST", headers: j, body: JSON.stringify({ session_id: sid }) })).status === 204, "end accepted (204)");

  // Plugins / hooks the user set up (D-054: the user installs them).
  const cs = readJson(join(process.env.CLAUDE_CONFIG_DIR || join(HOME, ".claude"), "settings.json"));
  info("Claude plugin", cs?.enabledPlugins?.["humanneeded@humanneeded"] ? "enabled" : "not enabled");
  const cache = join(HOME, ".claude/plugins/cache/humanneeded/humanneeded");
  if (existsSync(cache)) info("Claude CLI plugin versions in cache", readdirSync(cache).join(", "));
  // Claude desktop app (Code tab, Cowork) keeps its plugins under <Claude data>/rpm/.
  const rpm = WIN ? join(process.env.APPDATA || "", "Claude/rpm") : join(HOME, "Library/Application Support/Claude/rpm");
  const found = [];
  (function walk(d, depth) {
    if (depth > 4 || !existsSync(d)) return;
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      const pj = readJson(join(d, e.name, ".claude-plugin/plugin.json"));
      if (pj?.name === "humanneeded") found.push(pj.version);
      else walk(join(d, e.name), depth + 1);
    }
  })(rpm, 0);
  info("Claude desktop plugin", found.length ? found.join(", ") : "not installed");
  if (found.length) ok(found.every((v) => !/-dev/.test(v)) || /-/.test(health.version), "desktop plugin is a stable version for a stable app (R-06)", found.join(", "));
  for (const [a, f] of [["Codex", ".codex/hooks.json"], ["Cursor", ".cursor/hooks.json"]]) {
    const t = existsSync(join(HOME, f)) ? readFileSync(join(HOME, f), "utf8") : "";
    info(`${a} hooks`, t.includes(`127.0.0.1:24816/v1/event/${a.toLowerCase()}/`) ? "installed" : "not installed");
  }

  info("starts at login", loginItem() ? "yes" : "no (on by default; the user may have turned it off)");

  // Updater: checks 10 s after start; log says what it found.
  const log = tail(join(DATA, "logs/app.log"));
  const up = log.split("\n").filter((l) => /update: /.test(l)).slice(-2);
  info("updater (last log lines)", up.join(" | ") || "none yet");
  ok(!/update: (?!.*(found|downloaded|installing)).*/.test(up.at(-1) ?? ""), "last updater line is not an error", up.at(-1) ?? "");

  if (preview) {
    const logf = join(DATA, "logs/app.log");
    const full = () => { try { return readFileSync(logf, "utf8"); } catch { return ""; } };
    const before = full().length;
    ok(JSON.parse((await raw("/v1/test/needs", { method: "POST", headers: j, body: "{}" })).text).ok === true, "Needs preview accepted");
    let seen = "";
    for (let i = 0; i < 150; i++) { // Needs with no limit previews up to 60 s (D-060)
      await sleep(500);
      seen = full().slice(before);
      if (/screen: off/.test(seen)) break;
    }
    ok(/screen: needs/.test(seen), "screen glow showed Needs", seen.split("\n").filter((l) => /screen:/.test(l)).join(" | "));
    ok(/screen: off/.test(seen), "screen glow ended");
    ok(!/sound(: stream error| \S+: )/.test(seen), "no sound error", seen.match(/^.*sound.*$/m)?.[0] ?? "");
  }
}

// Login item (D-027: on by default after the first run).
function loginItem() {
  if (WIN) {
    try { return /humanneeded/i.test(execFileSync("reg", ["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"], { encoding: "utf8" })); } catch { return false; }
  }
  const la = join(HOME, "Library/LaunchAgents");
  return MAC && existsSync(la) && readdirSync(la).some((f) => /humanneeded/i.test(f));
}

// ---- after uninstall ----------------------------------------------------------------------
async function removed() {
  console.log("after uninstall");
  ok((await raw("/v1/health")).status === 0, "nothing listens on 127.0.0.1:24816");
  if (WIN) {
    const q = (k) => { try { return execFileSync("reg", ["query", k], { encoding: "utf8", stdio: "pipe" }); } catch { return ""; } };
    ok(!q("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\HumanNeeded"), "uninstall entry gone");
    ok(!/humanneeded/i.test(q("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")), "login item gone");
    ok(!existsSync(join(DATA, "humanneeded.exe")), "app binary gone");
    let procs = "";
    try { procs = execFileSync("tasklist", ["/FI", "IMAGENAME eq humanneeded.exe"], { encoding: "utf8" }); } catch { /* */ }
    ok(!/humanneeded\.exe/i.test(procs), "no humanneeded.exe running");
    const lnk = [join(process.env.APPDATA || "", "Microsoft/Windows/Start Menu/Programs/HumanNeeded.lnk"),
                 join(process.env.APPDATA || "", "Microsoft/Windows/Start Menu/Programs/HumanNeeded")];
    ok(!lnk.some(existsSync), "Start menu entry gone");
  } else if (MAC) {
    ok(!existsSync("/Applications/HumanNeeded.app"), "/Applications/HumanNeeded.app gone");
    const la = join(HOME, "Library/LaunchAgents");
    const left = existsSync(la) ? readdirSync(la).filter((f) => /humanneeded/i.test(f)) : [];
    ok(!left.length, "no LaunchAgent left (else it fails at every login)", left.join(", "));
  }
  const left = existsSync(DATA) ? readdirSync(DATA) : [];
  const settings = new Set(["hn.sock", "agents.json", "alerts.json", "app-settings.json", "cloud.json", "glow.json", "seen", "logs"]);
  ok(left.every((f) => settings.has(f)), "only settings and logs left in the data folder", left.join(", "));
  info("settings kept", left.length ? left.join(", ") : "none (removed)");
}

const [mode, ...rest] = process.argv.slice(2);
const flag = (f) => rest.includes(f);
const val = (f) => (rest.includes(f) ? rest[rest.indexOf(f) + 1] : undefined);
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    if (mode === "release") await release(rest.find((a) => !a.startsWith("--")));
    else if (mode === "installed") await installed(val("--expect"), flag("--preview"));
    else if (mode === "removed") await removed();
    else if (mode === "first-run") { console.log("first run on a clean machine"); ok(loginItem(), "starts at login is on (D-027)"); }
    else { console.log("usage: release [tag] | installed [--expect v] [--preview] | removed"); process.exit(2); }
  } catch (e) { ok(false, "check crashed", e.message); }
  console.log(failed ? `\n${failed} FAILED` : "\nall ok");
  process.exit(failed ? 1 : 0);
}
