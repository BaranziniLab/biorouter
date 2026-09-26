# Fresh 5455 shared-daemon acceptance (sanitized)

Artifacts:
- Native CLI: `/private/tmp/biorouter-crew-artifacts-5455ebf9/biorouter`, SHA-256 `b329b6ad161e9b6722ef6b08ebef9aab3df4462c25c516a3206f051525fc7bdc`.
- Native daemon: same artifact directory, SHA-256 `e3bcbf581b4ec06f4a24e7a76a27f2843c36e9ef3e8b100087ab85021d5d5498`.
- Verified Linux broker: SHA-256 `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.
- Provider fixture: loopback Ollama-compatible synthetic server on `127.0.0.1:18765`; actual loopback classification observed as private.

Deterministic shared-daemon protocol:
- Create-only session `20260923_1`; exact grant returned run `f2095834-a770-481a-8c52-1dfc6399a293`.
- Ordinary shared-daemon streaming turn completed with private provider pin and terminal Finish.
- Real `crew__connections` returned the fresh connection/workspace/destination/source metadata.
- Real `crew__request` calls returned `messages.history`, `context.manifest`, and `run.project` results; project post used a deterministic idempotency key.
- Exact grant revoke marked the run revoked; subsequent shared turn failed with typed `Crew run was revoked`, and the provider audit recorded zero requests for the refused turn.
- Synthetic tool-call results are protocol evidence only and do not establish natural-language model parity.

Installed-model workflow:
- New create-only session `20260923_2`, exact grant run `5d523c3f-d5d8-4d2d-98c0-41fbfd56d42e`.
- `qwen3:1.7b` through local Ollama completed one ordinary shared-daemon streamed turn with private provider pin, terminal Finish, and the requested `crew-natural-pass` marker observed in the assembled response.
- No model download, old workflow/profile, GUI, AWS, or public-provider boundary claim.

## qwen3:8b ordinary shared-daemon tool workflow (2026-09-23)

Fresh native 5455 session `20260923_3` was created with explicit provider `ollama`, model `qwen3:8b`, then granted to the fresh authorized destination before the ordinary shared-daemon turn. The model used the actual Crew tools in this order: `crew__connections` (call `call_n6futk7g`, success), `crew__request` `messages.history` (call `call_fih4z3wq`, success; returned the existing fresh roundtrip and prior synthetic marker), `crew__request` `context.manifest` (call `call_gf75efzq`, success), then `crew__request` `run.project`.

The first `run.project` request (call `call_v9seg0cx`) was refused by the broker with `invalid_params: body must be a string`; this was an actual schema correction, not a retry of transport or provider. The model corrected the request once (call `call_gntotuva`) with `body: synthetic progress marker`, `status: progress`, and idempotency `qwen8-status-1`; broker success returned message receipt `45029dbb-c658-4aeb-9c25-c728a4f081ca`, `status: progress`, and the fresh run identifier. The final assistant output cited the returned Crew receipt and did not claim success before the tool result. Stream completed with `Finish(stop)`.

This is natural-language-to-real-Crew-tool workflow evidence using the installed local qwen3:8b model; it does not claim real-model parity beyond this bounded request. All profiles, credentials, and channel identifiers remain private to this lane.

## Observer lane (bounded)

A fresh history selection was required before observation: starting `watch --after 0` returned the broker's explicit `stale_cursor` error and ended observation. Refreshing with the cursor from `history --latest --limit 1` allowed a fresh watch. With the refreshed cursor, six authenticated sends (`observer-burst-1` through `observer-burst-6`) were observed by the watch stream; all six markers were received.

A slow-reader probe then consumed the watch stream at 0.5 seconds per line. The reader consumed four lines, while the concurrent sender stopped making progress within the bounded 30-second command window, observing a slow-reader arrangement; the initial compound shell also waited on the watch/pipe lifecycle. Cause of that shell-level stall was unproven. A follow-up per-request probe used the refreshed cursor, a 0.5-second reader, and four independent sender subprocesses with 3-second per-request timeouts: reader consumed 3 frames, and all four sends completed in 0.016–0.042 seconds. After reader termination, an independent send and latest history read completed in 0.042 and 0.037 seconds, respectively, and the marker was present. Thus the earlier 30-second compound-command stall must not be treated as a transport liveness or backpressure failure. The exact owned watcher/probe processes were terminated after their bounds. This probe does not claim fairness or revoked-buffer semantics: no fair concurrent backlog or post-revocation transport-delivery distinction was asserted from this run.


## Redacted reproduction templates

All commands below were run with fresh lane-owned profile roots and secrets supplied only through stdin; concrete roots, connection identifiers, channel identifiers, and approval values are omitted.

```sh
BIOROUTER_PATH_ROOT=<fresh-profile-root> HOME=<fresh-profile-root>/home \
  biorouter crew --no-start --approval-key-stdin --connection <fresh-connection> \
  send <authorized-channel> --text <marker> --output-format json

