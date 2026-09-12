# Reaching a private chat from a script

> **What this is.** How a program — a monitoring dashboard, a CI job, a shell script, an agent
> running in a terminal — reads or follows a **private** Biorouter conversation over the daemon's
> HTTP API, using the `X-Caller-Provider` header.
> **Status:** Current.
> **Audience:** operators and developers writing automation against a running `biorouterd`.

Biorouter classifies every conversation by the sensitivity of what it has touched. A conversation
that has run against a private model — one your institution hosts, or one running on the machine
itself — is marked private for the rest of its life, and the daemon refuses to hand it to a caller
whose own capability does not cover it. Holding the daemon's secret key is not enough, and that is
deliberate: [privacy tiers](../security/privacy-tiers.md) exist precisely so that a component with
the secret cannot launder a private transcript into a public model.

That rule has a consequence people meet as a bug. A script that is *already* running under a
private model — a dashboard watching a UCSF-hosted run, a CI job driving `versa_azure` — has the
capability the daemon is asking for, and gets a `403` anyway, because nothing on the request says
so. The fix is one header. This page is that header.

If you want the interface in a browser rather than the API, read
[Reaching Biorouter from a browser](browser-access.md) instead.

---

## The short version

```bash
curl -H "X-Secret-Key: $SECRET" \
     -H "X-Caller-Provider: versa_azure" \
     "http://127.0.0.1:$PORT/sessions/$SESSION_ID"
```

Without the second header that call returns `403`. With it, it returns the transcript — provided
`versa_azure` really is a private-tier provider on the daemon you are calling.

## What the header is

`X-Caller-Provider` states **the provider the calling program is itself running under**. It carries
a provider *name*, exactly as it is spelled in `config.yaml` — `versa_azure`, `versa_bedrock`,
`ollama`, `llamacpp` — and never a tier.

The daemon resolves that name against **its own** installed provider registry and takes the tier
from there. A caller does not get to say how sensitive it is; it says what it is running, and the
daemon decides what that means. The rule then applied is the one the whole feature states:

> the caller's capability must be at least the target conversation's classification.

That is the same rule the agent's tool surface applies to a conversation read, and the same rule the
bind gate applies when a session changes model. Reaching a private chat over HTTP is not a special
case with its own logic — it is that rule, stated on a request.

The header is read by
[`crates/biorouter-server/src/routes/session_reach.rs`](../../crates/biorouter-server/src/routes/session_reach.rs);
`biorouter session watch`, `send` and `attach` already send it, which is why those commands reach a
private chat from a terminal that can never prove a human is present.

## What the header is *not*

