# Testing

How this repository's tests are kept honest: what a test may safely touch, which shared state
it must not inherit, and the standing guards that stop a fixed hazard from quietly coming back.

Come here when a test fails in a subsystem your change never touched, when you are about to
write to the process environment from a test, or when you are adding a source-scan guard. This
folder is about the *properties* tests must hold. How to run a particular suite lives with the
subsystem it exercises, and the crate-level commands are in
[`CLAUDE.md`](../../CLAUDE.md).

| Document | What it covers |
|---|---|
| [Process-global state in the Rust workspace](process-global-state.md) | The audit of every process-global value the crates read and write — environment variables, `Paths::*`, statics frozen at first read, mutable registries — plus the ledger of unlocked readers, the two non-interchangeable remedies, and the recipes to re-measure it all. |

## Related documentation

- [How this documentation is organized](../organization.md) — where a document belongs, and how to add to this folder
- [System overview](../architecture/system-overview.md) — the crate boundaries the audit walks
