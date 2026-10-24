# Private session history and deliberate relocation

BioRouter keeps session history in the app data directory's `sessions` folder. Before connecting to SQLite, it requires a real directory owned by the current account. It restricts Unix directory access to that account, clears macOS extended ACLs on that app-owned directory, and applies a protected current-user DACL to Windows session storage, including existing history files and sidecars. Filesystems that cannot enforce these permissions produce a recovery message; BioRouter does not silently continue with public history.

Unsuitable storage now returns an actionable error rather than panicking during construction. The daemon checks storage before serving requests. Other hosts check it before the first database connection, and retry preparation after a failed attempt so a corrected folder can be used without replacing the session manager.

For deliberate relocation, use the existing `BIOROUTER_PATH_ROOT` setting with a real, absolute app-root directory. BioRouter uses `<app-root>/config`, `<app-root>/data`, and `<app-root>/state`; history is `<app-root>/data/sessions`. This setting relocates the whole app root, so use the same value for the desktop daemon and CLI.

1. Close all BioRouter processes that use the old root. Keep a backup of the actual app folders.
2. Copy the configuration, data, and state folders to the chosen real app root while BioRouter is stopped. Copy the complete SQLite store together; do not copy a live database without its active WAL/sidecar files.
3. Set `BIOROUTER_PATH_ROOT` to the chosen root in the environment that launches BioRouter. A desktop launch must inherit that environment too.
4. Restart BioRouter and verify existing history before removing the backup.

If an earlier setup used a `sessions` symlink or Windows junction, point the app root at real storage instead. BioRouter does not infer authorization from a link, follow it to tighten permissions on another folder, or change ownership of a foreign account's directory. Keep the old link/backup until the copied or explicitly selected real store has been verified.

These permissions isolate ordinary separate local accounts. They do not revoke an already-open descriptor, isolate processes running as the same account, or protect against an administrator controlling the host. Existing audit evidence remains tied to its recorded version and environment; the Windows implementation and rebuilt-application behavior need their own validation.

The integration tests are `cargo test -p biorouter --test session_storage_security --test security_assessment`. Windows also runs `cargo test -p biorouter --lib session::storage_permissions::tests`; macOS exercises an explicit extended-ACL fixture. To verify a rebuilt Linux daemon, run `python3 scripts/security-assessment/session_storage_rebuilt.py --daemon /absolute/path/to/biorouterd --image <existing-compatible-runtime-image> --source-commit <build-commit> --output <new-evidence-directory>`. That runner uses isolated read-only containers, synthetic history, two non-root accounts, and a read-only mount of the supplied binary. It checks fresh storage and repair/reopening of an intentionally permissive legacy fixture. It does not reuse or overwrite frozen audit evidence.
