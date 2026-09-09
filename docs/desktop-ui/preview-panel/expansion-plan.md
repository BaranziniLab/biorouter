# Preview panel expansion plan

> **What this is.** The plan to widen the artifact side panel along five axes — image formats,
> Office documents, live websites, an annotation channel into the chat, and agent access to what
> the panel is showing. Decision records, five workstreams, the security analysis, and how each
> piece is tested.
> **Status:** Proposed — awaiting approval. Nothing here has shipped. Branch
> `design/preview-panel-expansion`, worktree `biorouter-preview-panel-wt`.
> **Audience:** the reviewer approving this, and whoever executes it afterwards.

Read the [current-state survey](current-state.md) first, or at least §1 below. The panel is
substantially more capable than the brief assumed, and the plan is smaller and differently shaped
as a result.

---

## 1. What the measurement changed

Five things were asked for. Measured against the code, they are not five equal pieces of work.

| # | The ask | What is actually true |
|---|---|---|
| 1 | More image formats | Six work today. **Three more already decode in Chromium and are excluded for no reason.** Two more (HEIC, TIFF) genuinely need a decoder — and the Rust crates for TIFF are already in the dependency tree. |
| 2 | View-only Word / Excel / PowerPoint | **Already shipped.** All four of PDF, DOCX, XLSX and PPTX render today via bundled offline renderers. The real work is the *gap* around them: legacy `.doc`/`.xls`/`.ppt`, OpenDocument, and RTF. |
| 3 | Browse a real website | **Genuinely absent, and deliberately so.** Frame navigation is a closed set of two literals with a test asserting an `https` URL is refused. This is the one item that changes the security model. |
| 4 | Annotation → chat | Genuinely absent. No selection, range, crop or overlay code exists. But every delivery mechanism it needs already exists. |
| 5 | Agent access to the panel | Genuinely absent. But the channel it needs already exists and already does blocking round-trips. |

Two of the five are mostly done. Two are new features on existing plumbing. **One — live
websites — is a real architectural decision**, and most of this document is about it.

### Three facts I verified rather than assumed

These are load-bearing, so they were measured rather than researched.

**Half of real sites cannot be framed, including the example in the brief.** Measured with
`curl` on 2026-08-23:

| Refuses framing | Allows framing |
|---|---|
| `google.com` (`SAMEORIGIN`) — the brief's own example | `ucsf.edu` |
| `pubmed.ncbi.nlm.nih.gov` (`DENY`) | `uniprot.org` |
| `ncbi.nlm.nih.gov/gene` (`SAMEORIGIN`) | `useast.ensembl.org` |
| `clinicaltrials.gov` (`SAMEORIGIN`) | `genome.jp/kegg` |
| `nature.com` (`DENY` + `frame-ancestors 'none'`) | `gnomad.broadinstitute.org` |

The refusers are exactly the destinations this audience uses most. An `<iframe>` implementation
would appear to work — `ucsf.edu` renders fine — and fail on PubMed. That is worse than not
shipping it.

**`capturePage(rect)` composites sandboxed iframes.** Verified against the exact shipping
Electron (39.8.10 darwin-arm64, extracted from the local cache): a host page painted white with a
`sandbox="allow-scripts"` `srcdoc` iframe painting lime returned `255,255,255` at a host pixel and
`0,255,0` at an iframe pixel; a rect capture returned exactly the requested box. This matters
because the artifact preview is sandboxed *without* `allow-same-origin`, so `html2canvas` and
every DOM-walking screenshot library structurally cannot see into it. **One main-process
primitive serves both the annotation crop and the agent screenshot, and no screenshot library is
needed at all.**

**`avif`, `bmp` and `ico` already decode.** Verified by loading a real file of each format.
Chromium 142 (which Electron 39 ships) has exactly eight raster decoders; `heic` and `tiff` are
not among them and `jxl` is flag-gated until Chrome 145. ⚠️ My own probe initially reported a
JPEG XL pass — ImageMagick had silently written a PNG when asked for `.jxl`, and `file(1)`
caught it. Two of seventeen rows were wrong before that check.

---

## 2. Decision records

### PP-01 — The panel stays the single display surface

Nothing here adds a second renderer, a second CSP, or a second action channel.
[`artifact-display-surfaces.md`](../artifact-display-surfaces.md) records what the last one cost:
two policies that failed independently, a prompt bar whose Send was a *truthy* no-op, two
read-only surfaces giving different answers to the same guest action, and a fabricated session
id. Live web content gets its own *container* (PP-02) but it is still presented in, and owned by,
the one panel.

### PP-02 — Live web content never travels through the artifact iframe

It gets a `WebContentsView` in its own session partition, with no preload.

Three independent reasons, any one sufficient:

