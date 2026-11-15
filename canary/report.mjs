// Turns canary results into a job summary and GitHub issues (one open issue per agent).
//   node canary/report.mjs <results dir>      env: GH_TOKEN, GITHUB_REPOSITORY, RUN_URL, DRY_RUN=1
// fail/error -> open an issue labelled `canary` (or comment once on the agent's open issue when the
// break or the version is new). pass -> comment and close the agent's open issue. skip -> nothing.
import { readFileSync, readdirSync, appendFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fingerprint } from "./lib.mjs";

const dir = process.argv[2] || "canary-out";
const DRY = !!process.env.DRY_RUN;
const RUN_URL = process.env.RUN_URL || "";
const NAMES = { claude: "Claude Code", codex: "Codex CLI", cursor: "Cursor CLI" };

const files = [];
(function walk(d) {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    if (e.isDirectory()) walk(join(d, e.name));
    else if (/^(claude|codex|cursor)\.json$/.test(e.name)) files.push(join(d, e.name));
  }
})(dir);
const results = files.map((f) => JSON.parse(readFileSync(f, "utf8")));

function gh(args, input) {
  if (DRY) { console.log("DRY gh " + args.join(" ")); return "[]"; }
  const r = spawnSync("gh", args, { encoding: "utf8", input });
  if (r.status !== 0) throw new Error(`gh ${args.join(" ")}: ${r.stderr}`);
  return r.stdout;
}

const icon = { pass: "✅", fail: "❌", error: "⚠️", skip: "⏭️" };
let md = "## Agent canary\n\n| Agent | Version | Result | Details |\n|---|---|---|---|\n";
for (const r of results) {
  const det = [...r.failures.map((f) => "FAIL " + f.msg), ...r.soft.map((f) => "soft: " + f.msg), r.note].filter(Boolean);
  md += `| ${NAMES[r.agent]} | ${r.version || "-"} | ${icon[r.status]} ${r.status}${r.break_hook ? ` (break test: ${r.break_hook})` : ""} | ${det.join("<br>") || "-"} |\n`;
}
md += "\nNot covered headless: " + results.flatMap((r) => r.not_covered.map((n) => `${NAMES[r.agent]}: ${n}`)).join("; ") + "\n";
console.log(md);
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, md);

gh(["label", "create", "canary", "--color", "B60205", "--description", "Agent canary found a break", "--force"]);
const open = JSON.parse(gh(["issue", "list", "--label", "canary", "--state", "open", "--json", "number,title,body", "--limit", "100"]));

for (const r of results) {
  if (r.status === "skip") continue;
  // Break tests get their own issue key so they never touch a real one.
  const keyName = r.break_hook ? `${r.agent}-breaktest` : r.agent;
  const marker = `<!-- canary-agent:${keyName} -->`;
  const issue = open.find((i) => i.body?.includes(marker));
  if (r.status === "pass") {
    if (issue) {
      gh(["issue", "comment", String(issue.number), "--body", `Green again on ${NAMES[r.agent]} ${r.version}. ${RUN_URL}`]);
      gh(["issue", "close", String(issue.number)]);
    }
    continue;
  }
  const fails = r.status === "error" ? [{ id: "error", msg: r.note }] : r.failures;
  const fp = fingerprint(r.agent, fails);
  const fpMarker = `<!-- canary-fp:${fp} v:${r.version} -->`;
  const text = [
    `**${NAMES[r.agent]} ${r.version}**: ${r.status === "error" ? "the canary could not run it" : "our integration broke"}.`,
    "", ...fails.map((f) => `- ${f.msg}`),
    ...(r.soft.length ? ["", "Also (not alerting):", ...r.soft.map((f) => `- ${f.msg}`)] : []),
    "", `Posts seen: ${r.seen.map((s) => `${s.event ?? "?"}→${s.state}`).join(", ") || "none"}`,
    r.break_hook ? `\nThis is a deliberate break test (BREAK_HOOK=${r.break_hook}); close it by hand.` : "",
    "", `Run: ${RUN_URL}`, "", fpMarker,
  ].join("\n");
  if (!issue) {
    const title = `${r.break_hook ? "[break test] " : ""}Canary: ${NAMES[r.agent]} ${r.version} ${r.status === "error" ? "could not run" : "broke " + fails.map((f) => f.id).join(", ")}`.slice(0, 200);
    gh(["issue", "create", "--title", title, "--label", "canary", "--body-file", "-"], text + "\n" + marker);
    continue;
  }
  // Same break, same version -> stay quiet. New break or new version -> one comment.
  const comments = DRY ? "" : gh(["issue", "view", String(issue.number), "--json", "comments", "--jq", ".comments[].body"]);
  if ((issue.body + comments).includes(fpMarker)) { console.log(`#${issue.number}: same break already reported`); continue; }
  gh(["issue", "comment", String(issue.number), "--body-file", "-"], text);
}
