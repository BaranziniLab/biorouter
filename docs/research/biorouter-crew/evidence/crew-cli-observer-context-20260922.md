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