1. **It would not work.** Half of real sites refuse framing (§1), including the brief's example.
   Microsoft reached the identical conclusion from the identical evidence: VS Code's 1.109 notes
   say its Simple Browser "relied on iframes", so "website authentication wasn't possible, and
   common sites like Google, GitHub, and Stack Overflow couldn't be opened" — and that is why
   they replaced it with a `WebContentsView`. Across a survey of Electron apps the split is clean:
   those embedding a *fixed set of known* web apps use `<webview>`; those that must survive
   *arbitrary* sites use `WebContentsView`. We are the second kind.
2. **It would reverse a ruling.** `isAllowedArtifactFrameNavigation` is `about:srcdoc` or
   `about:blank` and nothing else, and `permissionPolicy.test.ts:48` asserts
   `https://example.test/exfiltrate` is `false`. That guard exists to stop agent-authored HTML
   phoning home. It stays exactly as it is.
3. **The policies are opposites.** The artifact CSP is `default-src 'none'; connect-src 'none'`.
   A live website needs the network by definition. Loosening the artifact policy to admit
   websites would loosen it for every figure too.

**The cost is real and must be accepted knowingly.** A native view paints above the DOM, so it
does not respect stacking, scrolling, border radius or modals. VS Code itemizes the bill in its
own source — placeholder-screenshot masking during show/hide swaps, an overlay-pause system that
hardcodes a list of overlay class names with hit-test exclusions, a focus dance between the
workbench DOM and the view, native key-event forwarding, and a pixel-snap layout contribution —
and notes that an in-DOM iframe "would need none of the above." We are choosing the expensive
renderer because the cheap one does not work, not because it is nicer.

Concretely for this app: the view must be hidden whenever a modal, the tab-overflow dropdown, a
toast or the artifact resize shield is up, and its bounds must track the panel through resize and
the overlay/side rung switch in `useArtifactPanel`.

### PP-03 — Browsing is a user capability; the agent's access to it is separate, visible and revocable

This is the single most valuable pattern in the competitive research, and both mature
implementations landed on it independently: Codex has `in_app_browser` (a pane a person uses) and
`browser_use` (an agent capability) as separate flags with separate policy; VS Code's tabs are
private by default and the agent cannot read one until the user picks **Share with Agent**.

For us that means:

- **The agent cannot open a live page.** Today an MCP resource *link* with an `http(s)` URI
  becomes an artifact through Path A with no transcript card at all, and can auto-open. If
  in-panel rendering were simply wired onto the existing `externalUrl` kind, any MCP server could
  cause an arbitrary site to load and execute in the user's app. So `externalUrl` **keeps
  rendering its card**, and gains an explicit "Open here" control. One deliberate click is the
  whole boundary, and it makes agent-initiated navigation structurally impossible.
- **The agent cannot read a live page** until the user shares that tab. Sharing is per-tab,
  visible on the tab, and revocable.

### PP-04 — Per-origin policy, with turn-scoped approval

Modelled on Codex's `requirements.toml`: separate axes (`access`, `downloads`, `uploads`) rather
than one on/off, and `access_approval_lifetime = "turn" | "thread"` so an approval can expire.

And stated with the honesty Cursor's own docs use: an origin allowlist is **best-effort**, not a
boundary — link navigation, redirects and `window.location` from an allowed origin all defeat it.
That is the same "privacy is safety, not security" line this repo already takes. Do not describe
it as a boundary in the UI.

> 🚩 **The highest-priority security item in this plan, and it was found by measurement, not by
> reading.** A permission handler installed on `session.defaultSession` **does not cover a
> partitioned session, and an unhandled session grants by default.** Two views were run side by
> side: the one on its own partition with no handler returned `granted` for
> `Notification.requestPermission()` and allowed geolocation, and the app's handler was **never
> called**. `main.ts:4767` installs BioRouter's handler on `defaultSession` — so a new partition
> would silently auto-grant notifications, geolocation, clipboard-read, media and display-capture.
> The embedded session needs its own `setPermissionRequestHandler` (async prompts) **and**
> `setPermissionCheckHandler` (synchronous checks), plus `setDevicePermissionHandler` and
> `setDisplayMediaRequestHandler`. Deny-all first, allowlist after.

### PP-04b — A login flow must leave the app

RFC 8252 §8.12: native apps **MUST NOT** use embedded user-agents for authorization, because the
host can read the credential and log every keystroke, and the user has no browser security
indicators to check. Google has refused OAuth from embedded webviews since 2016. For a UCSF
product this is a compliance point rather than a preference: any SSO or sign-in navigation hands
off to the system browser.

This also disposes of a tempting shortcut. Stripping `X-Frame-Options` to make iframes work does
not merely re-enable clickjacking against the embedded site — **it does not even work**, because a
cross-site frame is a third-party context where `SameSite=Lax` cookies are not sent. The user gets
a logged-out, half-broken render.

