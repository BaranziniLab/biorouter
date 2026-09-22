# Synthetic 50-UID broker soak (2026-09-22)

This record covers a standalone Linux ARM64 broker workload in a fresh,
disposable Docker container. All users, keys, workspace data, message bodies,
and identifiers were synthetic. The GUI, daemon, existing three-user SSH
fixture, and installed profiles were outside the run.

## Provenance

- Artifact: `/private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew`
- Artifact SHA-256: `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`
- Harness: `docs/research/biorouter-crew/smoke/local_real_uid_soak.py`
- Harness SHA-256: `02d9d4f9d8fa1a61bd3e08f9752772ab45884debc0fd9d6dbfe7e6a21690970a`
- Image digest: `sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`
- Container network: `none`
- Container CPU limit: `4` CPUs
- Workload command: `python3 local_real_uid_soak.py --profiles 50 --duration 1800 --interval 60 --retain-state`

The broker was started with synthetic `/etc/machine-id` state inside the fresh
container only. The container and its synthetic runtime state were removed
after replay evidence was collected.

## Results

- Workload interval: `2026-09-22T18:36:18.247385+00:00` to `2026-09-22T19:06:50.028293+00:00` UTC; requested duration `1800` seconds.
- Messages sent: `1500`; acknowledged: `1500`.
- Unique posted IDs: `1500`; unique read-back IDs: `1500`.
- Disconnects: `0`; harness errors: `[]`.
- Harness minimum per participant: `24` messages; all 50 synthetic participants produced terminal summaries.
- Latency p50/p95/p99: `192.21` / `787.96` / `1017.90` ms.
- Broker maxima: RSS `1260260` KiB, open FDs `106`, journal `2233383` bytes.

The workload exercised signed `message.post` and `messages.history` traffic
with opaque cursor strings. After stopping and restarting the same synthetic
state, enrolled UID replay paginated the retained history and matched every
record's expected body hash:

```json
{"expected": 1500, "mismatched": [], "missing": [], "replayed": 1500}
```

This is standalone broker evidence. It does not establish GUI-under-load,
provider, daemon, real-user, or cross-platform acceptance.

Raw scoped evidence retained outside the repository is at
`/private/tmp/biorouter-crew-soak-refresh-luna-20260922/evidence/`.
