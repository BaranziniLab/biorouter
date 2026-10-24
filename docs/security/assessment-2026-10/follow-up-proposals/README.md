# Security follow-up proposals

These proposals were recorded after the controlled defensive security assessment. They are design requirements, not implemented changes or new audit results.

- [#424: External folder access: ask for scoped permission and resume file operations](https://github.com/BaranziniLab/biorouter/issues/424) — [local requirements](external-folder-authorization.md). The original bug report and reproduction are preserved in the GitHub issue.
- [#452: Anchor file operations to authorized directory handles](https://github.com/BaranziniLab/biorouter/issues/452) — [local design and staged PR plan](handle-backed-filesystem.md).

The first decides which additional folders the user authorizes. The second keeps actual file operations attached to authorized directory objects and prevents validation from being followed by unrestricted pathname-based opens. Neither requires rebuilding the agent's planning loop.

Related integration issues: [#446](https://github.com/BaranziniLab/biorouter/issues/446) (audit path finding), [#437](https://github.com/BaranziniLab/biorouter/issues/437) (trusted result-file handoff), [#444](https://github.com/BaranziniLab/biorouter/issues/444) (visible permission requests), and [#117](https://github.com/BaranziniLab/biorouter/issues/117) (resumable user-input setup).

The local audit fixes remain at commit `5c5dd82b96c9432e65ec1a114b68c52ab1698323`. These follow-up documents do not change or expand the frozen assessment evidence, manuscript results, or platform-validation claims. No implementation PR is created by recording these proposals.