BIOROUTER_PATH_ROOT=<fresh-profile-root> HOME=<fresh-profile-root>/home \
  biorouter crew --no-start --approval-key-stdin --connection <fresh-connection> \
  history <authorized-channel> --latest --limit 100 --output-format json

BIOROUTER_PATH_ROOT=<fresh-profile-root> HOME=<fresh-profile-root>/home \
  biorouter crew --no-start --approval-key-stdin --connection <fresh-connection> \
  watch <authorized-channel> --after <fresh-history-cursor> --output-format stream-json
```

Baseline send/read completed in 0.044/0.036 seconds. In the slow-reader probe, each of four sender subprocesses had a 3-second timeout; all four completed, while the reader consumed three frames at 0.5 seconds per frame. Post-reader-close send/read completed in 0.042/0.037 seconds. Bob and Carol independent history verification was attempted with their fresh profiles but both required interactive SSH host-key/MFA reauthentication; the supported noninteractive command returned that explicit fixture-auth requirement, so receipt visibility for those two clients remains unverified.

## Large observer backlog stress (2026-09-23)

Before this stress test, Bob and Carol's fresh daemons were restarted with their
intended `BIOROUTER_DEV_PROFILE_ROOT`. Supported interactive TTY authentication,
fresh approval/vault credentials and strict fixture known hosts restored both
connections. Each user's independent history then contained receipt
`45029dbb-c658-4aeb-9c25-c728a4f081ca` and the synthetic progress marker.
This resolves the fixture-authentication prerequisite recorded above and
establishes cross-user visibility for that specific model-generated post.

The fresh Alice profile seeded 60 authenticated messages, each approximately 50 KiB, with zero send errors and 60 broker acknowledgements. A watch stream was opened from the pre-backlog history cursor and its stdout reader was intentionally paused for 6 seconds. During that pause, independent Bob operations remained responsive: human post completed in 0.230 seconds and latest-history read completed in 0.028 seconds, each under the predefined 3-second operation timeout.

After the reader resumed, only 12 unique backlog markers were received across 13 stream lines; the watch exited nonzero with `Crew observation ended without a reconnect frame`. This is an observed observer catch-up/liveness defect under a paused large backlog, not a pass. The same harness with 20 approximately 50 KiB messages and immediate consumption received all 20; therefore the failure is tied to paused pipe/socket consumption and not simply message size. A 20-message 1 KiB diagnostic backlog also received all 20 after the pause. Persisted-history verification of the full 60-message large response was refused by the broker as `response_too_large` when requesting a 100-message window; the 60 individual sends were acknowledged before observation.

Reproduction arrangement: one fresh Alice watch subprocess writes stream-json to a pipe whose reader consumes nothing for 6 seconds; an independent Bob CLI subprocess posts a small marker and a separate Bob latest-history request, each with a 3-second timeout; then the reader drains and counts unique message markers. All credentials are stdin-only and redacted from this report. This requires an Astra fix request for observer reconnect/catch-up handling under large paused output; no source change was made in this lane. Fairness across multiple observer streams and revoked-source queued-context behavior remain unverified.
