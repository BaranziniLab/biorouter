# BioRouter Crew feasibility results

> September 22 design update: [the implementation plan](implementation-plan.md) now follows the user's no-administrator, home-based deployment requirement, 2–50-user scope, cluster Public/Private toggle and built-in saved-SSH/MCP integration. It also contains the required three-user AWS/dev-app computer-use testing plan. Earlier deployment recommendations in this research snapshot are superseded; recorded source findings and probe results remain historical evidence, not rootless product acceptance.

Tested September 21, 2026 Pacific / September 22 UTC. These are synthetic architecture probes, not tests of an implemented Crew product. No PHI, private research records, real model prompts, production daemon installation or institutional compute jobs were involved.

## Result

The proposed SSH + Unix identity + text journal + ordinary file foundation is feasible on the measured paths. Both supplied institutional accounts can execute a stdlib-only probe, exchange text and arbitrary binary data, use local Unix sockets and access the SFTP subsystem. An isolated AWS fixture established that two real Unix UIDs can remain distinct through native OpenSSH jump routing and can enforce a small owner check.

MFA, institutional gateway policy, full channel/file authorization, real owned agents and private/public confinement remain required production acceptance work. A successful synthetic owner predicate does not establish that the rest of an application cannot bypass it.

## Institutional host probes

Command run in the isolated worktree:

```sh
python3 docs/research/biorouter-crew/smoke/run_host_probes.py
```

Final exit code: `0`. [Exact results](institutional-host-smoke.json); [driver](smoke/run_host_probes.py); [remote probe](smoke/host_probe.py).

The driver uses `ssh -T`, `BatchMode=yes`, `StrictHostKeyChecking=yes`, connection timeouts and `ForwardAgent=no`. It sends the probe to `python3 -` and uses only a newly created private temporary directory. An additional process returns 1 MiB of supplied synthetic bytes unchanged over SSH. `sftp -b -` runs `pwd` and `quit` without listing any files.

| Observation | `wagu@narrows-login.sdsc.edu` | `wanjun@leo.ucsf.edu` |
|---|---|---|
| Existing noninteractive SSH authentication | Passed | Passed |
| Authenticated account/UID | `wagu`, 1135 | `wanjun`, 1020 |
| Kernel | Linux 4.18.0-553.64.1.el8_10.x86_64 | Linux 5.15.0-141-generic |
| Python used by probe | 3.9.16 | 3.12.7 |
| Home filesystem from `stat -f` | NFS | ZFS |
| Temporary fixture filesystem | XFS | `ext2/ext3` reported by `stat` (do not infer exact on-disk ext version) |
| `sbatch` / `srun` in current PATH | Both present | Neither found in this PATH |
| `biorouter` in current PATH | Not found | Not found |
| Private fixture directory/file modes | 0700 directory; 0600 files | Same |
| JSONL write, fsync, close, reopen | Exact three-record replay | Exact three-record replay |
| Cursor example | Sequence > 2 returned sequence 3 | Same |
| Incomplete last JSON record | Detected and truncated; prior complete records preserved | Same |
| Atomic synthetic file publication | 1 MiB, write/fsync/rename/directory-fsync, matching hash | Same |
| Unix socket peer identity | Kernel UID matched authenticated account | Same |
| Supplied name vs kernel identity | A different claimed name did not change kernel identity; mismatch detected | Same |
| Binary SSH stdin/stdout round trip | 1,048,576 bytes, SHA-256 matched | Same |
| SFTP subsystem | `pwd` command completed | Same |
| Temporary fixture cleanup | Confirmed removed | Confirmed removed |

The remote program has 11 boolean primitive assertions per host. Each passed; the driver additionally checks SSH binary integrity and the SFTP command, requiring zero exit statuses. The socket server and client in this probe belong to the same account. Its name-mismatch observation does not test rejection of an authorized operation; the AWS fixture supplies the actual forged-identity and cross-owner denial checks. The synthetic torn tail is a controlled incomplete-record case, not a power-loss, disk-corruption or multi-writer test.

