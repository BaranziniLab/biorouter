## Problem and plain-language goal

Current file APIs can validate a pathname and subsequently open that pathname. Another process could replace a folder or symbolic link in the interval. The completed defensive audit fixed selected dangling-link and patch-path cases, but did not establish resistance to concurrent path replacement; see #446.

Open the authorized folder once and perform file operations through that open directory reference. This keeps operations attached to the folder that was authorized, rather than trusting that a pathname still refers to the same location.

This is a contained filesystem-layer refactor, not a rewrite of the agentic reasoning/planning loop. Keep existing tool names, user-facing file arguments, session concepts, and ordinary research workflows. The external-folder approval proposal in #424 decides **which roots are allowed**; this proposal governs **how file operations safely use those roots**.

## Existing pieces to build on

- Shared workspace resolution and SandboxClient interfaces in crates/biorouter-sandbox.
- DeveloperServer editor/path handling, FilesServer/Jail, scoped permission records, and existing action-required approval routing.
- The repository already uses cap-std directory handles in crates/biorouter-server/src/crew/local_files.rs, including no-follow directory walking and handle-relative publication/identity checks.

Extract or adapt suitable low-level primitives into a shared leaf crate rather than importing the server into MCP. Crew's sharing/private-folder rules and blanket symlink refusal are not the editor's product policy: reuse mechanisms without silently copying stricter policy.

## Proposed scope

- Introduce a shared handle-backed filesystem object for one or more explicitly authorized roots, with read-only/read-write access and session/request authority.
- Resolve absolute user paths to an authorized root plus a relative path, preserving the external tool API. No automatic expansion to an ancestor or implicit new root.
- Perform the actual open, parent creation, read, list, write, replacement, rename, and removal through bounded directory operations. Returning a checked PathBuf and then calling ordinary path-based I/O would leave the central race unresolved.
- Integrate direct editor writes, replacement/insertion, patch-selected child files, undo/history restores, and host-side sandbox file APIs. Audit every delegated library that opens a filename itself.
- mpatch currently performs filesystem effects. Adapt its I/O interface or separate parsing/content transformation from commits through the bounded filesystem API; do not validate and then hand it an unrestricted pathname.
- Define revocation, root identity changes, directory renames, internal symlink behavior, hard links, and concurrent execution deliberately. An open directory handle alone is not a complete security policy.
- Use native Unix and Windows directory/handle and link/reparse protections. Define behavior for unsupported filesystems; never silently fall back to unchecked opens while claiming the stronger guarantee.
- Preserve supported internal-link workflows where confinement can be guaranteed. A proposed restriction must be explicit and accompanied by a usable recovery path.
- Keep missing folder authorization recoverable through #424. A changed or ambiguous destination should yield an actionable result rather than making the agent guess or silently skip work.

## Staged PR plan

1. **Shared foundation:** define the small API and policy contract, adapt proven directory-handle primitives, and add deterministic path-substitution tests. No broad tool migration in this PR.
2. **Read/list adoption:** migrate supported sandbox/file-tool/editor read routes, with parity tests and cross-platform checks. Keep external schemas stable.
3. **Mutation adoption:** migrate creation, overwrite, replacement, undo, and patch child targets, with atomic-publication and no-outside-effects assertions. A patch's all-target preflight is separate from any promise of a transactional multi-file rollback.
4. **Complete integration and validation:** verify remaining executor paths, real editor workflows, cancellation/revocation, performance, and platform/filesystem compatibility. Document each backend's actual guarantee and residual limits.

Each PR should be reviewable on its own, run applicable Linux/macOS/Windows checks, and state which routes have migrated. Avoid a single claim of complete protection while a supported route still reopens unchecked path strings.

## Acceptance criteria

- [ ] Deterministically substitute a link or directory between resolution and the operation; reads/writes/creation must not escape the authorized root.
- [ ] Cover existing and new files, intermediate path components, multi-file patch children, undo, cross-root operations, rename/replacement, cancellation, and grant revocation.
- [ ] Preserve normal nested edits and documented internal symlinks without adding routine confirmation dialogs.
- [ ] Reuse explicitly approved additional roots from #424 without silently widening scope.
- [ ] Test Linux and macOS directory operations, Windows handles/reparse behavior, and declared network-filesystem support independently.
- [ ] Inspect remaining direct std/tokio/library pathname opens and verify real file-editing behavior, not only source/unit assertions.
- [ ] Keep generic shell/process containment as a separate OS-sandbox concern. This file-API refactor is not a new full-computer sandbox.

## Impact and status

Expected user-visible impact is mostly transparent file access; compatibility work centers on links, file replacement, patch/undo semantics, Windows file sharing, and filesystem behavior. This is a moderate-to-substantial internal refactor with broad executor coverage, not a tiny validation change or an agent-loop rebuild.

Related: #446 (audit finding and current local remediation), #424 (explicit external-folder approval and resume), #437 (trusted tool-result handoff).

Proposal only. No implementation, new PR, live race-test result, or completed platform-parity claim is included here.


### Windows directory-move compatibility in the current candidate

The historical proposal above predates the implementation candidate now being integrated through
[PR #454](https://github.com/BaranziniLab/biorouter/pull/454). Final acceptance remains pending;
this note describes its intended Windows semantics and the checks needed to establish them.

The Windows backend retains directory identity with delete sharing while idle and takes a
short move-preventing lease for operations that require stable cap-std lookup paths. This
supports an idle move of the exact captured directory; it does not promise arbitrary ancestor
moves while descendant directory capabilities remain open. Windows can refuse those ancestor
moves even if each child handle permits delete sharing. Finish or stop active work, revoke the
affected external grants, and retry after their holders release the handles. Other clients,
primary workspaces, or Developer roots can still hold descendants, so changing the workspace
or stopping the owning backend may be necessary. Do not promise immediate handle closure from
revocation or from closing a browser/manual external-backend interface. Preserve this OS
restriction rather than dropping authority or falling back to a fresh pathname open.
The synthetic native Windows diagnostic must distinguish exact-directory moves, retained-child
ancestor refusal, and successful retry after child close. These are acceptance criteria and
platform semantics, not a claim that a new CI run passed.
