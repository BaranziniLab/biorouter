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

## Bounded policy and reconnect extension

The test-owned fixture was rerun with strict generated host pins and an added
SFTP-only target. The current artifact and Crew daemon were not involved; these
are native OpenSSH compatibility probes.

| Probe | Result | Evidence |
| --- | --- | --- |
| Two-hop route with target forwarding disabled | PASS | Target `AllowTcpForwarding no`; approved `ssh target 'printf forwarding-disabled-exec'` returned exactly the marker with exit 0 |
| Forced SFTP-only target | PASS (incompatible) | `ForceCommand internal-sftp`; exec returned exit 1 and `This service allows sftp connections only.` with no marker |
| Changed gateway key through two jumps | PASS (refusal) | Gate A key replaced; strict pinned route exited 255 with `REMOTE HOST IDENTIFICATION HAS CHANGED` naming port 57102; no target command output |
| Changed final target key | PASS (refusal) | Strict pinned route exited 255 with changed-key warning naming port 57104; no target command output |

The separate existing synthetic PAM fixture at `127.0.0.1:56929` was used for
exactly two manual reconnect invocations with the enrolled test key and the
synthetic PAM password. Both prompted once, returned UID 1000, and produced one
accepted keyboard-interactive session each in `/tmp/sshd.log`; no automatic
retry loop was observed. This is native OpenSSH/PAM evidence only: it does not
exercise Crew's desktop reconnect state machine or prove institutional MFA
semantics. The fixture contained no Duo/TOTP provider.

## Native Crew CLI MFA/ProxyJump acceptance (2026-09-22)

A separate disposable profile and the existing independent synthetic Linux PAM container were used after the retained Crew Electron clients were closed. The refreshed native artifacts were hash-checked locally: CLI `784c945022ca661b2cb97c8677686c044061226e00a9fa26bc752b0aa5b2271c`; daemon `5aa6d1c3e539e3c3c635ef0660af599aba51a008f49fa44c12a7c29937d2ded3`. The isolated profile was initialized with a fresh credential vault and a synthetic connection descriptor. Its SSH profile was patched only to add the disposable localhost jump alias and pinned host key; no retained profile or workspace files were changed.

The real native `biorouter crew auth` command used the refreshed CLI and daemon with an encrypted synthetic key, strict host-key checking, and a two-hop localhost ProxyJump ending at the PAM fixture. It reached both hop key-passphrase prompts and the target keyboard-interactive PAM prompt, then returned `authenticated: true` and exit code 0. A follow-up `crew status` after CLI exit remained `connected`, confirming the daemon-owned master persisted. Counts: successful ProxyJump/PAM auth 1; wrong-secret rejection 1; prompt cancellation 1; initial zero-size PTY rejection 1; corrected 24x80 PTY success 1; profile/fixture secret sweeps with no persisted synthetic secrets 1. The initial PTY error was `Invalid authentication terminal size`; the corrected PTY passed. Per-hop authentication was verified for the two jump-key prompts plus the final PAM prompt; the jump hosts themselves used public-key auth, while PAM was required at the final Linux target. No raw SSH result is promoted as a Crew result without the native CLI command and daemon status.
