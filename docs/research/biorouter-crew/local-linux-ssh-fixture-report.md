# Local Linux SSH fixture report

Probe time: 2026-09-22 (UTC)

This is a local Docker Linux fixture using the existing `rust:latest` image. No
AWS source, credentials, or network-hosted source were used. The fixture is
kept running for the Crew QA handoff and is bound to localhost only.

## Fixture connection

| Field | Value |
| --- | --- |
| Container | `biorouter-crew-ssh-luna` (`48fd10535ab3291dc3db18da9e6f8b2adaa4c21df642c852af768faa17aac8fc`) |
| Image | `rust:latest` |
| Host endpoint | `127.0.0.1:56928` -> container SSH port 22 |
| Host key file | `/private/tmp/biorouter-crew-ssh-fixture/known_hosts` |
| Machine ID | Container-local root-owned `/etc/machine-id` |

The three synthetic users have distinct Unix IDs and Ed25519 keys:

| User | UID | Private key file |
| --- | ---: | --- |
| `alice` | 1101 | `/private/tmp/biorouter-crew-ssh-fixture/keys/alice` |
| `bob` | 1102 | `/private/tmp/biorouter-crew-ssh-fixture/keys/bob` |
| `carol` | 1103 | `/private/tmp/biorouter-crew-ssh-fixture/keys/carol` |

Connection uses pinned host verification and disables agent fallback:

```sh
ssh -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none \
  -o StrictHostKeyChecking=yes \
  -o UserKnownHostsFile=/private/tmp/biorouter-crew-ssh-fixture/known_hosts \
  -i /private/tmp/biorouter-crew-ssh-fixture/keys/alice \
  -p 56928 alice@127.0.0.1
```

## Initial fixture checks

Three SSH logins passed with the expected identities:

```text
alice: uid=1101, Linux 6.12.76-linuxkit aarch64
bob:   uid=1102, Linux 6.12.76-linuxkit aarch64
carol: uid=1103, Linux 6.12.76-linuxkit aarch64
```

The Linux ARM64 CLI built from revision `e9a09c6c` with `CARGO_BUILD_JOBS=2`
and was installed at `/usr/local/bin/biorouter-crew` and at each user-local
path below. All four copies have SHA-256
`8b97398e547a051a661035e64f207ff6c62ab4001570d4968606b108be79ba5f`.

```text
/home/alice/.local/bin/biorouter-crew
/home/bob/.local/bin/biorouter-crew
/home/carol/.local/bin/biorouter-crew
```

Running `status` as each user against a new state directory returned the
expected `No such file or directory` error. This verifies rootless invocation
without creating a broker or state. A later read-only check observed the coordinated QA broker in
the persistent fixture with workspace
`0fb6e0c6-32c0-4d0a-957e-8a76240d54c2`, socket
`/tmp/crew-1101-d9f258fd97f241a3bf55a6b56b20e0e5/broker.sock`, and journal
sequence 7. The Alice bridge process was live at that observation. Later application
acceptance uses its own process and binary provenance.

The no-argument CLI probe also executed successfully as each user (the
expected usage error, exit code 1), confirming that the installed Linux
executable and its runtime dependencies are available under all three UIDs.

The rootless command used for initial startup has this form:

```sh
/home/alice/.local/bin/biorouter-crew start \
  --state-dir /home/alice/.local/share/biorouter-crew/lab \
  --bootstrap-key <QA_ALICE_PUBLIC_KEY_HEX>
```
