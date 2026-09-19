# Computer Use capability

Computer Use (`computercontroller`) observes and controls applications through BioRouter's bundled native helper. It is the only built-in desktop observation and control capability. Web fetching and document processing are provided separately by [Web & Documents](web-documents.md).

## Tools

| Tool | Purpose |
| --- | --- |
| `list_apps` | Discover available applications. |
| `get_app_state` | Inspect an application's current accessibility state and image. |
| `click` | Click a current target. |
| `perform_secondary_action` | Invoke a target's supported secondary action. |
| `scroll` | Scroll a target. |
| `drag` | Drag between supported targets. |
| `type_text` | Enter text. |
| `press_key` | Send a supported key or key combination. |
| `set_value` | Set an accessible control's value. |
| `screen_capture` | Capture a display/window or request list-only discovery through the native helper. |

Use the advertised schemas for platform-specific arguments. Discover, inspect, act, and verify; refresh state after a changed window, stale element, handoff, or helper restart. Actions are sequential. A timed-out action can already have happened, so inspect before any retry.

`scroll.pages` accepts finite values greater than zero and no larger than 100, including fractions. `click.click_count` accepts whole numbers from 1 through 100. Both default to one only when omitted; malformed supplied values produce an error before an action.

Scrolling uses the target viewport where the platform exposes measurable geometry. Windows verifies UI Automation or native scrollbar position. Linux text views can align only to a line or character boundary; the result reports the observed movement and granularity. Unsupported fractional or horizontal scrolling fails explicitly. For a Linux nontext control that supports only vertical page keys, the result identifies the delivered commands and states that actual displacement is unverified. Inspect fresh state to confirm the outcome.

## Approval and privacy

Enable Computer Use in Settings → Chat → Capabilities or `biorouter configure`. Enabling it does not approve desktop access. Before the first observation or action, BioRouter requests approval for the task, model/provider, and target computer. The grant covers all tool turns within the current user request without a prompt for every click or capture. Completion or cancellation ends it; a new user request is a new task. Stop or revoke prevents further actions. OS accessibility and capture permissions are separate prerequisites.

Public-model approval discloses that screenshots and app text can be sent to the provider. Private-model approval names the actual provider/deployment; private does not always mean on-device. Each chat has separate results, snapshots, element references, and approval. Private and public chats never share stored observations or grants. They still operate the same physical desktop: material left visible can appear in a new capture. A private-to-public handoff therefore pauses for acknowledgement before capture.

The target is the backend host. A browser connected to `biorouter serve` does not grant access to the browser user's computer. Missing runtime, denied OS permission, no desktop, and unsupported actions produce explicit errors; never bypass them with a script or loop on approval requests.

## Platform prerequisites and current limits

The helper operates in the signed-in desktop session of the **host running BioRouter's backend**.
A remote browser connection does not expose the browser user's machine. These prerequisites do
not guarantee that every application, display server, or packaged environment supports every action.

| Platform | Prerequisites | Current limits |
| --- | --- | --- |
| macOS | macOS 14 or later; Accessibility and Screen Recording permission for the native helper. | Earlier macOS versions are unsupported. Signed-package installation, Intel execution, and permission continuity after upgrades require separate release validation. |
| Windows | An interactive signed-in desktop; Windows PowerShell and .NET UI Automation available under local policy. | No UAC/secure-desktop control or automatic elevation. Window capture uses visible screen pixels, so overlapping windows can appear in the result; minimized windows may be unavailable. |
| Linux X11 | An active X11 desktop and user D-Bus session; Python 3, GI bindings, AT-SPI, and GDK 3/GTK 3 dependencies. | Application accessibility support varies. Window capture uses visible screen pixels and may include overlapping windows; minimized windows may be unavailable. Xvfb alone does not establish working accessibility or input. |
| Linux Wayland | Accessibility discovery depends on the compositor, user session, and AT-SPI support. | **Pixel capture is currently unsupported.** The helper reports that a consented desktop portal is required; list-only discovery is not proof that capture or input works. |
| Headless service/container | A desktop session and its dependencies would need to be explicitly available to the backend. | No desktop means no computer use. Ordinary chat and Web & Documents remain independent. |

Use the runtime's diagnostics and returned errors to determine what is available on the actual
host. Native unit tests and a successful protocol handshake do not establish cross-platform GUI
or installer support; the [implementation status](../../design/computer-use-implementation-status.md)
records those validation gaps.

## Native runtime and replacement

Computer Use works independently of Developer and Web & Documents. The native helper exposes all ten tool contracts without a runtime npm/install bootstrap; each operation still requires the platform prerequisites above. Platform support and GUI release evidence are tracked in the [implementation status](../../design/computer-use-implementation-status.md).

The previous script-driven controller and Developer capture tools have been removed. There are no compatibility aliases or script fallback routes. Update stored workflows to current native tools; an old tool approval does not grant the new capability.

## Related documentation

- [Developer](developer.md): code, shell, and file tools.
- [Web & Documents](web-documents.md): URL and document utilities.
- [Permission modes](../../security/permission-modes.md): ordinary tool approval behavior.
- [Integration plan](../../design/computer-use-integration-plan.md): runtime, isolation, packaging, and validation requirements.
