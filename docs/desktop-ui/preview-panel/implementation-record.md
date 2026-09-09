# What shipped, and what was measured

> **What this is.** The record of executing the [expansion plan](expansion-plan.md): what was
> built, the three places the implementation departed from the plan and why, what was verified
> against real Electron and a real browser, and what is deliberately still open.
> **Status:** Current — all six phases implemented on `design/preview-panel-expansion`.
> **Audience:** whoever reviews or extends this work.

## Phases

| Phase | Contents | State |
|---|---|---|
| 1 | pdf.js offline assets, the one image list, blob URLs | Shipped |
| 2 | Format-specific refusals, fidelity notes | Shipped |
| 3 | `workspace_read_panel`, `workspace_capture_panel` | Shipped |
| 4 | TIFF and HEIC decoding | Shipped |
| 5 | The live browser | Shipped |
| 6 | Region annotation into the chat | Shipped |

## Three departures from the plan

Each is a decision, not a shortcut.

**TIFF decodes in the renderer, not in Rust.** The plan chose Rust because `image` and `tiff` are
already in the tree and add no npm weight. The panel's read path, however, is entirely Electron
main and renderer — routing image preview through the daemon would have given it a failure mode it
does not have today (daemon down, no picture) for a preview that is otherwise self-contained. A
lazily-imported decoder in the renderer is exactly how the four document renderers already work.
Cost: one 105 KB MIT dependency. Only page one of a multi-page stack is shown; a page control is
honest future work, not a claim already made.

**HEIC ships macOS-only, on purpose.** The plan's tiering is implemented as far as Tier 1 (the OS
decoder, which carries neither copyleft nor HEVC patent exposure). Tier 2 — a bundled WASM decoder
for other platforms — is left where the plan put it: an item needing a licence decision rather than
a commit. Elsewhere the panel says plainly that nothing on the system can decode the file.

**The instruction budget moved, 2500 → 2800.** The guard exists to force a decision, and the
decision was that an agent which cannot be told it may look at the user's screen cannot use the
feature at all. Both new entries were cut to two lines *first*.

## Verified against real Electron 39.8.10

Run against the exact shipping binary, extracted from the local cache — not against docs.

| Claim | Result |
|---|---|
| A site that refuses framing loads in a `WebContentsView` | `google.com`, title "Google" |
| It is a top-level browsing context | `window.top === window.self` → `true` |
| The page is real and interactive, not an image | focusable `TEXTAREA` found |
| A **hardened** partition denies permissions | notifications `denied`, geolocation `denied:1` |
| An **unhardened** partition grants by default | notifications **`granted`** — the bypass, reproduced |
| `capturePage` works on the live view | 1000×700, 31 KB PNG |
| Loopback is blocked from the embedded partition | `ERR_BLOCKED_BY_CLIENT` |
| `capturePage` composites a **sandboxed** iframe | lime pixels inside a `sandbox="allow-scripts"` frame |

That fifth row is the reason `embeddedBrowser.ts` installs four handlers on its own session. It is
not a theoretical concern: it was observed.

## Verified in a real browser

Through `ui/desktop/.artifact-harness/`, which mounts the shipping `ArtifactViewer`:

- **`avif`, `bmp`, `ico`, `svg`** all render at 300×200 — every one of them previously refused.
- **`tiff`** decodes to a `blob:` URL at 300×200, showing the authored gradient.
- **`report.doc`** → "Legacy Word document · This is the pre-2007 binary Word format… · Save it as
  .docx"; **`deck.key`** → "Apple Keynote presentation · undocumented bundle format…". Previously
  both produced the same eight words.
- **Annotation** end to end: the control appears only with a session, the overlay dims the preview
  and states its affordances, and a drag produced a capture request of exactly the dragged rect
  (224×144 CSS px) before the mode exited.

The harness now shares the real `IMAGE_MIME_TYPES`, so it can no longer claim support the app does
not have — which is the only reason it is worth having.

## Verified by decoding real files

- TIFF: an LZW-compressed 40×30 file decoded to 4,800 RGBA bytes, first pixel `255,0,0,255` as
  authored.
- HEIC: a real HEIC round-tripped to PNG through the OS decoder.
- Image support: a real file of each of seventeen formats loaded in the shipping Chromium. ⚠ Two
  rows were wrong until `file(1)` checked the fixtures — ImageMagick had silently written PNG when
  asked for `.jxl` and `.apng`.

## Tests

Frontend **336 files / 3,357 tests**, from a baseline of 330 / 3,224. Rust: `biorouter` 2,778 lib
tests, `biorouter-cli` 363, `biorouter-server` 430. `lint:check` clean, including 332 contrast
assertions.

The new guards read each consumer's **real source** rather than importing a value, because the
failures being guarded are omissions — a list that stops being shared, a permission handler that
stops being installed — and no importable value would reveal them. The image-list guard was
mutation-tested: restoring a hand-written MIME map to `main.ts` fails it.

Three real bugs were caught by tests while writing them, and are worth naming because each would
have shipped:

1. Making the workspace handler async deferred the *invocation*, not just the result — reintroducing
   the tick-deferral race the tab planner documents.
2. `max_chars: 0` clamped up to one character instead of falling back to the default.
3. Anchoring the annotation's Space-reposition lazily swallowed the first move, so the marquee
   refused to budge until the user wiggled.

## Still open

- **HEIC off macOS.** Needs the licence decision, not code.
- **Multi-page TIFF** shows page one.
- **Text selection inside an artifact iframe** is impossible from the host (no `allow-same-origin`).
  Region capture covers every artifact kind and shipped first, as the plan intended; a text
  selector needs an injected agent posting through the existing trusted-frame gate.
- **PDF text selection** needs pdf.js's `TextLayer` added to the canvas-only preview.
- ~~The **`frame-src`** hole (`'self' blob: https: http:` in both CSPs).~~ **Closed 2026-09-09**:
  both policies now read `frame-src 'self'`, pinned by `ui/desktop/src/frameSrcCsp.test.ts`, which
  also scans every `<iframe>` the renderer writes so the policy cannot outlive the fact that makes
  it right.

## Related documentation

- [Preview panel expansion plan](expansion-plan.md) — the decision records this executed.
- [The preview panel as it stands today](current-state.md) — the *before* picture; re-measure rather than editing it from memory.
- [Where a generated artifact is displayed](../artifact-display-surfaces.md) — the one-surface rule, which the live browser observes: it is a container inside the panel, not a second surface.