### PP-05 — The agent reads the panel over the existing `WorkspaceBridge`

Not a new channel. `/ui/workspace` is the renderer's only WebSocket to the daemon, it is
session-routable (`bridge_for_session`), and it already does blocking round-trips
(`emit_and_wait`, 10 s). `workspace_list` already reads GUI state this way.

Copy `WorkspaceBridge`, **not** `UiBridge`: the former fuses the generation counter with the
sender under one lock; the latter splits them and has a documented, 200/200-reproducible severing
race.

Two changes are needed on the renderer side and none on the daemon: the inbound command handler
is **synchronous** and its reply is `{ok, detail: string}` only. A capture is inherently async and
needs a wider envelope. The daemon already resolves with the whole frame, so extra reply fields
survive today.

### PP-06 — Two observation channels, with an explicit division of labour

Text for acting, pixels for judging. VS Code encodes this in the tool descriptions themselves —
`read_page` says *"This is better than screenshot"*, and `screenshot_page` says *"You can't
perform actions based on the screenshot"*. Replit reached the same conclusion independently and
priced it: a DOM+ARIA snapshot per step costs about $0.20 per session against about $0.50 for a
*single* form under vision.

So the default is text. The screenshot is for "does this look right", which is exactly what the
brief asks for and exactly what text cannot answer.

### PP-07 — Pixels go out of band; the echo frame carries only a descriptor

`MAX_INBOUND_FRAME_BYTES` is 128 KiB and its own comment explains why: the stored echo is handed
to the model verbatim, making it both a memory sink and an injection vector. A base64 PNG does
not belong there. The capture is written to the temp dir and the frame carries the path.

The panel descriptor — what kind of artifact, its title, its path or origin — is small and *is*
pushed in the existing echo, so "what is on screen?" costs no round-trip at all.

### PP-08 — `capturePage(rect)` is the only capture primitive

Verified in §1. No `html2canvas`, no `html-to-image`, no `modern-screenshot`. Those libraries
re-render the DOM, often get it wrong, and structurally cannot enter the sandboxed frame that
holds most of what is worth capturing. `html2canvas` specifically has been unmaintained since
July 2024, still carries a README disclaiming production use, and **does not support `oklch()`** —
which is how this app's theme tokens are authored.

Measured semantics to design against: `capturePage` grabs the compositor's current surface rather
than forcing a render. It works on an **occluded** view and on one moved off-screen, returns the
**last painted frame** for a hidden window, and returns an **empty image** if the view is hidden
*and then* navigated — `stayHidden` does not help, and Windows is reportedly worse. So always
check `NativeImage.isEmpty()`: the failure mode is a silently empty buffer, not a rejection. None
of this bites the annotation path, where the user is by definition looking at the thing.

### PP-09 — An annotation crop reuses the existing image attachment path, unchanged

`canvas → saveDataUrlToTemp → { path, kind: 'image' } → createUserMessage → { type: 'image',
data, mimeType }`. **No schema change on either side, and no Rust change.** The existing caps
apply and are the right ones: 4 MiB data-URL string, 3 MiB decoded, 5 images per message, and a
MIME allowlist of `png|jpeg|jpg|gif|webp`. Crops are emitted as PNG, which is on that list.

Three corrections to how that payload is built, each cheap and each with a measurable effect:

- **Send the crop, not the whole image plus coordinates.** Anthropic's own guidance says so for
  fine targets, and the arithmetic is stark: a 640×420 crop costs roughly 308 visual tokens, while
  a full 3840×2160 screenshot costs 4,784 *and* gets downscaled — moving the very coordinates you
  were pointing with.
- **Never hard-code a coordinate convention.** Claude wants absolute pixels and explicitly warns
  against normalized 0–1000; Gemini emits normalized 0–1000. A crop is the one payload that is
  universal across providers.
- **Put the image block before the text block.** `createUserMessage` currently appends images
  after the text; Anthropic states Claude "works best when images come before text." One-line
  reorder. ⚠️ Also note the widely-repeated `(w × h) / 750` token formula is **stale** — cost is
  patch-based, `⌈w/28⌉ × ⌈h/28⌉`. Resize crops to the model's tier client-side (1,568 px long edge
  standard; 2,576 px on the high-resolution tier) so no server-side resize happens behind you.

### PP-10 — A text annotation reuses the `<biorouter-ref>` chip grammar

Issue #65 already solved "attach a non-prose payload to a message and draw it as a chip", and
solved it by putting the payload **inside the message string** — because draft save/restore, the
`?prompt=` deep link, the queue, steering and local history each key off that one string, "and
each one forgotten is a reference the user attached and the agent never sees." A new `RefKind`
inherits all of that. A parallel mechanism would have to re-earn it.

