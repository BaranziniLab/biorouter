# Reliable Slack posting from the agent

> **What this is.** An options memo comparing three ways to give the agent reliable Slack posting — incoming webhook, a Slack MCP server with a bot token, and a user token or the Slack CLI — ranked by how much setup each asks of the user, with a recommendation.
> **Status:** Current — an open investigation, first written 2026-06-20. **Nothing described here is implemented.** No `slack_post` tool and no Slack extension exist in the tree; the recommendation has not been accepted or rejected, so treat it as a decision still to be made.
> **Audience:** maintainers deciding how Biorouter should integrate with Slack.

UI automation of Slack is inherently brittle: Slack exposes almost no AppleScript API, and its Electron UI shifts between releases. The robust path is Slack's HTTP API. Three routes wire that into Biorouter, and they differ far more in setup burden than in capability, so this memo ranks them by setup cost (lowest first) and then says which one that argues for.

## What Biorouter already provides

Two existing facts shape every option below:

- Developer's `shell` can call a user-authorized HTTP API. Web & Documents' `web_scrape` is GET-only. Native Computer Use is a separate application-control capability with its own task grant.
- External MCP servers can be added via **Settings → Extensions → Add custom extension** (type: Standard IO or Streamable HTTP), with secrets stored in the OS Keychain (`env_keys`). The repo already anticipates a `slack-mcp` stdio extension keyed on `SLACK_TOKEN` — see `crates/biorouter-cli/src/workflows/secret_discovery.rs`.

---

## Option A — Incoming webhook (simplest; works with zero new code)

**What it is:** a Slack "Incoming Webhook" is one URL tied to one channel. POST `{"text":"..."}` to it and the message appears in that channel. Send-only, one channel.

**Biorouter support today:** already works — the agent can post via `developer__shell` after the user authorizes sending:

```bash
curl -s -X POST -H 'Content-type: application/json' -d '{"text":"<msg>"}' <WEBHOOK_URL>
```

No code change required. Optionally, the URL could be stored so the agent does not need it pasted each time — see [Optional polish](#optional-polish) below.

**One-time user setup (~2–3 min, no scopes, no bot):**

1. Go to <https://api.slack.com/apps> → **Create New App** → **From scratch** → pick the target workspace.
2. **Incoming Webhooks** → toggle **On**.
3. **Add New Webhook to Workspace** → choose the destination channel → **Allow**.
4. Copy the URL (`https://hooks.slack.com/services/T…/B…/…`).
5. Hand it to Biorouter (paste it in the request, or store it once — see below).

**Good for:** announcing a report in a known channel.

**Limits:** one channel per URL; send-only, with no reading, searching, or threads; the channel cannot be picked dynamically, so covering several channels means one webhook per channel.

---

## Option B — Slack MCP server with a bot token (most capable; more setup)

**What it is:** add an external Slack MCP server — a small program that wraps Slack's Web API. The agent then gets real tools: list channels, post to **any** channel, read history, search, reply in threads, and so on.

**Biorouter support:** add it in **Settings → Extensions → Add custom extension** (type **Standard IO**), set the command to a Slack MCP server, and store the token as an env secret. No Biorouter code change — it is a configuration step. Requires `npx` (Node) or `uvx` (uv) on the machine to launch the server.

**One-time user setup (~10–15 min):**

1. Create a Slack app (From scratch) in the workspace, as in Option A.
2. **OAuth & Permissions → Bot Token Scopes**: add `chat:write` (post), plus `channels:read` and `channels:history`; add `groups:read` and `groups:history` for private channels, and `search:read` to search.
3. **Install to Workspace** → copy the **Bot User OAuth Token** (`xoxb-…`).
4. **Invite the bot** to each channel it should use (`/invite @YourBot`).
5. In Biorouter, add the Slack MCP extension: the command, plus the `xoxb-…` token pasted as a secret env var. Some servers also want the workspace or team ID.

**Good for:** flexible, robust posting and reading across channels with no UI flakiness.

**Cost:** scopes, app install, inviting the bot, a dependency on `npx`/`uvx`, and trusting a third-party MCP server.

---

## Option C — User token or the Slack CLI

**What it is:** authenticate as a *user* rather than a bot, either through user-token OAuth or through the Slack CLI.

**Setup:** more involved than either option above — user-token OAuth carries its own scope grants and consent flow, and the Slack CLI is a further tool to install and keep authenticated.

**Assessment:** not recommended given the preference for simple setup. It is listed only for completeness; this memo did not evaluate it in the depth given to Options A and B.

---

## Recommendation

- For the originating need — posting an announcement or message to a single lab channel — **Option A (incoming webhook)**: roughly 2–3 minutes of setup, no scopes, no bot to invite, and it works with the tooling that already ships, so there is no implementation and nothing to maintain.
- If posting to **multiple channels** or **reading and searching** Slack from the agent becomes a requirement, move to **Option B**.

## Optional polish

This would need implementing; it is not implemented today.

To make Option A first-class instead of the agent shelling out `curl`, Biorouter could add a small `slack_post` tool, or a documented config key such as `SLACK_WEBHOOK_URL`, so the agent posts with a clean tool call and the URL is stored once in the Keychain. This is a small, self-contained change that was deliberately left undone pending a decision on this memo.

## Related documentation

- [Extensions, skills, and MCP agents](extensions-and-skills-guide.md) — how to actually add the custom stdio extension Option B depends on
- [Web & Documents](built-in/web-documents.md) — its GET-only URL tool cannot post messages
- [Secret storage](../security/secret-storage.md) — how `env_keys` secrets such as `SLACK_TOKEN` reach the OS Keychain
- [Computer Use multi-app orchestration run](../history/computer-controller-hardening/multi-app-orchestration-run.md) — the hardening run that exercised Slack via UI automation, showing the brittleness this memo proposes routing around