**It is not authentication, and it must not be described as one.** A caller that holds the daemon's
secret key can spell any installed provider's name here. That is not a hole this header opens — the
daemon has no principal at all, so such a caller already reaches every public session it can name,
which is the open residual tracked as
[issue #47](https://github.com/BaranziniLab/biorouter/issues/47). What the header buys is that the
gate can *express* the correct rule instead of the proxy it used to enforce (proof-of-a-human),
which got the answer backwards in both directions: a terminal running an institutional model was
refused, while the desktop app was admitted for the same chat while running a public one.

**It is not a way to raise or lower a tier.** Reaching a chat and *reclassifying* one are separate
decisions. Raising a session's classification, declassifying it, and binding a private model all
still require proof that the person at the keyboard acted (`X-User-Action`), and no header changes
that. A capability is a fact about a model; neither of those is a decision a model may make.

**It is not a per-request opt-out.** There is no header that turns the gate off. The only
machine-wide switch is the privacy master switch, which lives in its own record beside
`config.yaml` and is not an environment variable.

## The fail-safe fallback

Anything the daemon cannot positively resolve to a private provider is treated as **public** — the
refusing side:

| Header value | Resolves to | Result against a private chat |
|---|---|---|
| `versa_azure`, `versa_bedrock`, `ollama`, `llamacpp` | Private | `200` |
| `anthropic`, `openai`, `claude_code`, … | Public | `403` |
| `private` (a *tier*, not a provider) | Public | `403` |
| A name this install does not publish | Public | `403` |
| Empty or whitespace | Public | `403` |
| Header absent | Public | `403` |

Two consequences worth internalising. A **typo fails closed** — it does not error, it silently
resolves public and you get the same `403` as before, so check the spelling against
`GET /config/providers` before assuming the gate is broken. And every client written before the
header existed keeps working unchanged, because "absent" and "public" are the same answer.

One caveat: the tier resolved is the provider's **declared** tier, not a live instance's. An
`ollama` re-pointed off the machine with `OLLAMA_HOST` still declares Private. That residual is
inherited from how every other surface reasons about provider tiers, not introduced here.

## Recipes

Set these once:

```bash
export SECRET="$BIOROUTER_SERVER__SECRET_KEY"
export BASE="http://127.0.0.1:8791"
export SESSION="20260904_1"
export CALLER="versa_azure"
```

### Read the transcript as JSON

```bash
curl -s -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" "$BASE/sessions/$SESSION"
```

### Follow a run live over SSE

`GET /sessions/{id}/events` opens with a snapshot of the whole stored conversation and then tails
it, so a monitor attaching mid-run sees everything that came before:

```bash
curl -sN -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" "$BASE/sessions/$SESSION/events"
```

Frames arrive as `data: {...}` lines. `TurnStarted` opens a turn, `Message` carries streamed
content and token counts, `Finish` closes it, and `Error` reports a failure. A minimal monitor:

```bash
curl -sN -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" \
     "$BASE/sessions/$SESSION/events" \
| while IFS= read -r line; do
    case "$line" in
      data:*) printf '%s\n' "${line#data: }" | jq -r 'select(.type) | "\(.type) \(.turn_id // "")"' ;;
    esac
  done
```

### Drive the run as well as watch it

`POST /reply` is gated by the same rule, so a script that starts a turn in a private chat needs the
header too — not only the one that watches it:

```bash
curl -sN -H "Content-Type: application/json" \
        -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" \
        -X POST "$BASE/reply" --data @turn.json
```

### Export the transcript, or pull a diagnostics bundle

```bash
curl -s  -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" "$BASE/sessions/$SESSION/export"
curl -sL -H "X-Secret-Key: $SECRET" -H "X-Caller-Provider: $CALLER" "$BASE/diagnostics/$SESSION" -o bundle.zip
```

## Which routes honour the header

These consult the reach gate, so the header is what admits a private chat on each of them. Every
one of them resolves the target's tier **before** it touches the session, so a refusal cannot leak
"this chat is busy" or "no such extension" as a side channel.

| Route | What it reaches |
|---|---|
| `GET /sessions/{id}` | The transcript. |
| `GET /sessions/{id}/export` | The same transcript, as a download. |
| `GET /sessions/{id}/events` | The same transcript plus a live tail (SSE). |
| `GET /diagnostics/{id}` | The same transcript in a zip, with this session's logs. |
| `POST /reply` | Runs an agent turn, with tools, in the named session. |
| `POST /agent/continuation/recover` | Resumes a parked continuation. |
| `POST /agent/resume` · `restart` · `stop` | Lifecycle control of the session's agent. |
| `POST /agent/update_provider` | Rebinds the model (also needs `X-User-Action` to raise a tier). |
| `POST /agent/update_from_session` | Adopts another session's provider configuration. |
| `POST /agent/update_working_dir` | Repoints the session at a directory. |
| `POST /agent/add_extension` · `remove_extension` | Attaches or detaches tools. |
| `GET`/`POST /knowledge/active` | Reads or repoints the session's knowledge bases. |
| `POST /agent/cancel` · `POST /agent/continuation/abandon` · `recover` | Stops or settles the session's running turn — **on a daemon that holds no user-action key only**, such as `biorouter serve` or a hand-run `biorouterd` ([SD-11](serve-decisions.md#sd-11--stop-works-on-a-daemon-with-no-key-steering-does-not-and-a-subagents-tab-stays-the-persons)). There they admit exactly the callers `POST /agent/stop` admits, a subagent's session excepted. `POST /interrupt` is **not** among them: it takes the proof on either kind of daemon, because injecting text into a turn already running is the one thing no other route on a keyless daemon can do. |

## What the header does *not* cover

Being explicit about this is the point of listing it. These routes address or expose sessions and do
**not** consult the reach gate. Each is one of three things, and the difference matters:

**Gated by a different, deliberate instrument** — the header is not the mechanism here, and adding
it would be wrong:

| Route | What guards it instead |
|---|---|
| `POST /interrupt` | `X-User-Action` on **every** daemon — steering a turn that is already running is the user's decision, not a capability, and no other route on a keyless daemon reaches into a turn in flight. A keyless daemon's refusal says so in words rather than with an empty `403`. |
| `POST /agent/cancel`, `POST /agent/continuation/abandon` · `recover` | On a daemon that holds a user-action key — the desktop application's — `X-User-Action` and nothing else. A daemon that holds no key cannot check the proof at all, so these routes honour the header there instead (the table above). |
| `POST /sessions/{id}/declassify`, `POST /sessions/{id}/diverge`, `POST /sessions/{id}/edit_message` | `X-User-Action` — these change or copy a classification, which no model may decide. |
| `POST /agent/cross_affiliation_grant`, `POST /action-required/tool-confirmation` | `X-User-Action`, plus a decision-authority check on the resolving surface. |
| `POST /knowledge/bases/{id}/ingest-conversation` | Its own Gate G: capability is derived from the model named in the request body, and every selected conversation is checked against it before a transcript is rendered. |
| `POST /agent/call_tool` | Privacy Gate C at the extension-manager dispatch point, plus the uninspected-boundary refusals. |
| `POST /agent/read_resource` | Gate C's sibling at the extension-manager resource read. Like `call_tool` it has no caller identity, so it declares `CallCapability::public_enforced()` rather than sampling the named session: naming a private chat buys nothing, and a private extension is refused with `403`. |

**Ungated, and low-yield.** These name a session but return only its tool surface, not its contents:
`GET /agent/tools`, `GET /agent/callable_tool_count`, `GET /skills/catalog`, `POST /skills/refresh`.
They are listed as a measurement, not as a ruling — nothing in the source records a decision to
exempt them, so read this row as "not gated" rather than "deliberately not gated". `POST
/agent/read_resource` was on this list until 2026-09-09 and is now gated; the row above says how.

**Ungated, and a known residual.** These reach or describe a private session without the gate. None
returns a transcript, so none is the boundary this feature defends — but none is closed either, and
a reader should not infer from this page that the surface is complete:

| Route | What an ungated caller gets |
|---|---|
| `GET /sessions`, `GET /sessions/sidebar` | Every session on the machine — id, name, working directory and tier. Enumerates wholesale; recorded as an open residual in `session_reach.rs`. |
| `GET /sessions/running` | The ids of sessions with a turn in flight. |
| `GET /active_work` | Every running background job, subagent, detached turn and scheduled run — with `sessionId`, and a `title`/`detail` that carries the **shell command or task prompt**. This is content rather than metadata, and it is not named in `session_reach.rs`'s residual list. |
| `POST /active_work/{id}/cancel` | Cancels any of the above by its registry id. The id is not a session id, so the gate cannot be applied without a reverse lookup. |
| `GET /sessions/{id}/usage` | Per-model token counts for a named session, and a `200`/`404` that tells the caller whether the id exists. |
| `GET /sessions/{id}/extensions` | The session's enabled extension list. |
| `PUT /sessions/{id}/name`, `PUT /sessions/{id}/user_workflow_values`, `DELETE /sessions/{id}` | Renames, edits workflow values, or deletes the session. |
| `POST /skills/session` | Rewrites a session's per-chat skill overrides. |
| `GET /schedule/{id}/inspect`, `POST /schedule/{id}/run_now`, `POST /schedule/create` | Inspects or launches scheduled work that may run in a private session. |

The daemon has no principal, so none of this is a *tier* bypass in the strict sense — a caller
holding the secret is already inside. It is the same open problem as
[#47](https://github.com/BaranziniLab/biorouter/issues/47), and the list above is a snapshot of an
enumeration rather than a proof of completeness.

## Troubleshooting

**`403` with a long refusal that points here.** You sent no `X-Caller-Provider`, or a value that did
not resolve private. Check the spelling against `GET /config/providers`. The refusal names this page
and deliberately does not name the header: its reader is far more often a model than a person, and a
refused caller handed a header to add would read that as permission to retry, which is exactly what
the refusal exists to foreclose. The mechanism lives here, where the reader is you. The refusal is
also identical for "this chat is private" and "there is no such chat", deliberately — so a `403` is
not evidence the session exists.

**`403` saying the daemon was started without a user-action key.** A different state: this daemon
holds no key with which to verify a human, which is normal for `just run-server`, a hand-run
`biorouterd agent`, and `biorouter serve`. The capability header is unaffected and still admits a
private chat on that daemon — a capable caller never reaches the reach gate's version of this
message, because capability is checked before proof. If you are seeing it, your caller is public,
or the session is a delegated subagent's: changing, stopping or steering a subagent from its tab
needs proof that a person acted, whatever the capability, and that refusal says so in its own
words.

**The header worked yesterday and now does not.** The tier came from the daemon's own registry, so a
provider removed from `config.yaml`, or renamed, now resolves public.

**`200` but a null conversation.** Reach is not the problem — `GET /sessions/{id}` carries the full
message list once there is one, and reports `null` for a session that has not yet stored a message.
A session created by `POST /agent/start` is empty until its first turn runs, and it is also still
*public* until then: the classification ratchets when a turn binds a private model, not when the
provider is set. So a brand-new session on a private provider is reachable without the header, and
becomes unreachable after its first turn. That is the ratchet working, not a flapping gate.

## Related documentation

- [Reaching Biorouter from a browser](browser-access.md) — `biorouter serve`, the access token, and why a browser session cannot change its model.
- [Privacy tiers](../security/privacy-tiers.md) — the classification system this header states a capability against; read its "What shipped, and what did not" section first.
- [Headless Linux deployment](headless-linux.md) — running the daemon as a long-lived service, which is where most automation points.
- [How browser-served Biorouter is built](serve-architecture.md) — how the daemon authenticates a request, and what `X-Secret-Key` does and does not prove.
- [biorouter CLI command reference](../cli/command-reference.md) — `session watch`, `send` and `attach`, which send this header for you.