Selections are clamped like `RENDER_ERROR_TEXT_LIMIT` (2,000 chars). There is no cap on composer
text today, and an unbounded selection from a large document would otherwise go straight into the
prompt.

### PP-11 — Annotation works on every preview kind, not just the browser

Codex shipped "Comment with Codex" in its browser but not in its Markdown/HTML preview pane, and
the open issue against it names our exact case: users read a generated report and cannot comment
on it. For a research tool the report *is* the artifact. Annotation is defined once against the
panel, and each preview kind supplies a locator.

### PP-12 — Freeze the frame before annotating

Cursor: *"the annotation sits over a frozen frame of the viewport, so the agent sees the exact
page state you were responding to."* On an animated figure or a live page, an annotation over a
moving target describes something the agent will never see. The freeze is a `capturePage` taken
at the moment the mode is entered; the overlay is drawn on that image.

### PP-13 — The picker overlay lives in a shadow root at maximum z-index

Replit does this so the previewed app's CSS can never clobber the picker. This codebase has
already been bitten by the general form of that problem — Prism's unprefixed `token` classes
colliding with Tailwind's `.table` utility — so the precedent is local as well as external.

### PP-14 — Formats Chromium already decodes are a list change, not a feature

`avif`, `bmp`, `ico` (plus `apng`, `cur`, `jfif`, `pjpeg` for completeness). Zero dependencies,
zero bytes. Four lists must be edited together or the format half-works; two adjacent lists
(`ItemIcon`, `MentionPopover`) already *offer* `bmp`/`tiff`/`ico` the panel cannot render, so this
also closes a pre-existing inconsistency.

`jxl` is **not** added: Chromium 142 has no JPEG XL, and it is flag-gated even in 145.

### PP-15 — TIFF decodes in Rust with crates already in the tree

`image` and `tiff` are already dependencies, both MIT/Apache-2.0. The `tiff` crate supports
BigTIFF and real multi-page (`seek_to_image`/`next_image`/`more_images`) — which is what
microscopy needs. ⚠️ Use the `tiff` crate **directly**: `image`'s `TiffDecoder` exposes only the
first IFD.

Gaps to detect and report honestly rather than paper over: no CCITT G3, no JPEG 2000, no LERC,
and **no YCbCr or palette photometric interpretation**. That last one is not academic —
JPEG-compressed TIFF is usually YCbCr, which is exactly what scanner and whole-slide output
produces. An unsupported variant must say so, not render blank. Pair with `kamadak-exif`
(BSD-2-Clause) for orientation; wrong-rotation previews are the commonest cosmetic bug in this
class of feature.

### PP-15b — HEIC is tiered and OS-first, because the patent exposure is not a licensing choice

This is the decision that changed most under research, and the reason is worth stating plainly:
**HEVC patents attach to the technique, not the implementation.** A pure-Rust MIT decoder would
solve the copyleft question and do nothing whatever about patents. Access Advance's own FAQ
answers "we make software that is free for users to download" with *"In general, HEVC software
downloaded by users requires a license."*

The market's behaviour corroborates it: `sharp` excludes HEIC from its prebuilts and calls it
"patent-encumbered"; `wasm-vips` compiles `libheif` with `-DWITH_LIBDE265=OFF`; Debian splits the
HEVC decoder into a plugin that is not installed by default; no browser has ever shipped it.

**The one route that plausibly sidesteps patents is decoding through a decoder the OS vendor has
already licensed.** So:

| Tier | Platform | Mechanism | LGPL | Patent | Build cost |
|---|---|---|---|---|---|
| 1 | macOS | shell out to `sips` (ImageIO, 10.13+) | none | **covered by Apple** | none |
| 2 | elsewhere | a lazily-loaded WASM decoder in the renderer | trivial | same as anyone's | none |
| 3 | Windows | WIC if the user has the extensions; else Tier 2 | none | user's own | none |

Since HEIC files are overwhelmingly iPhone and Mac artefacts, Tier 1 covers most real traffic for
nothing. Verify once at startup with `sips --formats` and cache it — the published man page
predates High Sierra and does not list HEIC, so trust the runtime probe over the documentation.

**WASM in the renderer beats a native Rust `libheif` on every axis except none.** LGPL compliance
is trivial because a discrete `.wasm` is replaceable by construction, where a bundled dylib is
not "already present on the user's system" and its relink right conflicts with our own code
signature. Security matters more than it looks: `libheif` published nine advisories in five weeks,
three rated High, all heap errors in parsing **untrusted user files** — in WASM that corruption is
contained by the sandbox; linked into the backend it runs beside the user's research data. And it
avoids a `libheif` ≥1.17 floor that Debian bookworm's 1.15.1 does not meet, plus a mingw
cross-compile with no vcpkg fallback.

