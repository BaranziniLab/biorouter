# Network/API protocol assessment

The corpus was fixed before measured execution. Its 55 cases comprise 40 attack
cases, 14 usable positive controls, and one known-bridge-grant control that remains
unexercised. Each repetition uses a fresh daemon and synthetic state. Do not treat
three deterministic repetitions as 120 independent attack designs.

`prespecified/case-manifest.json` SHA-256:
`3f702f10ace78d8e101764896dcf8c7cb9cf84de22736fadd9ffa8931df1b237`.

The target is the unmodified official v1.93.4 Linux amd64 package. The daemon runs
as UID 10001; the protocol client runs as UID 10002 in the same disposable
`--network none` container. It connects only to container loopback. The bundled
official browser interface is configured; no model, institutional endpoint,
patient data, real credential, host home, or host service is involved. The broader
environment manifest records container isolation and provenance.

## Reproduction

First emit the concrete corpus without sending any request:

```sh
python3 scripts/security-assessment/network_probe.py --manifest-only \
  --output docs/security/assessment-2026-10/network/prespecified
```

After the environment owner starts a fresh target for repetition `N`, run:

```sh
docker exec --user 10002:10002 brsec-target \
  python3 /assessment/harness/network_probe.py \
  --base-url http://127.0.0.1:3000 \
  --secret ba11ab1e00000000000000000000000000000000000000000000000000000001 \
  --browser-token ba11ab1e00000000000000000000000000000000000000000000000002 \
  --output /evidence/network/repetition-N --repetition N
```

These two published values are disposable synthetic test credentials. The harness
sanitizes them in raw evidence and refuses to overwrite a measured run. Preserve
all results, including errors and unavailable cases. Copy each completed evidence
directory from the environment's dedicated evidence mount into this directory.

## Interpretation and limits

- HTTP and WebSocket tests exercise server protocol decisions. They do not run a
  browser engine or establish browser exploitability.
- Configuration-write attacks check the stored synthetic canary after each
  request, using the authenticated positive-control channel.
- The authentication-window case uses a separate loopback source address so
  earlier cases cannot prefill its counter. It records twenty `401` failures then
  five `429` failures despite changing `X-Forwarded-For`. The paired valid-key
  request succeeds from that same source. This checks failure-response accounting
  and operator availability; resistance to guessing the actual secret relies on
  secret entropy, not a lockout of correct-key requests.
- Correct-secret hostile-origin WebSocket tests isolate the origin check. Their
  provisioned credential is a test instrument, not evidence that an outsider can
  obtain it. Other tests separately exercise absent/incorrect credentials.
- B-U01 is `not_exercised`: no actual coding-agent turn grants and revokes a bridge
  capability here. Unknown-grant denials do not establish expiry enforcement.
- B-19 compares two absent app IDs. It checks their denial responses; an
  existing-versus-absent app enumeration pair is unassessed.
- Manually configured weak/empty daemon secrets, deliberately disabled browser
  tokens, actual remote TLS proxies, native macOS, Electron, stochastic prompt
  injection, and runtime-created app agents are outside this concrete corpus.

The corpus is purposive and finite. Report unique cases separately from repeated
observations, and do not infer a general probability of compromise from its
observed proportions.
