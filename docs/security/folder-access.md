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
  the displayed access. It expires on revocation, deletion of the chat, or process restart.
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
