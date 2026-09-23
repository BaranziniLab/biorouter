# Fresh Crew CLI observer and context evidence

Date: 2026-09-22. This artifact records a fresh synthetic three-user CLI
fixture. Credential values, profile paths, socket paths, host ports, process
IDs, and private inventory are intentionally omitted.

## Provenance

- Native CLI SHA-256 for enrollment, transfers, helper, and initial context
  runs: `f27849c0c3224db9365e25090e360f9dc3f06acc1da9c86ce66d85bb92c41e32`.
- Native daemon SHA-256 for those phases:
  `0ad2d72adfbaabddcf30fd4e3b589ed9ca2f8fd43d67bc1cb35086e5ee233f6c`.
- Observer CLI SHA-256 for watch, revocation, and final context phases:
  `0450257748d1870047ee311cccd387a66f950b12de562ca6eeb440523d34a197`.
- Observer daemon SHA-256 for those phases:
  `56e254bc6a7b1c1a1f51404c19c5f4d9f6d869fb4f13edf97f0f35aee5a52d1f`.
- Personal MCP session `20260922_7` used the same recorded observer CLI and
  daemon pair above, before a later mutable-target rebuild. The hashes above
  are measured phase provenance, not a claim about the current file at those
  paths.
- Matched-906 pause/post phase used CLI SHA-256
  `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8` and
  daemon SHA-256
  `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`.