An initial raw SFTP INIT probe closed its input immediately and received no version bytes despite a clean SSH exit. That was an inconclusive harness behavior, not proof that SFTP was disabled. The final probe uses the standard SFTP client, which waits for the response and successfully executes `pwd` on both hosts. The final checked-in driver incorporates that correction.

No sustained storage benchmarking was performed. The home filesystem observation motivates an approved storage decision; temporary-directory success does not validate HOME or institutional backups. Neither account was shown an MFA challenge on the measured connection. These runs use the selected native SSH profile; they do not record a definitive authentication method or fresh authentication at every hop. They say nothing about fresh-device, expired-credential or other institutional authentication paths.

## AWS two-account and jump-route fixture

Command:

```sh
python3 docs/research/biorouter-crew/smoke/aws/run.py
```

Exit code: `0`. One `t3.micro` instance, Amazon Linux 2023, `us-west-2`, from 03:09:33 to 03:11:29 UTC. [Detailed report](aws-identity-smoke-report.md), [recorded assertions and cleanup](crew-smoke-20260922T030933Z-8c3c8b.json), [reproduction scripts](smoke/aws/README.md).

- Entry SSH port 22 → loopback gate A:2222 → loopback gate B:2223 → loopback target:2224. Separate sshd listeners and host keys, all on one VM; this is a simulated multi-hop topology.
- `crew_alice` and `crew_bob` had distinct real UIDs 1001/1002. Kernel-derived identity survived the route; a claimed different username was rejected.
- Each could invoke its own synthetic agent record; Bob's attempt to invoke Alice's record was rejected. No BioRouter agent, tool subprocess or model ran.
- A 1,048,599-byte synthetic binary file uploaded through SSH and downloaded via SFTP with matching local/remote/download hashes.
- Four fsynced JSONL events and the ownership mapping survived a clean broker restart and fresh connections; replay after sequence 2 returned 3 and 4.
- Host keys were obtained through authenticated AWS console output. Strict host-key checking remained enabled. The instance used encrypted delete-on-termination EBS, required IMDSv2 and SSH ingress limited to the client's IPv4 `/32`.

The fixture broker ran as root for a disposable infrastructure probe and exposed all synthetic history; it is not the planned production unprivileged broker or its ACL design. Both test accounts used the same disposable public key. This establishes distinct Unix identities, not independent human identity assurance. These limitations are explicit to avoid promoting a tiny prototype into a security claim.

The instance was terminated. The root volume, security group and key pair were deleted, and the local private-key directory was removed. The research agent verified this after the run; the primary agent repeated read-only checks of the exact instance, volume, group and key. No fixture resources remain active. Runtime cost is not asserted to be zero; no cost report was queried.

## Design consequences

1. Native OpenSSH stdio is a viable first transport. SFTP is available on the measured accounts but remains optional for Crew's versioned, authorization-aware attachment protocol.
2. Broker-side Unix peer credentials can bind local bridge requests to accounts. They do not distinguish a human from an unrestricted process running as that same UID, or authenticate a bridge on another kernel.
3. JSONL and separate files work as portable primitives. A production journal still needs idempotency, checksum validation, crash injection, exclusive writer ownership, retention and backup/restore tests.
4. Narrows NFS HOME argues against blindly using a home-directory database or shared writable logs. Choose one approved service and durable storage location, and test its semantics.
5. The next meaningful milestone is the real protocol/authorization fixture and desktop SSH/MFA adapter. Re-running these small probes at larger payloads will not establish the missing security properties.

## Explicitly untested

Real Duo/TOTP/security-key interactions; actual institutional jump hosts; changed-key UX and maximum auth-age enforcement; locked-down SSH `ForceCommand`/forwarding policies; Windows/Linux desktop clients; dedicated unprivileged broker installation; team/channel ACLs and active revocation; multiple simultaneous clients; load/rate limits; production JSONL crash/compaction/migration behavior; broker-level attachment ACLs/resumability; Slurm execution; real private/commercial model calls; same-UID shell escape and egress confinement; backups and disaster recovery; HIPAA deployment acceptance.

Those gaps are mapped to implementation phases and exit criteria in the [implementation plan](implementation-plan.md).
