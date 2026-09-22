# Local multi-hop SSH acceptance probe

Run on 2026-09-22 from the `codex/biorouter-crew` worktree with
`python3 smoke/local_multihop_ssh.py`. The script creates four independent
localhost `sshd` listeners, four distinct Ed25519 host keys, a temporary client
key, and a strict per-hop OpenSSH config. It removes all daemons and temporary
files in `finally`; it does not use or alter the shared `biorouter-crew-ssh-luna`
container.

Observed results from the completed run:

| Probe | Result | Evidence |
| --- | --- | --- |
| Direct target login | PASS | Exit 0; remote command returned UID `501` and `Darwin 25.6.0 arm64` |
| Three-hop `ProxyJump gate-a,gate-b` | PASS | Exit 0; stdout `multihop-ok` |
| Active client cancellation | PASS | SSH process terminated before command completion; no `should-not-run` output |
| Unreachable connection cancellation | PASS | Exit 255; connection refused; no command output |
| Changed target host key | PASS | Exit 255; strict verification emitted `REMOTE HOST IDENTIFICATION HAS CHANGED`; command did not run |
| Keyboard-interactive prompt | UNTESTED | macOS fixture used `UsePAM no`; no synthetic prompt provider was installed |

The successful multi-hop run establishes native OpenSSH routing through two
independent jump daemons with strict host-key checking and forwarding disabled
for agent/X11. It does not establish MFA, Duo/TOTP, institutional policy, or
Linux UID behavior; the local fixture ran under the macOS account UID 501.

The shared Docker fixture was not probed because Docker API access was denied
to this process. No listener, machine ID, user profile, or broker state was
changed there.