- Linux broker SHA-256: `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.
- The checks ran in the existing synthetic Linux SSH container, but used a
  new broker state directory, fresh encrypted-vault profiles, fresh device
  identities, and strict pinned host trust. They did not use the retained
  broker state or retained GUI profiles.

## Results

| Case | Result | Evidence |
| --- | --- | --- |
| Three-user enrollment and shared channel posts | PASS | Fresh Alice, Bob, and Carol principals enrolled and each posted successfully. |
| 43-byte upload and downloads | PASS | Alice upload completed; Bob and Carol downloads independently matched SHA-256 `d5231aa05fefa605797ecc8e3ac2098e2903385d2e02bcdaa664e4d26a5836ff`. |
| 32 MiB pause/restart/resume | PASS | Receipt paused at a nonterminal offset, survived daemon stop/start, vault unlock, reconnect, and resumed to 33,554,432 bytes; final SHA-256 `56a388beba6bfe73739e2d0e635a4617e8e649bb69bea6109b8417af259fd7be`. |
| Owned helper task | PASS | Fresh empty destination and source. Ordered `remote.execute`, `remote.job_status`, and `remote.hash`; output SHA-256 `f1ff17176d0b566c832f5e6dbbeafe912d93b6f7a0548204c9d551cb1fab3c0a`, 18 bytes. The exact remote job record independently reported `completed`, exit code `0`, and matched run ID. |
| Public control/private denial | PASS | Synthetic public sink received one control request. Private-mode public-model request was refused with `Private cluster blocks public models`; sink count remained one. |
| Baseline watch and detach | PASS | One canary event was observed; Ctrl-C detached with exit code 0. |
| SIGSTOP/cancel/drain | PASS | Fresh broker was stopped, watcher was cancelled while stopped, broker resumed within two seconds, and the next supported history request succeeded. |
| Watch revocation/drain | PASS | Carol’s active watch ended with `channel_access_changed` after owner revocation; a subsequent owner post was not delivered to Carol. |
| Pause with concurrent post | OBSERVATION | Posting while the broker was stopped produced an observation refusal/stale-cursor message and a subsequent broken pipe. Explicit disconnect/connect restored history. No causal product conclusion is made. |
| Mixed-artifact pause/post probe | INVALIDATED | The test CLI and running Alice daemon were from different mutable-target phases; its observer framing error is excluded from acceptance. |
| Matched-906 pause/post/drain | PASS | After explicit reconnect and vault unlock, the structured baseline watch remained alive for one second. Broker SIGSTOP was followed by asynchronous post, SIGCONT after 0.575 seconds, 21 drained watch lines including the post, Ctrl-C exit 0, and history marker count exactly one. |
| Matched-906 live watch identity control | PASS | A separate control post was observed live and in history with the same message ID and body; post and history exited 0. Retained evidence is `/private/tmp/crew-cli-parity-final-luna/watch-exact-906.json`. |
| Observer slot admission/release | PASS | On a new private synthetic channel, 16 observers stayed alive for three seconds; the 17th returned HTTP 429 `Crew observer refused (429 Too Many Requests)`; after detaching one, a replacement observer stayed alive. All owned observers then detached cleanly. |
| Adaptive oversized history | PASS | A new private channel received 20 distinct 59,983-byte messages. `history --limit 200` returned broker `response_too_large` / `Request a smaller history window`. Default watch drained 20 direct message frames, all 20 seeded IDs and body hashes matched, with no missing or duplicate IDs and clean exit 0. |
| Expected-mode public file refusal | PASS | On the verified private connection, `files upload --expected-mode public` with a nonexistent local path refused with `Crew connection privacy changed; refresh the verified workspace before selecting a file`; local file selection was never reached. |
| Expected-mode private file upload | PASS | The same private connection accepted the existing 43-byte synthetic file and completed with SHA-256 `d5231aa05fefa605797ecc8e3ac2098e2903385d2e02bcdaa664e4d26a5836ff`. |
| Saved-public versus expected-private | PASS | A supported descriptor update changed only the saved mode to public; `files upload --expected-mode private` refused before transfer with the same privacy-change error. The exact private descriptor was restored and reconnect succeeded. |
| Resume reselection versus saved-public | PASS | CLI `files resume` reselected the local file after saved mode was changed to public; `--expected-mode private` refused before transfer with the same privacy-change error. This is reselection evidence, not opaque-capability reuse. The exact private descriptor was restored and reconnect succeeded. |
| API capability versus saved-public | PASS | Supported `POST /crew/files` registered a private 43-byte capability. After a supported saved-public flip, `POST /crew/transfers` reused that capability with the same request ID and no path reselection; it returned 400 `Local file approval does not match this transfer`. The before/after transfer list contained no receipt for that request ID. Private mode and reconnect were restored. |
| Task expected-private versus saved-public | PASS | With saved mode public, `tasks start --expected-mode private --allow-posting` returned an admission refusal: `Crew connection privacy changed; refresh the verified workspace before granting agent access`. No independent provider-request counter was measured; no provider dispatch follows the reviewed admission guard order. The exact private descriptor was restored and reconnect succeeded. |
| Selected-context first attempt | FAIL | The task returned the prior helper output hash instead of the fresh canary hash. The cause is unproven; no stale-history explanation is asserted. |
| Selected-context isolated repeat | PASS | New empty destination and new single-message source; the prompt omitted the canary value. The model retrieved the exact canary. Independent canary-value SHA-256: `eab33425f6b39d8137463cbc52d1704f41b888148092d17689345c73ed24fdaf`. |
| Unauthorized source history | PASS | Carol’s source read was refused with `forbidden: channel unavailable`; no source value was retrieved. |
| Unauthorized source task admission | PASS | Carol’s task on an authorized destination with Alice-only source context was refused before run/provider creation with `forbidden: channel unavailable`. |

## Reproduction shape

The checks used supported CLI operations conceptually equivalent to:

```text
biorouter crew connections prepare/save
biorouter crew workspace bootstrap
biorouter crew enroll invite/accept
biorouter crew teams/channels create
biorouter crew send
biorouter crew files upload/status/download
biorouter crew files pause/resume
biorouter crew daemon stop
biorouter crew daemon start
biorouter crew credentials unlock
biorouter crew connect
biorouter crew connections update -
biorouter crew files upload
biorouter crew files status
HTTP POST /crew/files
HTTP POST /crew/transfers
HTTP GET /crew/transfers
biorouter crew tasks start/show
biorouter crew history/watch
```

Approval and vault inputs were supplied through the documented secret stdin
paths. No secrets were placed in arguments, prompts, logs, or this document.

## Scope limits

This is synthetic local Linux/SSH evidence. It does not establish institutional
MFA, public cloud access, HIPAA compliance, GUI parity, or Windows support.
The first selected-context mismatch and the concurrent-post pause observation
remain retained as bounded findings for follow-up; neither is upgraded to a
root-cause claim here. A later mutable-target probe measured CLI
`4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8` and
daemon `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`,
but the running daemon image could not be inferred from those replaced path
files; that probe is excluded from acceptance evidence.

The matched-906 observer first emitted a structured baseline message carrying
`id`, `body`, `channel_id`, and `sequence`, and remained alive for one second
before the broker fault. Its output was continuously drained. After SIGCONT,
the asynchronous post appeared in 21 drained lines, and a fresh history page
contained the marker exactly once. Retained JSON evidence is
`/private/tmp/crew-cli-parity-final-luna/pause-post-906.json`.

## Personal natural-language Crew MCP post/read and revocation

Fresh Alice personal session `20260922_7` started with the local Ollama
model (`qwen3:8b`) and Crew builtin. A harmless first turn completed with
`PERSONAL_READY`; no human secret was included in the prompt. The supported
CLI grant supplied the human approval key only through stdin:

```text
biorouter crew --no-start --connection <alice-connection> --approval-key-stdin \
  grants grant 20260922_7 2d6ed29f-455e-4212-9c38-9ece2301d875
