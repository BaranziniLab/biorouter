## Required enhancement: authorize a specific external folder, then resume

When an agent needs a legitimate file outside the session folder, BioRouter should ask the user for access to that specific folder and continue the original action after approval. A missing folder grant is a recoverable authorization request, not a generic terminal error, a silent skipped step, or a reason for the agent to declare the task impossible.

Example: while working in Project A, the agent needs to edit Project B/results.csv. Show “Allow access to Project B to edit results.csv?”, the exact folder, read-only versus read/write scope, and whether descendants are included. After approval, edit the original file in place. Do not require a new chat, moving the file, restarting the app, or selecting an unnecessarily broad common parent.

### Required behavior

- [ ] Before any external-folder read or mutation without an applicable existing user grant, pause the affected operation and present a first-class authorization request in desktop/browser and interactive CLI. A new external folder requires explicit user authorization even in Auto mode; a model, hook, tool annotation, or path pasted into chat cannot mint the grant.
- [ ] Display the requested folder, intended action, access level, descendants, duration, and affected files. Default to the narrow containing folder; let the user choose a different scope deliberately. Never silently broaden to a parent, home directory, or filesystem root.
- [ ] Offer **Allow once**, **Allow for this session**, and **Deny**. “Allow once” covers only the pending action and its disclosed files; a session grant covers matching later operations without repeated prompts. Keep optional persistent grants as a separately explicit choice.
- [ ] Keep the session's working directory unchanged. Authorize an additional root at the executor, so relative-path meaning does not unexpectedly change.
- [ ] After approval, revalidate the target and execute the original pending operation automatically, with stable request identity and at most one authorization-triggered replay. Resume the task with an accurate tool result. Do not replay partially completed or unrelated side effects.
- [ ] Deduplicate concurrent requests for the same folder/access scope. Revocation and cancellation must take effect before further operations; provide a visible list of authorized folders.
- [ ] A denial or cancellation stops that pending action and is communicated clearly. Do not repeatedly ask, broaden the requested scope, or switch to shell/computer tools to evade the decision. The agent can ask about a materially different alternative when appropriate.
- [ ] For other user-resolvable permission/configuration blockers, explain the required action and offer the appropriate supported recovery flow rather than silently abandoning the task. An OS denial may require OS settings; an app approval does not override it. Administrator policy, explicit never-allow, secret protections, and OS enforcement retain their own semantics.
- [ ] Without an interactive frontend, return a structured `authorization_required` outcome with actionable resume information and a bounded wait policy; do not hang indefinitely or corrupt JSON output.
- [ ] Approval must be visible when it arrives, even if the transcript was scrolled upward; reuse the one-shot request visibility work tracked by #444.

### Implementation direction

Reuse the existing PermissionConfirmation/action-required lifecycle and directory-scoped grant machinery. Extend it with a typed folder-access request carrying the session ID, pending action ID, normalized target folder, requested operations, and expiry. Bind the decision to the authenticated requesting session.

The existing remembered directory grant changes approval inspection only; it does **not** enlarge DeveloperServer's hard working-directory jail. The actual executors must enforce the union of the session root and explicitly granted additional roots, with operation-specific read/write scope. Adding only an approval card would leave the current failure in place.

Revalidate link resolution after approval. A replaced/deleted directory or changed symlink must not silently acquire broader access. Check every child target of multi-file patches before side effects; undo and cross-folder moves/copies must respect current permissions on every affected location. Use #452 (handle-backed file operations) for race-resistant execution.

Use consistent authorization meanings across structured file tools and explicit shell working-directory requests. Arbitrary shell file effects cannot be contained reliably by parsing command text: enforce subprocess boundaries through the actual OS sandbox when containment is required, and do not claim that an editor grant alone confines all shell or computer actions.

### Acceptance checks

- [ ] From Project A, request a read and then an edit in Project B; each required scope is presented correctly, approval continues in place, and the original working directory remains A.
- [ ] A session grant removes duplicate prompts for matching work; one-shot access does not authorize later unrelated files or writes.
- [ ] An absolute path alone grants nothing; a legitimate selected folder becomes usable after explicit authorization.
- [ ] Deny, cancel, revoke, reconnect, duplicate/concurrent requests, and changed link targets produce correct outcomes without retry loops or silent bypass.
- [ ] Ordinary in-workspace editing, nested creation, supported internal links, patches, and undo remain convenient.
- [ ] Exercise the real desktop approval-and-resume flow, interactive CLI, and the non-interactive structured outcome. Unit tests alone are not UX acceptance.

### Related work and status

This requirement consolidates the original outside-cwd usability report below. It complements #437 (trusted result-file handoff), #446 (path-boundary hardening), #447 (invocation-scoped automatic judgments), #117 (resumable user-input setup), and #444 (approval visibility).

This is an enhancement proposal requested by the user. It has not been implemented or validated by the completed defensive audit. Preserve the audit's local safety fixes while adding an intentional authorization path.