⚠️ **Adding any native binary is a release-pipeline change, not a dependency change** — nested
code must be signed with the UCSF Developer ID and `@rpath`-fixed before the outer app, and
`scripts/release.sh` does not do that today. That cost is avoided entirely by staying in WASM.

Traps verified and to be avoided: `heic2any` declares MIT on npm while shipping 1.36 MB of
compiled LGPL `libheif`; the `heif-rs` crate wears an Apache-2.0 label while statically linking
**GPL-2.0 x265**; and `sharp` rebuilds bake absolute Homebrew paths into the `.node` that break
after packaging. Prefer `heic-to` over `libheif-js` — the latter is pinned about a year behind
upstream on CVE fixes.

**Two items for escalation rather than code:** whether UCSF is comfortable shipping an LGPL
component with a documented relink offer, and the HEVC patent question itself. If either answer is
no, the tiered design degrades gracefully — macOS keeps full support via Tier 1 and other
platforms show an honest "convert to JPEG" card.

### PP-16 — Large images move from data URLs to blob URLs

The panel builds `data:image/…;base64,…` in the main process, so an image costs about 4/3 of its
size as a JS string, twice. Chromium struggles with data URLs past a couple of MB for IPC
reasons; blob URLs handle 100 MB. Also enforce a 32,767 px downsample ceiling — past it Chromium
renders **blank**, which a converted whole-slide TIFF will hit.

### PP-17 — Privacy: one genuinely new channel, one that only looks new

- **The agent screenshotting the panel is not a new exposure class.** A public-capable session
  with `developer__shell` can already read the same file. That is §9.5's general filesystem
  read-deny, explicitly **descoped for v1** (DR-14). The screenshot tool sits behind Gate C like
  every other tool and should be documented as the same known gap, not dressed up as novel.
- **A live browser *is* new: it is a network egress surface inside the app.** The asymmetry this
  repo already uses applies exactly — *a user may proceed past a warning; the agent never does the
  same thing automatically.* A user browsing in a private-tier session gets a disclosure and
  proceeds. **The agent navigating that session's browser to an arbitrary URL is an exfiltration
  channel and is refused.** PP-03 already makes agent-initiated navigation impossible; this is the
  reason it must stay impossible rather than becoming a permission.

### PP-18 — Never `postMessage(payload, '*')`, and always check `event.source`

bolt.diy posts element data with `targetOrigin: '*'` in both directions and never validates
origin on receipt. Replit checks inbound but still broadcasts source paths and a PNG of the UI
outbound to whatever frames the app. Both are five-line fixes they did not make.

The existing gate is already correct and must be reused: `ArtifactViewer` accepts a `message`
only when `event.source === trustedFrameRef.current?.contentWindow`, because an untrusted frame
"would otherwise be able to inject instructions into a session that holds shell and file tools."

---

## 3. The five workstreams

### A — Image formats

| Step | Change | Cost |
|---|---|---|
| A1 | Add `avif`, `bmp`, `ico`, `apng`, `cur`, `jfif`, `pjpeg` to all four lists that must agree | 4 files, no deps |
| A2 | Reconcile `ItemIcon` and `MentionPopover` with the panel's list | pre-existing bug |
| A3 | Blob URLs for images over a threshold; 32,767 px ceiling | main process |
| A4 | TIFF → PNG in Rust via the `tiff` crate, multi-page aware | crates already present |
| A5 | HEIC via `libheif-js`, `.wasm` as a separate file, lazy-loaded | ~6.4 MB, LGPL notice |
| A6 | EXIF orientation via `kamadak-exif` for anything we rasterize ourselves | BSD-2 |

