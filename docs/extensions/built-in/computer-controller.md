# Biorouter Copilot capability

> **What this is.** The reference for Biorouter Copilot, the built-in capability that observes and
> controls native apps: its tools, what it refuses, how approval works and what each platform needs.
> **Status:** Current for Biorouter 1.94.6, where Copilot runs on the agent-desktop engine.
> **Audience:** people using Biorouter Copilot, and developers changing it.

Biorouter Copilot (`computercontroller`) observes and controls native applications on the computer that runs Biorouter's backend, through Biorouter's bundled native engine. It is the built-in capability for native applications and the desktop. Web fetching and document processing are provided separately by [Web & Documents](web-documents.md).

Biorouter Copilot operates native apps, the operating system and file dialogs on this computer. For anything on a web page (open, search, read, fill, click, drag, download) use the [Biorouter Browser](browser.md). Use Copilot on a web browser only when the person asks for their own browser window, tabs or profile. A decline in one capability is not consent in the other, and one Stop ends both.

## Tools

| Tool | Purpose | Needs |
| --- | --- | --- |
| `list_apps` | List the running apps Copilot can observe. | Looking |
| `get_app_state` | Read an app's window as lines of references (such as `@s1abc:e4`), roles, names and values, optionally with a screenshot. | Looking |
| `find_elements` | Find elements in a large window by role, name, text or value. | Looking |
| `wait_for` | Wait up to 30 seconds for an element, its disappearance, some text or a window title. | Looking |
| `screen_capture` | Screenshot one app's window, a window by id or a display, or list windows and displays. | Looking |
| `scroll` | Scroll an element by reference. | Looking (control on Linux X11, where it moves the real pointer) |
| `perform_secondary_action` | Expand, collapse or scroll an element into view (looking); show its menu, toggle, check, uncheck, select, focus or clear it (control). | Looking or control |
| `click` | Click a reference, or click at pixels of the app's latest screenshot. | Control |
| `drag` | Drag and drop with the real pointer, between references or screenshot pixels. | Control |
| `type_text` | Type text into a text field by reference. | Control |
| `press_key` | Press a key or combination in an app. | Control |
| `set_value` | Set an element's value directly. | Control |
| `open_app` | Open an app by name or bundle identifier, or bring it forward. | Control |

Observe first, act only on references from the latest observation, and observe again after anything changes. A reference from an older observation, another request or another chat is refused. Every action result says whether it was delivered and verified; an action whose outcome was uncertain, timed out or was stopped is never repeated, and the next action on that app needs a new observation.

Some actions use physical input: a double or triple click, a click at coordinates, `drag`, and on some systems typing, focus or scrolling. They move the person's real pointer and can bring a window forward. Biorouter decides this from a fixed table per operating system; the model has no option to change it, and a failed action is never retried with physical input.

### Drag and drop

`drag` presses at the source, moves and releases over the destination with real pointer events. Within one app it works on macOS, Windows and Linux X11. On Linux X11 it is measured: dragging a list item onto another reorders the list, and text dragged from one window lands in a text field of another ([fixture acceptance](../../testing/copilot-fixture-acceptance.md)); the Windows and macOS drives have not run yet. Between two apps some targets ignore it, and dropping a file from Finder or Explorer works only as the same gesture on the file. With `verify` (a `role:text` such as `listitem:Report.txt`), the result says whether that element appeared at the destination; without it, observe the destination before relying on the drop. Linux Wayland has no pointer input, so `drag` is unavailable there. Drag and drop on a web page is the Biorouter Browser's `browser_drag`.

## What Copilot refuses

These are checked before anything runs, whatever the person allowed:

- Biorouter's own windows and processes, including the terminal running the Biorouter CLI. A display screenshot is refused while a Biorouter window is on that display; capture an app window instead.
- The lock screen, sign-in and secure desktops, and system security and password prompts.
- Password fields: Copilot never types into, sets or clears them, and never reads their values.
- Keys and text while macOS secure keyboard input is on.
- Text that looks like a secret (private keys, cloud keys, provider credentials).
- Typing, keys or drops in terminal apps, which would run commands outside the Developer tools.
- Changes in privacy, security, users and sign-in settings; Copilot can still read them.
- The clipboard, notifications, quitting apps, rearranging windows and launching an app with options or a debugging port: none of these is in the tool set.

