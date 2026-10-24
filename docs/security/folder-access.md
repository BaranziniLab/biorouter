# External folder access

Built-in Developer file operations work inside the chat's workspace. When a file operation
needs another folder, BioRouter presents the specific folder, read-only or read/write access,
and the files needed by that pending operation. The same operation resumes after approval.
An absolute file path or a connector's returned result path does not itself grant access.

- **Allow once** covers the listed files and the required access for that operation. A patch's
  base-directory metadata can remain read-only while its named child files are writable.
  Directory analysis explicitly discloses read-only access to the chosen directory and descendants
  for that operation; it does not create a remembered folder grant or write access.
- **Allow for this session** includes the displayed folder and descendants in this chat, with
  the displayed access. It expires on revocation or deletion of the chat. In a managed desktop
  connection it also expires when the app quits or restarts. Browser and manually configured
  external-backend connections use the backend's lifetime; closing or restarting the interface
  does not revoke those service-owned grants.
- **Deny** leaves the files untouched. The agent cannot repeat or broaden that folder request
  during the same user turn. A new user turn may ask again.
- **Cancel operation** releases the pending operation without creating a grant. Stopping the
  turn also cancels its pending request.

Open **Folder access** beneath the chat composer to inspect and revoke additional folder
grants. Revocation is checked by operations that already hold a file capability as well as
future operations. It does not undo a completed edit. Read-only permission never silently
upgrades to write permission. Grants are not inherited by another chat.

In the local CLI or TUI, `/folder-access` lists the same session grants and
`/folder-access revoke <id>` revokes a selected grant. These commands act locally; they do not
send a permission request through the model.

A symbolic link that leaves the workspace follows the same approval flow. The card discloses
both the requested path and its discovered actual target. The pending operation uses a
dispatch-scoped alias and retained directory capability; changing the link while the card is
waiting does not redirect that operation to a newly selected folder. Later operations resolve
the link again and need approval if its target is outside their existing grants.

Secret-file rules, managed denials, user never-allow settings, and exact-call denials remain
in force. Folder approval does not grant shell execution, desktop control, or a general
ability to bypass another tool's permissions. An unattended route returns a structured
`authorization_required` response instead of waiting for a decision it cannot display.

## Desktop and backend lifetimes

A managed shared desktop connection holds a private, main-process lifetime connection to its
local daemon. Native grants belong to that client, even when two clients use the same chat.
Disconnecting the client revokes its session and pending once grants, releases its pending
folder requests, and leaves other active clients' grants and the shared daemon running.
After a lifetime disconnect, reconnecting obtains a new lifetime and requires fresh approval. An API secret or a renderer-
supplied lifetime identifier cannot claim another desktop client's grants. A shared daemon
that lacks this protocol must be updated or restarted explicitly before the desktop attaches;
the app does not silently fall back to grants without an owner.

The protocol applies to managed shared connections on supported Unix platforms. The existing
nonshared desktop mode, including the native Windows owned-daemon path, still starts a private
daemon for the application and stops that owned daemon on normal quit. It does not use the new
per-client shared-daemon protocol. A fresh owned launch does not reuse the prior process's grants.

A browser or a manually selected external backend remains a trusted service-session deployment.
Its grants are owned by that backend, not isolated by the new desktop lifetime protocol. Use
**Folder access** to revoke a service grant, delete its chat, or restart the backend. The interface
shows this backend lifetime, including for the environment-selected external development backend.
The primary workspace and ordinary in-workspace edits do not need additional folder approval.

On Windows, an idle captured directory can move while its own identity handle remains open.
Windows can still refuse moving an ancestor while a captured child directory is open, even
when the child permits delete sharing. Active file operations also hold a short move-preventing
lease. Finish or stop active work, revoke affected external-folder grants, and retry after the
holders release their child handles. Other grants, clients, the primary workspace, or long-lived
Developer roots may still retain handles; changing the workspace or stopping its owning backend
may be necessary. Closing only a browser or external-backend interface does not stop that backend.
No single action guarantees that every holder is gone. BioRouter does not drop authority or
reopen an unchecked pathname to force a move through this operating-system restriction.

Revocation prevents subsequent operations through retained file capabilities. It cannot cancel
an operating-system I/O operation that is already underway or undo a completed change.

## Browser setup

For a browser served by BioRouter, start from an interactive terminal:

```sh
biorouter serve --folder-access-approval
```

Choose a separate approval key of at least 16 characters in the hidden terminal prompt.
Enter that key only in the browser's **Folder approval key** field when allowing access or
revoking a grant. The browser clears the field after each decision, does not save it to
local storage or a chat, and sends it only to the folder decision route. The launcher sends
only its SHA-256 digest over the daemon's startup stdin pipe. Neither the raw key nor its
digest is supplied through an environment variable or a command-line argument.

This key authorizes only typed folder decisions and folder revocation. It cannot authorize
privacy, provider, extension, or Biorouter Copilot decisions. An API secret or browser access
cookie is insufficient. Failed key attempts are rate limited. A native desktop or interactive
local CLI uses its existing human decision channel.

The flag can be combined with `--computer-use-approval`; each scope has its own key. Without
folder approval configured, browser folder approval explains the required startup flag.
Restarting the server discards pending operations and session grants, so retry the original
file operation after restarting. Denial and cancellation do not require the folder key.

## Verification

`cargo test -p biorouter --test folder_access_authorization` exercises synthetic connector
result reads in place, exact once scopes, session grants, read/write escalation, revocation,
denial, cancellation, concurrent requests, session isolation, unattended recovery, and Unix
symbolic-link retargeting. Core authority tests ensure the folder proof cannot approve another
protected action. Server tests exercise scope separation and the startup digest protocol.
UI tests verify a refused key remains retryable, duplicate clicks produce one decision, and a
failed revocation remains visible. Actual model-driven app verification is a separate release
gate; unit-test results alone do not establish that it passed.

The desktop-lifetime correction is tracked by [#455](https://github.com/BaranziniLab/biorouter/issues/455),
following a real native quit/restart reproduction. Focused automated checks cover owner separation,
retained capabilities, pending cancellation, and scoped grant pruning. Actual headed app quit,
restart, abrupt disconnect, and reconnect checks against the final built artifact remain separate
acceptance gates; preparing the correction does not establish that they passed.