Deferred with reasons, not silently: DICOM (`dwv` is **GPL-3.0** and disqualifying; Cornerstone3D
is the MIT alternative if demand appears), NIfTI, OME-TIFF/`viv`, RAW (LibRaw is LGPL/CDDL and
`libraw-wasm`'s ISC label covers only the wrapper), EPS/PostScript (needs **AGPL** Ghostscript —
ImageMagick's own maintainer refused it on exactly these grounds), FITS.

### B — Office documents: gap-fill only

PDF, DOCX, XLSX and PPTX already render, and independent research validated all four library
choices as the best permissively-licensed options available. This workstream is the edges — and
one real bug.

> **B0 — the PDF renderer is missing its offline assets, and this is a live defect.**
> Verified: `cMapUrl`, `standardFontDataUrl`, `wasmUrl` and `iccUrl` appear **nowhere** in
> `ui/desktop/src/`, and `getDocument` is called with `{ data }` alone. In pdf.js 6.x the JPEG 2000
> and JBIG2 decoders are WASM modules loaded from `wasmUrl` — **so scanned and medical PDFs fail
> to decode images today**, which for a biomedical corpus is close to the worst possible gap.
> Missing `cMapUrl` also breaks CJK glyphs and `standardFontDataUrl` degrades non-embedded
> Standard-14 fonts. Fixing it costs about 2.3 MB of bundled assets (skip `quickjs-eval.wasm` —
> it is XFA/AcroForm scripting a view-only panel does not want) and requires
> `'wasm-unsafe-eval'` in the renderer `script-src`. Note **not** `'unsafe-eval'`: pdf.js dropped
> that requirement in 5.7.284, so every blog post saying otherwise is stale. All four URL options
> throw unless they end in a trailing slash.

| Step | Change |
|---|---|
| B0 | Ship pdf.js's offline assets; add `'wasm-unsafe-eval'`; move `workerSrc` → `workerPort`; drop the unnecessary `legacy/` build |
| B1 | Audit the four renderers against a real corpus — a manuscript with a TOC and tracked changes, a workbook with conditional formatting, a deck with SmartArt, and a scanned PDF with JBIG2 |
| B2 | Decide legacy `.doc`/`.xls`/`.ppt`. `calamine` reads `.xls`; `.doc` and `.ppt` have no viable path — decline explicitly |
| B3 | Decide OpenDocument and `.rtf` rather than letting them drift. `calamine` covers `.ods` |
| B4 | An unsupported document says *which* format and *why*, not "can't be previewed here" |
| B5 | State the fidelity ceiling in the UI — no pptx animations, 3D, equations or notes; no docx TOC or exact pagination. Every non-commercial renderer shares these limits, and saying so beats a user concluding their file is corrupt |

`workerPort` in B0 is worth its own sentence: under `file://` the origin serializes to `"null"`,
so pdf.js routes `workerSrc` through a `blob:` wrapper that the renderer's `worker-src 'self'`
forbids. `workerPort` never takes that path. One line.

**The Rust converters are not the answer here** and reusing them would be a downgrade: `docx.rs`
matches only paragraphs so tables and images silently vanish, and `pptx.rs` writes the literal
string `*[image omitted]*`. They exist to feed an LLM cheaply and deliberately discard appearance.
There *is* a good Rust role, though — `office_oxide` (MIT/Apache-2.0) for a `to_markdown()` that
feeds document text into the agent's context, complementary to the visual panel rather than a
replacement for it.

**Ruled out on licence or data-handling**, and worth recording because an automated scan catches
none of the interesting ones: `@cyntler/react-doc-viewer` **uploads the document to
`view.officeapps.live.com`**; `superdoc` and every ONLYOFFICE WASM port are **AGPL-3.0** (which
would force BioRouter's own Apache-2.0 code open); `pptx-preview` declares **ISC on npm while
being proprietary**; `gc-excelviewer` wears MIT over a commercial Wijmo engine; and npm's `xlsx`
is frozen at 0.18.5 with **two HIGH CVEs whose advisories record no patched version** — one of
them prototype pollution triggered by reading a crafted file, which is exactly this threat model.
LibreOffice (MPL-2.0) is fine to *detect and use* if installed, and must not be bundled: 300–800 MB
per platform against ~120 MB binaries today, plus re-signing hundreds of nested dylibs.

**The Rust converters are not the answer here** and reusing them would be a downgrade: `docx.rs`
matches only paragraphs so tables and images silently vanish, and `pptx.rs` writes the literal
string `*[image omitted]*`. They exist to feed an LLM cheaply and deliberately discard appearance.

### C — Live websites

| Step | Change |
|---|---|
| C1 | A `WebContentsView` host bound to the panel's geometry, its own partition, no preload |
| C2 | Hide/show against modals, dropdowns, toasts and the resize shield (the itemized bill in PP-02) |
| C3 | Minimal chrome: back, forward, reload, URL, loading and failure states |
| C4 | Deny every permission by default; `setWindowOpenHandler`; block `file:` and `127.0.0.1` from that partition |
| C5 | Per-origin policy with turn-scoped approval (PP-04) |
| C6 | The "Open here" control on the existing `externalUrl` card — the only way a live page loads |
| C7 | ~~Tighten `frame-src` in both CSP sources once nothing needs remote framing~~ — **done 2026-09-09**, ahead of the rest of C |

C7 was worth calling out, and it is now closed: both policies read `frame-src 'self'`, and
`ui/desktop/src/frameSrcCsp.test.ts` pins them. It landed before C1–C6 rather than after, because
the live browser is a `WebContentsView` and never needed a frame source — so the latent hole could
be closed without waiting on the feature that was expected to justify it.

### D — Annotation

| Step | Change |
|---|---|
| D1 | An annotate mode on the panel: freeze the frame (PP-12), overlay in a shadow root (PP-13) |
| D2 | Region select → `capturePage(rect)` → temp file → existing image attachment (PP-09) |
| D3 | Text select → clamped payload → a new `<biorouter-ref>` kind → composer chip (PP-10) |
| D4 | Per-preview-kind locators: file path + line for text, page + rect for PDF, cell for a sheet, URL + selector for a live page |
| D5 | Batch: stay in the mode, accumulate, send together |

**The payload format follows VS Code's**, which is the best template found: labelled markdown
sections — element, URL, ancestor path, outer HTML, dimensions, computed CSS — not a JSON blob
concatenated onto the message, which is what bolt.diy does with the same information and far worse
ergonomics.

One UX rule from three separate Codex bug reports: **the annotation composer's Enter key must
agree with the chat composer's.** Theirs disagree and it is their most-complained-about detail.

**This workstream needs no new dependencies.** Every piece is a platform API, an existing
dependency, or first-party code:

- **Painting a highlight** uses the CSS Custom Highlight API (`new Highlight(range)` +
  `CSS.highlights.set(...)`), which Chromium 142 has, along with `highlightsFromPoint` for click
  targets. It mutates no DOM, causes no reflow, survives React reconciliation, and handles overlap
  natively — where span-wrapping fights the virtual DOM and gets wiped on the next render. Use a
  **live `Range`**, not `StaticRange`, for exactly that reason. ⚠️ `::highlight()` accepts only
  colour, `text-decoration`, `text-shadow` and text-stroke properties — **no border radius and no
  background image** — so a rounded highlight pill must be an overlay driven from
  `Range.getClientRects()`, not a `::highlight()` rule.
- **Anchoring** is a hand-rolled W3C-shaped selector (`exact` + `prefix` + `suffix`, with a
  character offset as a *hint*), about 150 lines. The library field does not justify a dependency:
  Apache Annotator was retired from the incubator in 2025 and archived, and `rangy`, `mark.js` and
  the `dom-anchor-*` family are all years stale. The same `prefix`/`suffix` doubles as the
  surrounding context sent to the model, so anchoring and disambiguation cost the same bytes.
- **The drag-rect** is hand-rolled too. The affordances are where the perceived quality lives, and
  they come straight from `Cmd+Shift+4`: a live `W × H` badge tracking the cursor, `Shift` to
  constrain, `Option` to size from centre, `Space` to reposition mid-drag, `Esc` to cancel, and
  handles on the committed marquee so it is adjustable *before* commit rather than fire-and-forget.
- **The ancestor problem** — a click always lands on the innermost node. bolt ships the answer as
  "pick from layers": a small control that walks up the ancestor chain so the user can say they
  meant the card, not the label inside it.

⚠️ **Text selection inside an artifact iframe is impossible from the host** — no
`allow-same-origin` means no `getSelection()` across that boundary. Either inject a selection
agent into the artifact HTML that posts up through the existing trusted-frame channel, or treat
HTML artifacts as image-only for annotation. **Ship image-only first**: one mechanism covers every
artifact kind, and `capturePage` crosses the boundary that nothing else can. PDF *region* crop
works today; PDF *text* selection needs pdf.js's `TextLayer` added to the canvas-only preview, and
is its own piece of work.

### E — Agent access to the panel

| Step | Change |
|---|---|
| E1 | Extend `buildEchoFrame` with a small panel descriptor — free, no round-trip, already debounced |
| E2 | Widen the renderer's command handler to async and its reply beyond `{ok, detail}` (PP-05) |
| E3 | `panel_read` — text/structure of what is displayed. The default. |
| E4 | `panel_capture` — `capturePage(rect)` → temp file → `Content::image(base64, "image/png")` |
| E5 | Tool descriptions that encode the division of labour (PP-06), in the tools themselves |

The image return shape already exists and is copied verbatim from `screen_capture`:
`Content::text(note).with_audience(vec![Role::Assistant])` alongside
`Content::image(data, "image/png").with_priority(0.0)`. Note `with_audience([Assistant])` means the
note reaches the model but not the transcript.

If these hang off the `workspace` platform extension, four edits are needed and three are
test-guarded: `get_tools()`, the match arm, `WORKSPACE_TOOL_NAMES` in `agent.rs`, and
`workspace_parity.rs`. `PLATFORM_EXTENSIONS.len() == 6` is asserted, so adding a *new* extension
fails that test by design.

---

## 4. Sequencing

Ordered so that each phase ships something usable and the risky phase comes after the cheap wins.

| Phase | Contents | Why here |
|---|---|---|
| **1** | **B0**, A1, A2, A3 | B0 is a live defect, not an enhancement — scanned and medical PDFs fail today. The rest is hours and no dependencies. |
| **2** | B1–B5 | Audit before extending; may prove B2/B3 unnecessary |
| **3** | E1–E5 | Independent of C and D; the channel exists |
| **4** | A4, A5, A6 | New decoders, contained blast radius |
| **5** | C1–C7 | The architectural change; needs its own review |
| **6** | D1–D5 | Wants C in place so the browser is one of its surfaces |

Phase 5 is the one to stop and re-review before merging. Phases 1–4 are additive to a surface that
already works — and phase 1 now leads with a bug fix rather than a feature, which is the right
order regardless of what else is approved.

---

## 5. Testing

The existing suite is **330 files / 3,224 tests, green** on this branch at `edfd6325`; the
artifact subset is 7 files / 197 tests. That is the baseline every phase is measured against.

**Three coverage holes to close first**, because they are the surfaces being changed: no test
renders `ArtifactViewer`'s `image` branch, its `binary` branch, or its `document` branch, and
there is no `useArtifactPanel.test.ts` at all.

**What jsdom cannot see, and what covers it instead.** jsdom has no layout engine, never runs
Tailwind, does not evaluate `@container`, and does not implement `canvas.getContext('2d')`. So:

| Concern | Instrument |
|---|---|
| Render branches, list agreement, payload shapes, policy predicates | vitest |
| Panel geometry, the native-view overlay bill (PP-02), picker overlay | `ui/desktop/.artifact-harness/` in a real browser |
| Format decode support | a real file per format loaded in the shipping Electron — **not** a synthesized one; my own probe was wrong twice until `file(1)` checked it |
| `capturePage` behaviour | the shipping Electron binary, as in §1 |
| Live-site framing and policy | measured against real origins, re-measured periodically |

**Mutation-test every new guard.** The repo has a documented history of tests that pass against
the bug they were written for. A guard that does not fail when its fix is reverted is not a guard.

---

## 6. Risks and open questions

| Risk | Assessment |
|---|---|
| The permission-handler bypass (PP-04) ships unnoticed | **Would be a real vulnerability, and it is invisible to every existing test.** The measured default is *grant*. Must be a merge gate on C, verified in a running app rather than in jsdom. |
| The native view's overlay bill (PP-02) is larger than estimated | Highest-confidence *cost* risk. VS Code itemized it from experience, and the resize seam is a ten-year-old Electron issue with no fix coming. Mitigation: build C1+C2 first and evaluate before C3–C7. |
| An origin allowlist is mistaken for a boundary | Documented honestly in the UI, per PP-04. |
| HEVC patent exposure | **Not removable by any library choice** (PP-15b). Escalate; Tier 1 avoids it on macOS. |
| HEIC's LGPL obligation | Manageable — separate `.wasm`, never a bundled dylib. Worth a second pair of eyes before merge. |
| A HEIC decoder adds ~6.4 MB | Lazy-loaded on first `.heic` only. |
| Widening the panel's reach outruns its guards | `.biorouterignore` is **not** consulted on this path and fully-automatic mode admits any non-sensitive path. Any widening should state whether that is still acceptable. |

**Questions for the reviewer:**

1. **Should live browsing be gated behind a setting, default off?** PP-03 makes agent-initiated
   navigation impossible, but the feature still puts a full browser in a clinical-data
   application. I lean toward shipping it on, with the private-tier disclosure from PP-17.
2. **B2/B3 — how far into legacy and OpenDocument formats?** `.xls` and `.ods` are nearly free via
   `calamine`. `.doc` and `.ppt` have no viable path and I would decline them explicitly.
3. **Is DICOM in scope?** Deferred here, and the obvious library is GPL-3.0 and disqualifying. A
   biomedical audience may want it enough to justify Cornerstone3D.
4. **Phase 5 review gate** — should the live-browser work land as its own PR with a separate
   security review, rather than inside this branch?

---

## 7. What this amends

- [`artifact-display-surfaces.md`](../artifact-display-surfaces.md) gains a paragraph: the panel is
  still the one surface, but it now hosts a native view as well as DOM previews, and the
  "if you are about to add a second surface" warning should name that explicitly.
- [`current-state.md`](current-state.md) becomes the *before* picture once any phase lands and
  should be re-measured, not edited from memory.

## Related documentation

- [The preview panel as it stands today](current-state.md) — the measured evidence base for every claim above.
- [Where a generated artifact is displayed](../artifact-display-surfaces.md) — the one-surface rule and what the last second renderer cost.
- [How an Auto Visualiser figure's libraries reach the renderer](../artifact-cdn-assets.md) — why displayed content never reaches the network today.
- [Privacy tiers](../../security/privacy-tiers.md) — the capability/classification lattices and the gates PP-17 reasons about.
- [Renderer testing traps](../renderer-testing-traps.md) — why a frontend test passes while the code it covers is broken.
