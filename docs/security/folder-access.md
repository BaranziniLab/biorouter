# External folder access

> **What this is.** How a chat gets access to folders outside its working folder, and how you
> list and revoke that access in the desktop app, in a browser served by `biorouter serve`, and
> in the CLI.
> **Status:** Current.
> **Audience:** end users, and developers working on file access.

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
- **No answer** within 5 minutes (300 s) expires the request. The operation is not run, no grant
  is created, and the card shows **Expired: ask again** without its buttons. Ask again in the
  chat to retry. See
  [when nobody answers a card](permission-modes.md#when-nobody-answers-a-card).

A symbolic link that leaves the workspace follows the same approval flow. The approval card
discloses both the requested path and its discovered actual target. The pending operation uses a
dispatch-scoped alias and retained directory capability; changing the link while the card is
waiting does not redirect that operation to a newly selected folder. Later operations resolve
the link again and need approval if its target is outside their existing grants.

Secret-file rules, managed denials, user never-allow settings, and exact-call denials remain
in force. Folder approval does not grant shell execution, desktop control, or a general
ability to bypass another tool's permissions. An unattended route returns a structured
`authorization_required` response instead of waiting for a decision it cannot display.

## Seeing and revoking a chat's folders

In the desktop app, and in a browser served by `biorouter serve`, the name of the chat's
working folder (its workspace) sits beneath the chat composer. It is the folder's own name, as
the sidebar's folder groups show it: `~` for your home folder, and `Crew` for the folder Crew
tasks run in. Point at the name for a moment, or move focus to it with Tab, and a preview of a
card opens above the composer. It never covers the draft or the composer's controls. The card
has two sections:

- **Working folder** shows where the folder is. On Home, and in a chat that has no messages
  yet, a **Choose folder…** button in this section picks a different working folder. Once the
  chat has messages its working folder is fixed, and the section says so instead.
- **Other approved folders**, in a chat only, lists every other folder allowed for this
  session. Each one shows its access (read only, or read and write), whether it includes
  subfolders, and a **Revoke** button. Folders allowed once are not listed, because that
  approval covers only its pending operation.

The card ends by saying what folder approval covers. The file editor, file analysis and image
tools ask before they use another folder. Shell commands and document tools do not ask.

Every folder in the card is written in the card's ordinary text, in one colour. In the desktop
app, `~` stands for your home folder, so a folder in it reads `~/Desktop/BioRouter`, and a folder
anywhere else is written in full. In a browser served by `biorouter serve`, and when the desktop
app uses an external backend, every folder is written in full, because it may be on another
computer whose home folder is not yours. A long path wraps after a `/` or a `\`, not in the middle
of a folder's name, and selecting it copies exactly what it shows. Each **Revoke** button reads its
folder's full path to a screen reader. The folder approval card in the chat is different on
purpose: there the folder is the exact thing you are allowing, so it is written in full in a
monospaced face, `~` never stands in for it, and it still wraps after a separator.

- A card that a hover or Tab opened is a preview. It closes when the pointer leaves both the
  name and the card, or when focus moves away from the name, and the name looks as it does at
  rest.
- Click the folder name, or press Enter or Space while it has focus, to keep the card open.
  This works in every state, including Home and a chat before its first message, and the name
  then stays shaded until the card closes. A second click closes it. Opening it from the
  keyboard also moves focus into the card, so Tab reaches **Choose folder…** or each
  **Revoke** button. A press inside the card keeps it open too.
- A kept card stays open when the pointer moves away, when the page scrolls and when you switch
  to another window. Escape, or a click outside the card, closes it, as does focus moving to
  something else in the page. Escape, and Shift+Tab from the card itself, return focus to the
  folder name; a click outside leaves focus where you clicked.
- While the system folder chooser that **Choose folder…** opens is on screen, no card shows.
- While an existing chat's working folder has not loaded, a folder icon without a name stands
  in for it and opens the same card. The Working folder section reads "Not known yet.", and
  the chat's grants stay listed and revocable.
- In the desktop app, ⌘-click on macOS, or Ctrl-click on Windows and Linux, on the name opens
  the folder in the file manager and does not keep the card. In a browser served by
  `biorouter serve` there is no file manager to open, so the same click keeps the card like any
  other click.
- The context ring and the cost beside it open cards that behave the same way. One card is
  open at a time across the footer: a preview gives way to the next one you point at, but while
  you have kept a card open, pointing at another item opens nothing, so the kept card and
  anything you typed in it stay. Click the other item, or Tab to it, to switch.
- In a browser, the Other approved folders section also holds the **Folder approval key**
  field once the chat has a grant to revoke. See [Browser setup](#browser-setup).

Revocation is checked by operations that already hold a file capability as well as
future operations. It does not undo a completed edit. Read-only permission never silently
upgrades to write permission. Grants are not inherited by another chat, with the one exception
in [Subagents](#subagents).

## Subagents

A subagent is a chat of its own, so it has its own working folder and its own grants. Two rules
connect it to the chat that started it (`DelegatedChat` in `crates/biorouter/src/folder_access.rs`):

- **Its folder card appears where you are watching.** A card a subagent raises waits in the
  subagent's own session, which nobody may be looking at, since a background subagent has no tab.
  It is also shown in the chat you are watching (the top of the chain when work was delegated more
  than once), through the same relay as the subagent's other approval cards, and it is one
  decision: an answer in either place decides it. This widens who can see and click
  the card, never who may decide: a folder decision still needs a person's proof. A denial ends
  the subagent's call with "Folder authorization required for <folder>. … Access was not
  approved."; a card nobody answered ends it with "… The request expired before anyone answered
  it, so access was not given." The card names who asks. In the chat that delegated the work, the daemon puts the asker on the
  copy it shows there (`preview.requestedBy`: the subagent's role, else its chat's name), so the card
  reads "Subagent 'reviewer' asks to read ~/data" and never looks like the chat's own request. The
  card's home copy in the subagent's own session carries no `requestedBy`, and the subagent's tab
  names the same subagent itself; in a chat's own agent the card reads "Allow this chat to read
  ~/data?". Every card shows the folder as the call asked for it, with `~` for home, keeps the exact
  paths and the grant's terms under **Details**, and collapses to a transcript row once answered.
- **A direct subagent uses its parent's session grants.** The folders you allowed with **Allow for
  this session** in a chat also cover the subagents that chat starts, when the subagent is at the
  same privacy tier. They are the parent's own grants, read on each request, so revoking one in
  the parent ends it for the subagent as well. Nothing else is inherited: not a grandparent's
  grants, not a grant given in a sibling's or the subagent's own card (which stays on that
  subagent), not an Allow once, not the parent's working folder, not grants owned by another
  desktop connection, and nothing for an agent-created chat that is not a subagent or a subagent
  at another tier. Every session grant comes from a person, because a folder card requires a
  person's proof. The inherited folders are not listed in the subagent's own working-folder card;
  they are listed, and revoked, in the parent's.

In the local CLI or TUI, `/folder-access` lists the same session grants and
`/folder-access revoke <id>` revokes a selected grant. These commands act locally; they do not
send a permission request through the model.

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
Its grants are owned by that backend, not isolated by the new desktop lifetime protocol. To end
a service grant, revoke it from the working folder's card beneath the composer, delete its chat,
or restart the backend. The folder approval card and the working folder's card both state this
backend lifetime, including for the environment-selected external development backend.
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
revoking a grant. When allowing access, the field is on the folder approval card in the chat.
When revoking, it is in the working folder's card below the grants, and **Revoke** stays
unavailable until a key is typed.

The browser clears the field after each decision or revocation attempt, does not save it to
local storage or a chat, and sends it only with a folder decision or a revocation request. The
launcher sends only its SHA-256 digest over the daemon's startup stdin pipe. Neither the raw key
nor its digest is supplied through an environment variable or a command-line argument.

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
protected action. Server tests exercise scope separation and the startup digest protocol. `cargo test -p biorouter --lib folder_access::delegation_tests` covers subagents: a
subagent's card shown and answered in the parent, the denial and expiry sentences, a parent's
person-given session grant covering its direct subagent, no other chat inheriting one, and a
subagent at another tier not inheriting. `a_subagents_file_call_uses_its_parents_person_given_session_grant`
(`agents/extension_manager.rs`) holds the dispatch path to it.
UI tests verify a refused key remains retryable, duplicate clicks produce one decision, and a
failed revocation remains visible. `bottom_menu/FolderAccessCard.test.tsx` and, in Chromium,
`bottom_menu/footerCard.browser.test.tsx` cover how the card writes a folder (the `~` form, the
full path for a backend that is not this machine's, where a long path wraps, right-to-left names,
what a selection copies) and the modifier click on each platform and in a browser;
`bottom_menu/footerCards.contract.test.tsx` covers preview and kept cards;
`ToolCallPreview.folderAccess.test.tsx` holds the card's Details to the exact path. Actual model-driven app verification is a separate release
gate; unit-test results alone do not establish that it passed.

The desktop-lifetime correction is tracked by [#455](https://github.com/BaranziniLab/biorouter/issues/455),
following a real native quit/restart reproduction. Focused automated checks cover owner separation,
retained capabilities, pending cancellation, and scoped grant pruning. Actual headed app quit,
restart, abrupt disconnect, and reconnect checks against the final built artifact remain separate
acceptance gates; preparing the correction does not establish that they passed.

## Related documentation

- [Permission modes](permission-modes.md): the tool approvals that still apply inside the working folder.
- [Secret guard](secret-guard.md): the credential file rules that a folder grant never overrides.
- [Reaching Biorouter from a browser](../deployment/browser-access.md): starting `biorouter serve` and what a browser session can and cannot do.
- [The shared controls](../desktop-ui/shared-controls.md#composer-footer-cards): how the working folder's card is built, writes a folder, and answers a hover and a click.
- [External folder authorization requirements](assessment-2026-10/follow-up-proposals/external-folder-authorization.md): the requirements (#424) this behaviour was built to.
