# Linux synthetic SSH keyboard-interactive probe

Run on 2026-09-22 against a separate disposable Docker container, created
from `rust:latest` and published only as `127.0.0.1:56929 -> 22`. Container
ID was `831cc19a53db`. It was independent of the existing Crew SSH fixture
(`127.0.0.1:56928`) and was removed after the probe.

The container installed only test infrastructure (`openssh-server` and
`procps`), created user `mfa` with UID 1000, and configured:

```text
AuthenticationMethods publickey,keyboard-interactive:pam
PasswordAuthentication no
KbdInteractiveAuthentication yes
UsePAM yes
```

The PAM password is a synthetic fixture secret. It is not Duo, TOTP, an
institutional identity provider, or evidence of production MFA behavior.

| Probe | Result | Evidence |
| --- | --- | --- |
| Public key only with `BatchMode=yes` | PASS (rejected) | Server accepted the key with partial success, then client failed because keyboard-interactive required a prompt |
| Public key + synthetic PAM prompt | PASS | `expect` answered `Password:` and remote `id -u` returned `1000` |
| Wrong synthetic secret | PASS (rejected) | Three PAM attempts were exhausted; no `should-not-run` command output occurred |
| Prompt cancellation | PASS (cancelled) | Ctrl-C at the PAM prompt closed/returned without command output |
| Encrypted key passphrase + synthetic PAM prompt | PASS | `expect` answered the private-key passphrase, then PAM `Password:`; remote UID returned `1000` |
| Secret non-persistence | PASS for this fixture | `/tmp/sshd.log` contained neither synthetic PAM secret nor key passphrase |

The OpenSSH client used strict pinned host-key verification, `IdentitiesOnly`
and `IdentityAgent=none`. This validates native OpenSSH keyboard-interactive
and encrypted-key prompt behavior on Linux. It does not validate BioRouter's
desktop prompt plumbing, MFA provider semantics, or institutional policy; no
BioRouter binary or source was copied into the container.
