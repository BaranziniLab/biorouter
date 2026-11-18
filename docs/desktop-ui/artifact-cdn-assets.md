# How an Auto Visualiser figure's libraries reach the renderer

> **What this is.** The two-sided mechanism that lets a figure be stored as a few KB
> and still render offline under a `default-src 'none'` CSP — and the shape a library
> must be emitted in for that to work.
> **Status:** Current.
> **Audience:** Anyone adding a library to the Auto Visualiser, or debugging a figure
> that renders blank or says a library failed to load.

An Auto Visualiser figure is a self-contained HTML document. It is displayed inside a
`srcdoc` iframe carrying `ARTIFACT_BROWSER_CSP`
([`utils/artifactSecurity.ts`](../../ui/desktop/src/utils/artifactSecurity.ts)), whose
first directive is `default-src 'none'`. **A figure can never fetch anything at display
time.** No CDN, no fonts, no XHR.

That would seem to force every library to be inlined, and inlining is indeed the
default — the Rust binary carries D3, Chart.js, Leaflet and Mermaid as
`include_str!` assets. But Mermaid alone is 3.3 MB, and a session that stores a dozen
diagrams was measurably too heavy to reload. So the desktop app sets
`BIOROUTER_AUTOVIS_CDN=1` by default
([`biorouterd.ts`](../../ui/desktop/src/biorouterd.ts)), which makes the tools emit a
pinned CDN `<script src=…>` instead. The stored blob drops from ~3.3 MB to ~16 KB.

The gap between "the stored document references a CDN" and "the displayed document may
not touch the network" is closed in one place: the Electron **main process** fetches
each known CDN URL and splices its source into the document as an inline `<script>`,
*before* the CSP is applied. That is
[`utils/artifactCdnAssets.ts`](../../ui/desktop/src/utils/artifactCdnAssets.ts), called
from `prepareArtifactHtml` in `main.ts`.

## The two invariants

Both are asserted from the Rust side, in
[`crates/biorouter-mcp/tests/autovis_cdn_desktop_contract.rs`](../../crates/biorouter-mcp/tests/autovis_cdn_desktop_contract.rs),
because only the Rust side knows what it actually emits:

1. **Every URL the tools can emit in CDN mode is listed in `ARTIFACT_CDN_ASSETS`.** A
   library missing from that list is simply never inlined. The figure then fails in the
   packaged app *every time* while every Rust test passes, because nothing on the Rust
   side can see the desktop list.
2. **Each is emitted as `<script src="URL" …></script>`** (or `<link href="URL">` for a
   stylesheet). Those are the only two shapes the rewriter's patterns match, and the
   replacement it produces is a **classic** script. A `<script type="module">import x
   from '…/+esm'</script>` therefore fails twice over: the pattern never matches it, and
   module source spliced into a classic script would be a syntax error even if it did.

Invariant 2 is the reason a library must be referenced by its classic/IIFE build.
Mermaid's is `mermaid@11/dist/mermaid.min.js`, an esbuild bundle ending in
`globalThis["mermaid"] = …` — the same global the vendored offline copy sets, which is
what lets both asset modes reach an identical runtime state.

## When a library cannot be downloaded

The main process downloads each library at most once per app session and keeps it
once it has arrived. A download that fails (offline, an HTTP error, a proxy that
resets the connection) or has not finished within 20 seconds is aborted and **not**
kept, so the next figure that needs that library tries again. The fetcher is
`createCachingCdnFetcher` in `artifactCdnAssets.ts`. It used to keep the failed
download, and one open while offline then broke every later figure that needed the
library until the app was restarted.

A library the rewriter could not download does not leave its tag behind. The tag is
replaced with a one-line inline script that records the library's name in
`window.__biorouterUnavailableLibraries`, and the figure's shared runtime
(`templates/_common.js`) reads that record when anything fails. The figure then says
"This figure needs a chart library that could not be downloaded. Check the
connection, then open the figure again." in the figure's own neutral ink, instead of
"d3 is not defined", and posts its render error with the reason
`library-unavailable`. The artifact panel does not turn that error into a repair
turn, because no edit to the figure can bring the library back
(`LIBRARY_UNAVAILABLE_REASON` in `ArtifactViewer.tsx`). Other render errors reach the
repair path as before. `ArtifactViewer.libraryUnavailable.test.tsx` runs the real runtime in
jsdom with the network down to hold all three pieces together.

