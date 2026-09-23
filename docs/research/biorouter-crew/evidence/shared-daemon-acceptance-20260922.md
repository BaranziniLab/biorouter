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

A slow-reader probe then consumed the watch stream at 0.5 seconds per line. The reader consumed four lines, while the concurrent sender stopped making progress within the bounded 30-second command window. The cause remains unproven and is being investigated; a sender stall does not establish healthy bounded backpressure. The exact owned shell/watch processes were terminated after the bound. Fairness, slot release, memory bounds and revoked-buffer semantics remain unverified by this attempt.
