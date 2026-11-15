# Agent canary

Nightly check that Claude Code, Codex and Cursor still fire the hooks our integration
(`human-needed/plugins` main) relies on (pm/contracts/testing.md §1, D-042).
Workflow: `.github/workflows/canary.yml` (nightly 04:17 UTC + manual "Run workflow").

Per agent: install the latest CLI, install our integration as the app would, run one tiny
headless prompt against a recording hub on 127.0.0.1:24816, compare with `expect.json`.
Claude also: plugin install/detect commands, `~/.claude/sessions/<pid>.json` fields (D-022).

A break opens one GitHub issue labelled `canary` per agent (no duplicates; one comment when the
break or the version changes; closed automatically when green again). GitHub emails the watchers.

## Secrets (repo Settings > Secrets and variables > Actions)

| Secret | Agent | Missing → |
|---|---|---|
| `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`, uses the subscription) or `ANTHROPIC_API_KEY` | Claude Code | skipped |
| `OPENAI_API_KEY` | Codex CLI | skipped |
| `CURSOR_API_KEY` (Cursor dashboard > API keys) | Cursor CLI | skipped |

Optional variables `CANARY_MODEL_CLAUDE` (default `haiku`), `CANARY_MODEL_CODEX`, `CANARY_MODEL_CURSOR`.

## Prove it catches a break

Run workflow → `break_hook: Stop` (Cursor: `stop`). That agent fails and a `[break test]` issue opens.

## Local

```
PLUGINS_DIR=../plugins node canary/selftest.mjs     # no keys, no agents
PLUGINS_DIR=../plugins node canary/run.mjs claude   # needs the CLI + key; uses port 24816 (quit the app)
```
