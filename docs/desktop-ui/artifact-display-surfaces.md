# Where a generated artifact is displayed

> **What this is.** The rule that an Auto Visualiser figure, an Agent Drafter app card, or any other artifact the agent produces is displayed in exactly one place — the artifact side panel — on every surface that shows a transcript, and the record of what was removed to make that true.
> **Status:** Current.
> **Audience:** contributors working on the desktop renderer.

## The rule

A generated artifact has **one** display surface: the right-hand artifact side panel
(`ui/desktop/src/components/artifacts/ArtifactViewer.tsx`). In the transcript it appears as a
click-to-open card and nothing else. There is no inline frame, and no second "expand"
destination.

Nothing routes an artifact away from this path. The transcript used to carry a second
discriminator — a tool result whose `_meta.ui.resourceUri` named an MCP App, which was drawn inline
instead — and **no Rust code in this repository has ever emitted that key**. That is why removing
the MCP apps feature in September 2026 could not change what any Biorouter tool displays: the
branch it fed was unreachable from our own servers. The removal record is filed under
[`docs/history/`](../history/README.md).

This holds on all three surfaces that render a transcript:

| Surface | Component | Panel host |
|---|---|---|
| Live chat | `components/BaseChat.tsx` | `useArtifactPanel` + auto-open + auto-repair |
| Saved session (History, and a schedule's run detail) | `components/sessions/SessionHistoryView.tsx` | `useArtifactPanel`, read-only |
| Shared session (a transcript that left the machine) | `components/sessions/SharedSessionView.tsx` | `useArtifactPanel`, read-only |

The state machine is shared — `components/artifacts/useArtifactPanel.ts` — so the panel's
geometry, its open/close animation and the rung-2 overlay decision cannot differ between them.
The two behaviours that are genuinely chat-only stay with the chat: **auto-open** on a fresh
artifact of a live turn, and **auto-repair**, which feeds a render failure back to the agent.
A read-only transcript passes no `onRenderError`, so `ArtifactViewer` never installs the repair
listener at all, and it opens nothing until the reader clicks a card.

## Why this is enforced by types, not convention

`onOpenArtifact` is a **required** prop on the whole transcript chain — `ProgressiveMessageList`
→ `BioRouterMessage` → `ToolCallWithResponse` → `MCPUIResourceRenderer`. A surface that renders a
transcript without somewhere to put an artifact does not compile.

That is the correction for how the split arose in the first place. `MCPUIResourceRenderer` used to
infer "am I in a live chat?" from whether `onOpenArtifact` happened to be passed, and fell back to
an inline iframe when it was not. Nobody decided that saved transcripts should render figures
differently; two call sites simply omitted a prop. An optional callback made the divergence
invisible, so it survived for as long as nobody compared the two surfaces side by side.

## What an inline frame actually cost

The inline path was not a lighter version of the panel. It was a second renderer:

- **A second CSP.** The inline frame ran the figure through `/mcp-ui-proxy` under one policy while
  the panel ran it under another. When [`ARTIFACT_WRAPPER_CSP` was wrong](artifact-cdn-assets.md),
  the surfaces failed independently and had to be diagnosed independently.
- **A second action channel.** `handleUIAction` gave the guest document a way to request prompts,
  links, notifications and tool calls that the panel has no equivalent for. On a saved transcript
  the prompt bar rendered and its Send button called `append={() => {}}` — a *truthy* no-op, so the
  control looked live and did nothing. The shared-session view omitted `append` entirely and
  returned an honest `UNSUPPORTED_ACTION` for the same guest action. Two read-only surfaces, two
  different answers.
- **A second resize contract.** The inline frame set `autoResizeIframe: { height: true }` and grew
  with its content. The panel sizes the frame itself. Figures still post `ui-size-change`, which is
  now consumed only by an enclosing dashboard report.
- **A fabricated session id.** `SessionHistoryView` passed `sessionId: 'session-preview'`, a string
  belonging to no session. Everything that scopes work by id — the scroll broadcast, Branch —
  addressed a chat that does not exist. It now passes the real `session.id`.

## What was removed

- The inline branch of `MCPUIResourceRenderer` (523 lines → 83), with its proxy-URL effect, its
  five UI-action handlers, the pending-link and pending-prompt bars, and the hover expand button.
- The daemon's `/mcp-ui-proxy` route and its HTML shim, plus its exemption from the secret-key
  middleware in `crates/biorouter-server/src/auth.rs`. It had no non-desktop consumers. **The
  reasoning about opaque `null` origins in `routes/workspace.rs` survives it** — an artifact frame
  is still sandboxed without `allow-same-origin`, so it still presents `Origin: null`.
- The standalone artifact **window** (`open-artifact-window` IPC, `openArtifactInWindow`). Its only
  entry point was the inline card's expand button. "Open in browser" from the panel
  (`openArtifactInBrowser`) is a different path and stays.
- The `/mcp-ui-proxy` allowance in `isAllowedArtifactFrameNavigation`. The policy is now a closed
  set of two literals — `about:srcdoc` and `about:blank` — with nothing configurable about it, which
  is strictly tighter than what it replaced.

## What deliberately stayed

- **`@mcp-ui/client`.** The panel still mounts `UIResourceRenderer` for the `mcpResource` kind — a
  `ui://` resource that is neither HTML nor a URI list — and `ToolCallWithResponse` still uses
  `isUIResource` as a type guard.

## If you are about to add a second surface

Don't add one silently. The failure mode this page exists to prevent is not "someone writes a bad
renderer" — it is a prop quietly going missing at one call site and a whole class of user never
seeing the same thing everyone else sees. If a surface genuinely needs a different presentation,
make the choice explicit and testable at the call site, and write down why here.

## A link is only a link when the panel could open it

A file path the assistant mentions is rendered as an accent-coloured, underlined,
clickable control — but only once the main process confirms the panel could actually
show it. Until then, and whenever it could not, the same text renders as ordinary
prose: no accent, no underline, not focusable, no click handler.

"Could show it" is deliberately stricter than "the file exists". The check runs the
same allowlist and the same symlink refusal that `read-artifact-file` runs, because a
verdict reached down a *different* resolution than the click takes is worse than no
verdict — it paints an accent link onto a file the panel then refuses to open. It also
keeps the reply from being an existence oracle: a path outside the allowlist answers
"no" exactly as a deleted one does, and the reply carries two booleans and nothing else.

Four states, and the two that look alike are the design:

| state | meaning | rendering |
|---|---|---|
| `unchecked` | we know nothing — no bridge, or the check itself failed | link (legacy behaviour) |
| `checking` | asked, no answer yet | plain |
| `present` | confirmed | link |
| `missing` | asked, and it is gone or denied | plain |

`missing` is an answer; `unchecked` is the lack of one. Defaulting to plain and
*upgrading* is what stops a dead path being clickable even for one frame. Routing a
**failed** check to `unchecked` rather than `missing` is what stops one transient IPC
hiccup stripping every real file in the transcript of its link — the same bug pointed
the other way. Surfaces with no bridge at all (the vitest suites, `biorouter serve` in a
browser) keep the pre-existing behaviour of linking everything.

Path resolution is operating-system independent by construction: drive-letter and UNC
paths are recognised on every platform and refused rather than grafted onto a POSIX
working directory, `~` expands from `$HOME` / `%USERPROFILE%` with `os.homedir()` behind
it, and every join goes through `node:path`.

⚠ **jsdom cannot check the part that was reported.** The defect was a *colour*, and a
component test asserts a class without knowing whether that class resolves to anything.
`ui/desktop/.filelink-harness/` mounts the real component with the real stylesheet for
exactly that reason; run it with
`npx vite --config .filelink-harness/vite.config.mts --port 5299`.

## Related documentation

- [How an Auto Visualiser figure's libraries reach the renderer](artifact-cdn-assets.md) — the CSP and CDN-inlining mechanism behind whatever surface displays the figure.
- [Auto Visualiser capability](../extensions/built-in/auto-visualiser.md) — the user-facing guide to the figures themselves.
- [Renderer testing traps](renderer-testing-traps.md) — why a frontend test can pass while the code it covers is broken.