## Approval and privacy

Enable Biorouter Copilot in Settings, Chat, Capabilities, or with `biorouter configure`. Enabling it does not approve desktop access. On the first Copilot call of a request, Biorouter asks the person, naming the model, provider, destination and computer. The card above the composer offers **Allow** (look and control), **Only let it look** and **Cancel**; the terminal asks the same and starts on Deny. While Copilot is active the card says which was allowed. Under look only, a call that would change anything fails with a sentence telling the model to describe the change instead, and no second card appears. The grant lasts until the reply finishes or the person stops it; a new request asks again. Only a Biorouter window that holds the chat shows the card, so in a chat no window holds (one driven by `biorouter session send` or by another chat, or a subagent whose tab is not open) a call that would need the card is refused at once with that reason, rather than waiting for an answer nobody can give. Stop or revoke ends the running command, gives an interrupted gesture a moment to release the pointer, and deletes the request's stored observations. Operating system Accessibility and Screen Recording permissions are separate prerequisites, granted to Biorouter Copilot and never to Biorouter itself.

Public model approval says that screenshots and app text go to the provider. Private model approval names the actual provider and deployment; private does not always mean on the device. Each chat and request has its own stored observations and references, deleted when the request ends. Private and public chats never share observations or grants, but they operate the same physical desktop, so material left visible can appear in a new capture, and a private to public handoff pauses for acknowledgement before capture.

The target is the backend host. A browser connected to `biorouter serve` does not grant access to the browser user's computer. A missing runtime, a denied permission, no desktop or an unsupported action produces an explicit error; never bypass it with a script or loop on approval requests.

## Platform prerequisites and current limits

| Platform | Prerequisites | Current limits |
| --- | --- | --- |
| macOS | macOS 13 or later; Accessibility for Biorouter Copilot, and Screen Recording for screenshots. **Request access** in Copilot's setup has macOS ask for both by name; it may also ask about System Events, which Copilot does not use, so that one can be refused. | Permission continuity after upgrades and signed package installation require release validation. |
| Windows | An interactive signed-in desktop. | No UAC, secure desktop or sign-in screen control and no automatic elevation; an app running as administrator can be read but not driven. |
| Linux X11 | An active desktop session with its session bus and `at-spi2-core` 2.52 or newer (Ubuntu 24.04 or later). | Older accessibility bridges can crash the controlled app, so Copilot reports itself unavailable there. Application accessibility support varies. |
| Linux Wayland | As for X11. | No pointer input: coordinate clicks and `drag` are unavailable. Keys need the compositor's remote desktop support, which asks the person. |
| Headless service or container | A desktop session explicitly available to the backend. | No desktop means Biorouter Copilot is unavailable. Ordinary chat and Web & Documents remain independent. |

Use the readiness check in Settings or `biorouter doctor` to see what is available on the actual host.

## Native runtime

Biorouter Copilot works independently of Developer and Web & Documents. Its engine is built from a pinned, vendored copy of an open source desktop automation tool (see `NOTICE`) and runs once per operation with a cleared environment and a private working folder for the request. Biorouter 1.91 to 1.94.5 used a different engine, Open Computer Use, retired in 1.94.6; after an update on macOS, Biorouter ends that engine's background process if it is still running from the same installation. The previous script-driven controller and Developer capture tools have been removed, with no compatibility aliases or script fallback. Stored workflows that used element indexes must use references from `get_app_state` instead.

## Related documentation

- [Developer](developer.md): code, shell, and file tools.
- [Web & Documents](web-documents.md): URL and document utilities.
- [Biorouter Browser](browser.md): web pages in the chat's side panel, the capability Copilot hands web work to.
- [Permission modes](../../security/permission-modes.md): ordinary tool approval behavior.
- [Copilot on agent-desktop](../../design/copilot-agent-desktop.md): the design of the engine, the safety layer and the tests.
- [Biorouter Copilot fixture acceptance](../../testing/copilot-fixture-acceptance.md): how Copilot is driven against real fixture apps, drag and drop included, and what has been measured.
