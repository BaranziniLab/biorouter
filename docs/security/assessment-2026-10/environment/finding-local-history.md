# Separate-account access to session history


**Status:** confirmed in three fixtures using the official v1.93.4 Linux/amd64 release. The assessment branch contains an unmerged, unreleased source fix. Local owner-workflow regressions and a separate Linux/arm64 two-UID helper check passed. A full rebuilt Linux application's separate-account SQLite scenario has not been rerun. Tracked by [issue #445](https://github.com/BaranziniLab/biorouter/issues/445).

## Baseline conditions and impact

The owner's home was traversable (mode `0755`) with umask `0022`. App-created session storage was mode `0755`; the SQLite database and sidecars were readable by other accounts. UID 10001 created and named a session through ordinary authenticated application routes. UID 10002 then opened the actual SQLite database read-only and recovered the synthetic title. This succeeded in all three final fixtures without a daemon secret, root privileges, or the owner's UID.

The outsider was denied the separately tested synthetic secret, process-environment, and configuration-write accesses. The observed disclosure was a synthetic session title. The readable store also contains conversation data, but no PHI or real conversation content was used. A non-traversable ancestor such as a mode-0700 home blocks the demonstrated path. Same-account processes are a different trust boundary.

## Source change and evidence

`SessionStorage::create_pool` protects the session directory before opening SQLite. On Unix it opens the final directory with `O_NOFOLLOW | O_DIRECTORY`, verifies effective-UID ownership, and applies mode `0700` through the opened handle. This protects existing database and sidecar names through directory traversal restrictions. A foreign-owned or final-symlink directory is refused.

Local Darwin/arm64 application regressions checked both new and existing permissive directories and retained owner session creation, reading, closing, and reopening. The Linux/arm64 helper test compiled the production guard verbatim with the locked libc version. In three fixtures, a permissive synthetic history file was readable by the observer before the guard, remained readable by the owner afterward, and became unreadable to the observer. Additional checks refused a foreign-owned directory and a final symlink without changing its target's mode.

The original runtime baseline used Linux/amd64 under Docker Desktop x86 emulation. The helper used Linux/arm64 and a synthetic file, rather than a complete rebuilt BioRouter/SQLite runtime. Its evidence must not be described as that end-to-end retest.

## Recommendations preserving normal use

Keep owner-only session storage an automatic default and repair overly broad app-owned directory permissions during normal startup. Scope the repair to BioRouter-owned history; preserve intentionally shared research-workspace permissions. Provide a clear actionable error when the sensitive directory cannot be protected, without adding routine approval clicks or passwords.

Implement and verify equivalent owner-only Windows ACL behavior before claiming platform parity. Verify a rebuilt Linux package with two accounts before release integration is described as complete. Already-open outsider file descriptors are not revoked by chmod, and this change does not protect against other processes running as the owner.

## Evidence locations

- `environment/local-summary.json`, `local-results.csv`, and the local raw repetition folders contain baseline outcomes and permission snapshots.
- `environment/session-guard-regression/` contains extracted source, hashes, build output, and the two-UID helper summary.
- `tool-boundary/raw/post-change-core-regressions.log` contains the owner SQLite and unsafe-storage regressions.
- The recorded source patch contains the guard and tests; the official v1.93.4 package itself remains unchanged.

Compatibility limit: final-symlink session directories are refused, including manually symlink-relocated stores. The existing constructor uses a panic-based error path for this refusal. A recoverable, actionable startup message and safe relocation support remain recommendations before broad deployment.