```

The grant returned run `d5527b98-0969-478c-bb03-4b9e41c4833a`. An ordinary
natural-language `session send` turn invoked `crew__connections`, then
`crew__request` with the supported `run.project` posting method, then
`messages.history` for the same selected channel. The model reported posted
message ID `955187be-7031-4596-9a14-56a9ad2224f0` and body
`PERSONAL_MCP_POST_20260922_7`; the body matched the history result. This is
actual personal-session MCP evidence, separate from GUI acceptance.

The initial prompt intentionally requested unsupported `message.post`; the
typed Crew response refused that method and listed supported methods. No
message was claimed from that attempt. The corrected `run.project` call is the
supported owned-agent update/post path.

The grant was then revoked with the supported CLI command. A follow-up
natural-language turn attempted one connection lookup and one history request;
the daemon refused inference before tool execution with the exact sanitized
error `Crew run was revoked; request a fresh human grant`. No channel data was
returned after revocation.

This case used observer artifacts: CLI
`0450257748d1870047ee311cccd387a66f950b12de562ca6eeb440523d34a197`, daemon
`56e254bc6a7b1c1a1f51404c19c5f4d9f6d869fb4f13edf97f0f35aee5a52d1f`, and
Linux broker `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.

## Authority and provider-counter API checks

The bounded isolated loopback-provider harness was run with daemon
`45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9` and Linux
broker `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.
The public control run completed and produced exactly one loopback sink request.
After switching the synthetic connection to private mode, an equivalent public
provider run was refused with `crew_request_refused: Private cluster blocks
public models`; the sink count remained exactly one. This is local loopback
provider evidence and does not use an external provider.

On the fresh Alice daemon, direct `POST /crew/files` attempts with a synthetic
file and both missing and wrong `X-User-Action` proof returned HTTP 403
`crew_transfer_refused` with `A verified human action is required for local file
and transfer access; agent grants and API keys do not authorize it`. A
human-authenticated read-only transfer list contained six existing receipts and
no receipt with either negative-test request ID. No file or transfer side effect
was observed.

The same isolated loopback harness exercised foreign-owner cancellation: the
wrong connection returned HTTP 400 `crew_request_refused` with `This task is not
owned by this device and connection.` The owned cancellation returned 200 with
`cancelled: true`, `remote_revocation_confirmed: true`, and one cancelled finish
event; a completion-first cancellation returned `already_finished: true` and
`cancelled: false`. The sink saw two requests total (one held cancellation run,
one completion control), with no late completion overwriting cancellation.

## Follow-up observer backpressure attempt

The pinned daemon hash was rechecked directly:
`45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`.
No backpressure pass is claimed. A slow-reader attempt against the retained
adaptive channel exited before the eight-second unread interval with
`observation_refused`: `Room observation ended ... a stale cursor requires an
explicit fresh history selection`. A fresh private channel creation was then
attempted twice on the connected Alice profile; both returned HTTP 400
`Broken pipe (os error 32)`. Read-only status immediately afterward confirmed
the daemon remained connected/private with no last error. Because a fresh
channel could not be admitted and the retained channel had a stale cursor,
slow-reader expiry, fairness under sustained backlog, and ACL-revocation
transport recovery remain unexecuted rather than inferred from this attempt.

## Pinned 9a restart diagnosis

The owned Alice daemon was stopped and restarted with the pinned 9a pair (CLI
`9bbedb34c349e3b637c8b8e01e6e4e50807ffa86bf1bcfaa1562fc3c64195c5f`, daemon
`50eefaaebad298679d9ad525a98a911941ea8ff62c462033a389fcd3351b005f`). Vault
unlock succeeded. Immediate history then refused with the typed state
`Crew connection is disconnected; authenticate and connect in Crew`. A fresh
supported connect failed with `Crew SSH read failed; reconnect. Submitted
operation outcome may be unknown; inspect history before retrying` (HTTP 400).
The test stopped at this first actionable failure and did not retry a possibly
unknown operation. The earlier direct bridge probe is separate evidence; this
is a reproducible higher-level connect failure after the pinned daemon restart.

The original host-trust refusal is classified as fixture setup: the daemon
process had not inherited the owned profile SSH environment, even though the
profile and verified source known-hosts records matched. After restarting the
owned daemon with that profile environment (without changing host keys), native
authentication succeeded with exit 0 and `authenticated: true`.

With the authenticated 9a session, immediate small history succeeded in 0.036
seconds. The following adaptive history request correctly returned
`response_too_large` / `Request a smaller history window` in 0.065 seconds, and
the next small history request succeeded in 0.031 seconds. This is a bounded
current-artifact history recovery sequence; it does not qualify slow-reader
backpressure or fairness.

## Pinned 9a transfer checks

One fresh synthetic upload capability registered with HTTP 200. The transfer
completed with HTTP 200 and the receipt SHA-256 matched the local synthetic
file (`de436eaab40d2aa3ac514b47098f6fbf8d6b7bb5dd8e7bd615160763243d5cf9`). A
same-request replay was refused with HTTP 400 and did not create a second
receipt; the completed receipt remained the only outcome for that request ID.

After registering another capability, changing the selected source before
start returned HTTP 400 `The selected source changed; select the original file
again`. A missing path returned HTTP 400 `No such file or directory (os error
2)`; creating that path afterward did not turn the refused selection into an
approved capability. These checks used fresh request IDs and human proof.

## Pinned 9a download-capability checks

The completed synthetic upload supplied a blob for the download contract. A
download selection registered with `approval_pending: true` returned HTTP 200;
confirmation of the unchanged destination returned HTTP 200, and transfer
start completed HTTP 200. The destination sentinel was replaced by the blob,
with resulting SHA-256
`de436eaab40d2aa3ac514b47098f6fbf8d6b7bb5dd8e7bd615160763243d5cf9`.

Changing an existing destination after registration but before confirmation
returned HTTP 400 `Destination changed since selection; select it again to
approve publication`, and the replacement sentinel remained. Registering an
initially absent destination succeeded as a pending selection, but creating the
path before confirmation produced the same HTTP 400 refusal and preserved its
sentinel. Missing and wrong proof on confirmation both returned HTTP 403 with
the verified-human-action refusal.

A fresh registration using the completed download’s original request ID,
directory/name, overwrite choice, and blob returned a replay capability.
Confirmation and start both returned HTTP 200, the original receipt ID was
returned, and the destination hash was unchanged. This is the receipt-bound
download replay; it is distinct from reusing a consumed capability.

After confirming an unchanged existing destination, the destination was
replaced before publication. Transfer start returned HTTP 200 with a receipt,
but the receipt settled as `needs_file_selection` with
`Transfer stopped. Reselect the original local file or destination to resume.
Inspect any unconfirmed publication before retrying.` The replacement sentinel
and its SHA-256 remained unchanged, while no download bytes were written. This
is the post-confirm mutation refusal represented as a durable needs-selection
outcome rather than a second local overwrite.

A fresh 32 MiB synthetic download was started and paused at offset 262,144;
the persisted receipt was `needs_file_selection`. After daemon restart, vault
unlock and native PTY authentication, a new destination selection bound to the
same receipt resumed successfully. The final receipt reached `completed` at
offset 33,554,432; the downloaded file was 33,554,432 bytes and its SHA-256
matched the receipt (`56a388beba6bfe73739e2d0e635a4617e8e649bb69bea6109b8417af259fd7be`).

Receipt-bound cleanup was then exercised against the owned post-confirm
needs-selection receipt. Cleanup registration and deletion returned HTTP 200;
the receipt disappeared from the transfer list, while its destination hash and
an unrelated sentinel file remained unchanged.

A separate paused 32 MiB download reached `downloading` before pause, with
receipt offset 1,572,864 and an actual hidden partial file of the same size.
Receipt-bound cleanup returned HTTP 200, removed the receipt and that partial
file, and left the unrelated sentinel unchanged. The final transfer list no
longer contained the receipt.

## Pinned 4a observer-lane setup outcome

The owned Alice daemon was restarted with the pinned 4a CLI/daemon pair, but
the first supported status/auth call stopped before credential handling with
`Daemon identity could not be verified; refusing replacement: Operation not
permitted (os error 1)`. The observer slow-reader, backlog fairness, and
revocation-buffer checks were not run against 4a; no identity guard was bypassed.

## Broken-pipe recovery diagnosis

The owned Alice profile was checked after the observer attempt. Read-only status
reported the connection `connected`, mode `private`, and `last_error: null`.
The retained broker process was alive (PID 5350) and its owned stdio bridge was
also alive. A simple history request still returned HTTP 400 `Broken pipe (os
error 32)`. An explicit supported disconnect completed successfully; a fresh
supported connect completed in 0.094 seconds and returned the same connection
identity, workspace, and socket, but a subsequent status/history round-trip
still reproduced the same broken-pipe history error. The broker log contained
only its startup metadata and no additional sanitized error category.

This isolates the observation failure to the bridge/backend request path after
reconnect, rather than proving a stopped broker or evicted product connection.
No process was killed, no profile state was edited, and no backpressure pass is
claimed.

## Direct SSH bridge protocol isolation

One fresh strict-host-key SSH bridge session was opened using the owned Alice
fixture. Two successive public `hello` frames and one synthetic
`auth.challenge` frame each received a valid response: 3/3 responses, at about
0.071 seconds elapsed by the third frame. The challenge response had the
expected synthetic workspace/UID shape; nonce, signatures, endpoint details,
and credential material were discarded. The probe then terminated its own SSH
process, so its exit 255 is expected cleanup rather than a bridge failure.

This shows the SSH bridge and broker can answer successive direct frames while
the higher-level daemon history request still returns broken pipe. It narrows
the unresolved problem to the daemon request/response path or its bridge
session lifecycle; it does not qualify observer backpressure or recovery.

## 9a connect versus direct bridge comparison

Using the same saved target and configuration, one fresh strict SSH bridge
`hello` received a valid result in 0.071 seconds while saved connection status
remained `disconnected` with no stored last error. The intentionally terminated
probe exited 255 during cleanup. Aggregate read-only process-state counts in
the synthetic container were 7 Alice running-state entries, 4 Bob, 4 Carol,
and 58 Alice zombie entries; no process was modified. The bridge therefore
answered and the failure is not a simple listener absence. The zombie count is
fixture-health context only and is not asserted as causal.

## 9a native auth follow-up

After the pinned daemon restart, the supported native `crew auth` flow was
attempted with the owned profile’s explicit approval input and its profile SSH
configuration. The flow stopped before daemon verification because strict host
key checking reported `No ED25519 host key is known for [127.0.0.1]:56928` and
`Host key verification failed`; the CLI returned the sanitized category
`SSH authentication ended before the daemon verified and retained the broker
connection`. A prior attempt without the profile environment had the same
pre-verification class. No host key was added or replaced, and no further
connect/history mutation was attempted.
