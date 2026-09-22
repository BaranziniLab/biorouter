# Disposable Linux identity and multi-hop smoke

This is a synthetic feasibility fixture for design research, not production Crew code or a security certification. It creates one short-lived AL2023 `t3.micro` in `us-west-2`, uses no PHI or institutional credentials, and cleans up the VM, its security group, AWS key pair, and temporary local private key in a `finally` block. It has no third-party Python dependencies.

Run only after authorization to create AWS resources:

```sh
python3 docs/research/biorouter-crew/smoke/aws/run.py
```

The AWS CLI account must permit SSM public parameter reads, EC2 lifecycle/security-group/key-pair operations, and EC2 console-output reads. The account needs a default VPC and a subnet with public routing. Local requirements are Python 3, AWS CLI, OpenSSH `ssh` and `scp`, and Internet access to discover the current public IPv4 address. The runner records exact AWS resource IDs in a timestamped JSON evidence file and returns a nonzero status for a failed test or a cleanup error. If a process is forcibly killed or the machine shuts down, use the recorded IDs to finish teardown manually; `finally` is not a substitute for an infrastructure expiration policy.

Controls:

- The launch request first must return `DryRunOperation`.
- One encrypted 8 GiB `gp3` root volume with delete-on-termination; IMDSv2 required.
- SSH ingress restricted to the caller's discovered IPv4 `/32`.
- Disposable SSH private key exists only in a temporary directory with mode `0600`.
- All host keys come from authenticated AWS `GetConsoleOutput`, then OpenSSH uses `StrictHostKeyChecking yes`. It does not use `ssh-keyscan` as a trust source or disable host checks.
- All test users share a disposable test public key; actual authenticated Linux accounts have distinct UIDs. This establishes UID behavior, not independent human credential assurance.

The simulated network path is:

```text
local OpenSSH
  -> VM public sshd:22 (transport entry, ec2-user)
  -> same VM loopback gate A:2222 (ec2-user)
  -> same VM loopback gate B:2223 (ec2-user)
  -> same VM loopback target:2224 (crew_alice or crew_bob)
  -> root-owned AF_UNIX broker socket
```

Native `ProxyJump` performs the forwarding. The loopback listeners use separate host keys. This proves OpenSSH's chaining mechanics and transport of the tested requests. It is **not** evidence that either institutional host permits forwarding or supports the same authentication path. Passwords, MFA, Duo, keyboard-interactive prompts, expired sessions, scheduler allocation, and multiple physical gateway hosts are not exercised.

The broker uses Linux `SO_PEERCRED` to resolve the connected process UID, rejects a conflicting claimed username, and binds synthetic agent records to their creator UID. Test invocations only append events; no LLM or real agent command executes. Events are append-only JSON lines, flushed and fsynced after each accepted mutation, and replayed after a service restart. Every client request uses a new SSH connection. A binary attachment containing all 256 byte values is uploaded through SSH stdin and downloaded with `scp`/SFTP through the same chain; SHA-256 checks compare local, remote and downloaded bytes.

The broker deliberately has no team/channel ACLs, privacy labels, policy engine, invitation logic, audit tamper resistance, partial-write recovery, duplicate request IDs, concurrent append writers, snapshots, attachment authorization service, malware checks, quotas, or private-model routing. Its history endpoint exposes every synthetic event to both test users. Those omissions make it unsuitable for real user data. The identity result trusts the host kernel and administrator; UID recycling and multi-host identity mapping require an installation identity and principal lifecycle in the real design.

`broker.py` is the synthetic service, `client.py` is its tiny socket client, `user-data.sh` configures the ephemeral VM, and `run.py` orchestrates assertions and teardown. No source files in BioRouter are modified by this smoke.
