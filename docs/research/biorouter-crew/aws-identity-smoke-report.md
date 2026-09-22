# AWS identity and SSH transport feasibility

The synthetic smoke passed on 2026-09-22 UTC using one disposable Amazon Linux 2023 `t3.micro` in `us-west-2`. The run started at 03:09:33 UTC, completed in roughly two minutes, and left no fixture instance, volume, security group, key pair, or local temporary private key. Exact resource IDs, timestamps, assertions, and cleanup evidence are in [the run record](crew-smoke-20260922T030933Z-8c3c8b.json). Source and reproduction instructions are in [smoke/aws](smoke/aws/README.md).

The executed command was:

```sh
python3 docs/research/biorouter-crew/smoke/aws/run.py
```

It returned exit code `0`. A preceding syntax compilation passed. After the successful run, the runner gained an additional check that polls for deletion of its root volume; that narrow cleanup addition was syntax checked, and the corresponding AWS volume deletion was independently verified below, without launching another instance.

The saved record and current runner omit the AWS account number and client/temporary public IP addresses; fixture resource IDs remain for cleanup verification. This output-only minimization was syntax checked without repeating the billable run.

| Assertion | Observed result | What it establishes |
| --- | --- | --- |
| EC2 authorization preflight | `DryRunOperation` before launch | The requested fixture could be launched with the current account. |
| VM controls | IMDSv2 required; encrypted 8 GiB root EBS; delete on termination; SSH ingress caller IPv4 `/32` | The actual fixture matched its selected launch controls. |
| Trusted SSH host keys | Four public host keys fetched using authenticated AWS `GetConsoleOutput`; `StrictHostKeyChecking yes` | The smoke did not bypass SSH host verification. |
| Native multi-hop SSH | Entry port 22 → gate A loopback 2222 → gate B loopback 2223 → target loopback 2224 | OpenSSH `ProxyJump` works across this simulated topology with separate sshd listeners and host keys. All listeners were on one VM. |
| Linux kernel peer credentials | `crew_alice` UID 1001; `crew_bob` UID 1002 via `SO_PEERCRED` | An installation broker can distinguish two real authenticated Unix users without trusting a caller-supplied username. |
| Forged identity | Alice claiming `crew_bob` received `claimed_identity_mismatch` | The tested identity check rejected a forged application username. |
| Agent ownership | Bob invoking Alice's synthetic agent received `not_agent_owner`; both users could invoke their own | Kernel-derived identity can enforce the tested owner constraint. No real agent or model ran. |
| Binary files | 1,048,599 bytes, including every byte value, uploaded through SSH and downloaded via SFTP; SHA-256 equal at all three points | Arbitrary binary transport can coexist with text metadata over the SSH route. |
| Persistence and reconnect | Four fsynced JSONL events replayed after broker restart; cursor 2 yielded sequences 3 and 4; owner restriction remained | The small single-writer prototype's persisted history and ownership survived this clean service restart and fresh SSH connections. |

The exact binary SHA-256 was `78a320cf15e5b4bd78bf190b6bb1bd2d5425f4ee0267e73a1819e62f2df8d7a6`. Client OpenSSH was `10.3p1` and server OpenSSH was `9.9p1`.

After the runner completed, independent AWS reads confirmed instance state `terminated`, `InvalidVolume.NotFound` for its root volume, `InvalidGroup.NotFound` for its security group, and `InvalidKeyPair.NotFound` for its key pair. The temporary local directory containing the disposable private key no longer existed. There was one VM launch and no retained billable fixture resource.

## Design consequences

Use the SSH-authenticated remote process identity as the source of truth for a Linux broker, with a root-owned installation identity and a principal lifecycle that handles account removal and UID reuse. A username in protocol data can be presentation metadata or an asserted field to validate; it must not grant identity. On a shared login host, an AF_UNIX broker also needs a trusted installation, protected state and socket paths, and a peer-credential check on every connection.

Keep JSON Lines for commands, events and metadata. Store attachment bytes as separately authorized files and move them with SSH streams or SFTP. Encoding arbitrary files inside a line-oriented event log adds cost and corruption risk without helping transport security. This smoke validates raw transport only; production downloads still require channel membership, privacy checks, attachment authorization, quotas and integrity handling.

A single service owning append/fsync/replay is a plausible simple Linux baseline. These tests do not establish crash consistency under torn writes, multi-writer safety on NFS, exactly-once execution, migration behavior, event-log tamper resistance or reliable durability of a particular institutional filesystem. Those remain acceptance tests and storage design requirements.

## Boundaries of the evidence

This is not a HIPAA compliance assessment, a production Crew security test, or institutional MFA evidence. There was no PHI, real model/provider, institutionally managed account, team/channel policy, privacy boundary enforcement, or real multi-host gateway chain. The two accounts shared one disposable SSH public key, so the run establishes distinct Unix execution identities rather than independent human authentication assurance. The prototype's history endpoint intentionally exposes all synthetic events and cannot be used for real data.

Actual keyboard-interactive/Duo flows, host-key enrollment for each institutional hop, `AllowTcpForwarding` restrictions, scheduler allocation, time-limited SSH credentials, expired multiplexing connections, approval prompts and reconnection after revoked institutional credentials must be validated against administrator-approved institutional test sessions. Preserve native OpenSSH handling of those interactions and stop for a human when authentication needs input. Do not substitute this AWS fixture for those checks.
