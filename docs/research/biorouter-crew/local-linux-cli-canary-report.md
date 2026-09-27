# Local Linux CLI and helper canary report

Probe date: 2026-09-22 (UTC)

These checks used the existing local `rust:latest` image and the Linux ARM64
`biorouter-crew` binary built from revision `e9a09c6c`. No AWS source or
credentials were used. The main three-user SSH fixture was not modified.
The broker contract integration file is explicitly gated with `#![cfg(unix)]`;
the journal and peer-credential cases below are Linux-only evidence.

The updated artifact was built from base commit
`e9a09c6ce2845e0039b892a02dd3c408528edf55` plus the then-current working-tree
diff for `broker.rs`, `remote.rs`, and `broker_contract.rs`, whose SHA-256 was
`5f9bcc749229314f10eb5b893463ad0d45716269628bdd74ffd2d8176ae45354`.

The rebuilt artifact used for the updated checks has SHA-256
`4fae26ea928eb28ca2bdadc4840d81fd4ca3df77d1bd0a4d5421e7b3ed53beda`.
Reproducible sanitized harnesses are committed at
`smoke/local_remote_canary.py`, `smoke/local_uid_capacity_smoke.py`, and
`smoke/local_real_uid_soak.py`. The real-UID workload harness enrolls users,
accepts team invitations, posts and reads signed messages, records
acknowledgements/disconnects, and samples latency percentiles, broker RSS, and
journal size. Its 50-profile, 30-minute run was syntax-checked but not
started; it is reserved for the coordinated three-user QA completion and uses
a separate synthetic workspace.

Before the coordinated 50-profile run, two bounded warmups completed in the
separate disposable container `biorouter-crew-soak-luna`. Each participant
used a distinct Unix UID, signed enrollment and team acceptance, then sent and
read messages continuously for 60 seconds. The corrected harness collected
terminal records after every child completed, checked child exit status,
distinct UIDs, acknowledgement counts, and exact posted-ID history replay:

| Profiles | Sent / acknowledged | p50 / p95 / p99 (ms) | Broker RSS max | Journal bytes max |
| ---: | ---: | ---: | ---: | ---: |
| 10 | 60 / 60 | 22.57 / 47.77 / 52.69 | 17,352 KiB | 124,173 |
| 30 | 180 / 180 | 48.67 / 156.90 / 183.78 | 107,444 KiB | 386,957 |

Both runs reported zero errors and zero disconnects; broker CPU maxima were 37
and 229 ticks, and open-file maxima were 26 and 66. These are capacity probes
rather than the requested 30-minute acceptance soak. The 50-profile run
remains pending the clean three-user UI workflow.

## Broker restart and journal refusal

In disposable container `biorouter-crew-regression-luna`, a rootless Alice
broker stopped and restarted with the same workspace identity and socket:

```text
socket: /tmp/crew-1101-bd75a4fdae8f4a99a309df18f8ad8f2d/broker.sock
workspace: 2b7d789d-884f-4cb8-ba9f-65355b7f5427
socket_stable: true
workspace_stable: true
```

After a synthetic valid-checksum wrong-node delta followed by an unterminated
torn tail, CLI startup refused with
`node_identity_changed: workspace belongs to another writer node; automatic
failover is disabled`. The journal SHA-256 and top-level directory entry set
were unchanged across the refusal.

## Actual scoped remote helper

In disposable container `biorouter-crew-canary-luna`, rootless Alice created a
private run grant for `/home/alice/work/csv`. The initial canary read three
rows, wrote `summary.csv`, and completed with `total=10`. The updated-binary
rerun used `/home/alice/work/csv-updated` and completed with `total=31`; its
readback bytes and digest were:

```text
rows=3 total=31
readback: rows,total\r\n3,31\r\n
```

The following actual bridge requests were denied:

| Canary | Observed result |
| --- | --- |
| Fake worker credential | `remote_operation_denied`, invalid grant |
| Absolute `/etc/passwd` path | `remote_operation_denied`, path must remain relative |
| Parent traversal `../.ssh` | `remote_operation_denied`, path must remain relative |
| In-tree symlink read | `remote_operation_denied`, too many levels of symbolic links |
| In-tree symlink execution tree | `remote_operation_denied`, work tree contains a symlink |
| Python socket connection | Job failed with `PermissionError: [Errno 1] Operation not permitted` |
| Python `subprocess.run` | Job failed with `PermissionError: [Errno 1] Operation not permitted` |

## Connection availability probe

In disposable container `biorouter-crew-connquota-luna`, the pre-fix binary
allowed synthetic attacker UID 1109 to hold 64 idle Unix connections. An
independent Bob UID 1102 hello then received `ConnectionResetError`. After
releasing exactly one attacker connection, Bob completed hello and received a
valid protocol response. This is regression evidence for the old global-only
admission behavior.

The rebuilt artifact was then installed in the same disposable workspace. With
eight attacker connections held, the ninth attacker hello was reset while Bob
hello succeeded; after releasing one attacker connection, the ninth attacker
hello succeeded. This verifies per-UID admission and permit release.