## Adding a library

1. Add the `include_str!` asset and the `CDN_*` constant in
   [`autovisualiser/common.rs`](../../crates/biorouter-mcp/src/autovisualiser/common.rs),
   and emit it through `script_src` / `script_inline` like the others.
2. Add the same URL to `ARTIFACT_CDN_ASSETS`.
3. Run `cargo test -p biorouter-mcp --test autovis_cdn_desktop_contract`. It fails, by
   name, if either half is missing or the tag shape is wrong.

## Reading a failure

- **"… library failed to load", or a blank figure with `X is not defined`**: check
  whether the prepared document still contains `cdn.jsdelivr.net`. If it does, the
  rewriter did not recognise the reference: either the URL is not in the list, or the
  tag is not in a shape the patterns match. A library it recognised but could not
  download says so in plain words instead (see above), and the main process log has a
  `Could not inline artifact CDN asset` warning naming the URL.
- **A report (`render_dashboard`) is a separate case.** It always inlines, ignoring the
  flag, because its library tags live inside base64 panel blobs the rewriter cannot
  reach. Guarded by `crates/biorouter-mcp/tests/autovis_dashboard_cdn.rs`.
- **Not every display surface is the same, and the second one has its own policy.** The
  in-chat artifact panel hosts the figure's `srcdoc` inside the renderer document. The
  standalone surfaces — the artifact window, "open in browser", and the headless
  renderer's `blob:` tab — wrap it in a second document carrying
  `ARTIFACT_WRAPPER_CSP`, and **a `srcdoc` document inherits its parent's policy list
  and enforces both**. So the guest's effective policy there is the *intersection*, and
  a wrapper grant that is absent is a guest grant that is revoked. That is why
  `ARTIFACT_WRAPPER_CSP` is derived from the figure policy rather than written beside
  it, differing only in `frame-src`; see the comment on it in
  [`artifactSecurity.ts`](../../ui/desktop/src/utils/artifactSecurity.ts) and the drift
  guard in `artifactSecurity.test.ts`.

  ⚠ **Tightening the wrapper does not constrain the figure, it breaks it.** Until
  2026-08-22 the wrapper carried a hand-written
  `default-src 'none'; style-src 'unsafe-inline'; frame-src 'self'`, and *nothing* in
  any artifact ran on those three surfaces — not the chart runtime, and not the
  figure's own error handler, so every figure was an empty card with no message.
  Adding `script-src` alone is **not** the fix: measured in Chromium, that restores
  scripts and leaves `data:` images blocked, so a figure that embeds its own assets
  stays broken in a way that reads as a different bug. The containment that matters is
  the guest's own identical policy plus the sandbox attribute (no `allow-same-origin`,
  no `allow-top-navigation`).

  The terminal has the same rule. `biorouter run`, `biorouter session` and the TUI save
  a figure as a wrapper page around a `srcdoc` frame and print its `file://` address.
  That wrapper's policy is derived from the figure's in
  `crates/biorouter-cli/src/session/output.rs` (`artifact_wrapper_csp`), differing only
  in `frame-src`. It was hand-written as the old desktop one was, and the figure's own
  meta was also written without the opening quote of its `content` attribute, so every
  saved figure opened blank. `the_saved_figure_csp_attribute_is_quoted` and
  `the_wrapper_grants_everything_the_saved_figure_is_granted` in that file pin both.

- **A map shows no tiles in either surface, by design.** `img-src data: blob:` names no
  remote scheme, so Leaflet's tiles never load in an artifact; markers and vector
  layers do. That is the offline guarantee working, not a rendering failure.

## Related documentation

- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — how to get the real app in front of you to check a figure.
- [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) — driving it over CDP once it is running.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_AUTOVIS_CDN` and `BIOROUTER_AUTOVIS_DEBUG`.
