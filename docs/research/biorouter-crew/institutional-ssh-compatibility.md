# Institutional SSH compatibility probe

Probe time: 2026-09-22T10:43:38Z (UTC)

These are read-only metadata and kernel capability probes against the two
authorized saved SSH targets. They do not install software, write files, read
private file contents, or change host configuration. Connections used
`StrictHostKeyChecking=yes`, `BatchMode=yes`, `ForwardAgent=no`,
`ConnectTimeout=15`, and `ConnectionAttempts=1`.

The saved SSH identities authenticated without an MFA prompt because batch mode
disallows interactive prompts. This records “no MFA prompt observed”; it is not
evidence that MFA is absent or that an institutional MFA policy was verified.

## Metadata command

```sh
ssh -o StrictHostKeyChecking=yes -o BatchMode=yes -o ForwardAgent=no \
  -o ConnectTimeout=15 -o ConnectionAttempts=1 HOST \
  'printf "uid="; id -u; printf " user="; id -un; \
   printf " uname="; uname -srmo; printf " python="; command -v python3 || true; \
   python3 --version 2>/dev/null || true; printf " filesystem="; \
   df -P -T "$HOME" 2>/dev/null | tail -n 1'
```

Results:

| Target | Result |
| --- | --- |
| `wagu@narrows-login.sdsc.edu` | UID `1135`, user `wagu`; Linux `4.18.0-553.64.1.el8_10.x86_64`; Python `/salemlab/users/wagu/miniconda3/bin/python3` 3.9.16; home filesystem NFS `qs-rits.sdsc.edu:/salemlab`, mounted at `/salemlab`, 66% used; exit 0. |
| `wanjun@leo.ucsf.edu` | UID `1020`, user `wanjun`; Linux `5.15.0-141-generic x86_64`; Python `/pool1/home/wanjun/anaconda3/bin/python3` 3.12.7; home filesystem ZFS `pool1/home`, mounted at `/pool1/home`, 1% used; exit 0. |

## Kernel capability command

The following Python command invokes only the read-only ABI/version query for
`landlock_create_ruleset` (x86_64 syscall 444) and opens a pidfd for the
current process (x86_64 syscall 434), then closes that descriptor. It does not
apply a sandbox or inspect another process.

```sh
python3 -c 'import ctypes,json,os; libc=ctypes.CDLL(None,use_errno=True); libc.syscall.restype=ctypes.c_long; a=libc.syscall(444,None,0,1); ae=ctypes.get_errno() if a<0 else 0; p=libc.syscall(434,os.getpid(),0); pe=ctypes.get_errno() if p<0 else 0; (os.close(p) if p>=0 else None); print(json.dumps({"landlock_abi":int(a),"landlock_errno":ae,"pidfd_open":int(p>=0),"pidfd_errno":pe}))'
```

Results:

| Target | Result | Interpretation |
| --- | --- | --- |
| `wagu@narrows-login.sdsc.edu` | `{"landlock_abi": -1, "landlock_errno": 38, "pidfd_open": 0, "pidfd_errno": 38}` | `ENOSYS`; these syscall entry points are unavailable to the process/kernel. |
| `wanjun@leo.ucsf.edu` | `{"landlock_abi": 1, "landlock_errno": 0, "pidfd_open": 1, "pidfd_errno": 0}` | Landlock ABI 1 and pidfd opening are available. This does not establish support for every later Landlock feature or a production policy. |

The Narrows result is a capability blocker for a Linux helper that requires
Landlock or pidfd-based lifecycle control. The Leo result is only a positive
primitive check; the actual helper policy still needs a separate approved test.
