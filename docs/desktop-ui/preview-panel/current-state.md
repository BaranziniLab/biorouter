# The preview panel as it stands today

> **What this is.** A measured survey of what the artifact side panel renders, how content reaches
> it, and every guard on the path — the evidence base the [expansion plan](expansion-plan.md) is
> built on. Every claim carries a `file:line`.
> **Status:** Current; measured 2026-08-23 against `main` at `edfd6325`.
> **Audience:** contributors changing the panel.

Read this before proposing a change to the panel. Three of the five things the expansion was
scoped to add turned out to be partly or wholly present already, and one of the two that are
genuinely absent is absent *by an explicit decision with a test pinning it*. Guessing at the
current surface produces a plan that rebuilds what exists and quietly reverses a security ruling.

## 1. Two types, not one

Nothing in the codebase is named `Artifact`. There are two unions.

**`ArtifactSource`** — what a host hands the panel ([artifactTypes.ts:3-27](../../../ui/desktop/src/components/artifacts/artifactTypes.ts)):

| `kind` | Fields |
|---|---|
| `'html'` | `title`, `html`, `preferredWidth?`, `preferredHeight?` |
| `'externalUrl'` | `title`, `url` |
| `'file'` | `title`, `path` |
| `'mcpResource'` | `title`, `resource: ResourceContents`, `preferredWidth?`, `preferredHeight?` |

**`ArtifactFilePreview`** — what the Electron main process returns for a `'file'`
(artifactTypes.ts:51-117): `'text' | 'html'` (one shared arm), `'image'`, `'document'`,
`'directory'`, `'gitDirectory'`, `'binary'`, `'error'`.

`ArtifactDocumentFormat` is `'pdf' | 'docx' | 'xlsx' | 'pptx'` (artifactTypes.ts:38).

## 2. What renders today

`ArtifactPreviewBody` (ArtifactViewer.tsx:868-1029) branches in source order. The whole surface:

| # | Line | Predicate | Renders |
|---|---|---|---|
| 1 | :883 | `kind === 'loading'` | spinner |
| 2 | :891 | `kind === 'error'` | `ArtifactErrorState` |
| 3 | :895 | `kind === 'html'` | srcdoc iframe |
| 4 | :913 | `kind === 'externalUrl'` | **a card, and an "Open in default browser" button — the URL is never loaded** |
| 5 | :935 | `kind === 'mcpResource'` | `UIResourceRenderer` |
| 6 | :949 | `kind !== 'file'` | `null` |
| 7 | :952 | `file.kind === 'error'` | `ArtifactErrorState` |
| 8 | :956 | `file.kind === 'image'` | `<img src={file.dataUrl}>` |
| 9 | :966 | `file.kind === 'document'` | `DocumentPreview` |
| 10 | :970 | text/html **and** extension `ipynb` | `NotebookPreview` |
| 11 | :974 | `directory` / `gitDirectory` | `DirectoryTreePreview` |
| 12 | :986 | `file.kind === 'binary'` | "This file can't be previewed here." |
| 13 | :1018 | `file.kind === 'text' \| 'html'` | `TextFilePreview` |

Branch 10 is the only extension sniff in this dispatch, and it deliberately precedes the text
branch. Everything else keys off the `kind` the main process assigned.

`TextFilePreview` (:1457-1575) sub-dispatches on the path: markdown → prose with a Raw toggle;
`.csv`/`.tsv` → `DelimitedTable`; `html` → a second srcdoc iframe; everything else → `CodeBlock`.

### Office documents already work

`DocumentPreview.tsx:480-487` dispatches `pdf → PdfPreview`, `docx → WordPreview`,
`xlsx → SpreadsheetPreview`, and falls through to `PowerPointPreview`. The four renderers are
bundled, lazily imported, and offline:

| Format | Library | Version |
|---|---|---|
| PDF | `pdfjs-dist` (legacy build + worker URL import) | ^6.2.108 |
| DOCX | `docx-preview` | ^0.4.0 |
| XLSX | `xlsx-preview` (`xlsx2Html`) | ^1.0.5 |
| PPTX | `@aiden0z/pptx-renderer` | ^1.2.4 |

Landed in `cf5ff4f1`, restyled in `8f6a9464`. Seven tests in `DocumentPreview.test.tsx`.

**The Rust converters are not a substitute.** `crates/biorouter-mcp/src/knowledge/convert/`
produces markdown for an LLM to digest and deliberately discards appearance:
`docx.rs:7-30` matches only `DocumentChild::Paragraph`, so **tables and images are silently
dropped**; `pptx.rs:226-228` writes the literal string `*[image omitted]*`;
`spreadsheet.rs:11-13` caps at 500 rows × 64 columns and keeps no formatting; `pdf.rs` extracts
text and has no rasterizer. There is no PDF-to-image path anywhere in Rust — no `pdfium`,
`mupdf`, `poppler`, `resvg` or `tiny-skia` in `Cargo.lock`. Reusing them would be a downgrade.

## 3. Images: six formats, four lists

The complete supported set is **`png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`**. Four lists must
agree for a format to work end to end:

| Location | Role |
|---|---|
| [main.ts:311-344](../../../ui/desktop/src/main.ts) `mimeTypeForArtifactPath` | **The decisive one.** `kind:'image'` is assigned by `mimeType.startsWith('image/')` at main.ts:3026 |
| [artifactUtils.ts:42](../../../ui/desktop/src/components/artifacts/artifactUtils.ts) `IMAGE_EXTENSIONS` | the "is this previewable" gate |
| [BaseChat.tsx:111](../../../ui/desktop/src/components/BaseChat.tsx) `PREVIEWABLE_TEXT_ARTIFACT_RE` | discovery from assistant prose |
| [ArtifactViewer.tsx:253](../../../ui/desktop/src/components/artifacts/ArtifactViewer.tsx) | tab icon only |

Two further lists are a **different subsystem** — pasted chat images, not the panel — and omit
`svg`: main.ts:2723 and main.ts:2859-2865. A sweep that edits them together will produce a
confusing diff.

Two *adjacent* lists are already wider than the panel and disagree with it today:
`ItemIcon.tsx:54` and `MentionPopover.tsx:427-431` both offer `bmp`, `tiff`, `ico`. A `.bmp`
offered by the mention popover currently lands in the `binary` branch. **This inconsistency
predates the expansion.**

Bytes reach the `<img>` as a base64 `data:` URI built in the main process
(main.ts:3026-3036) — no `file://`, no custom protocol, no renderer disk access. An image
therefore costs about 4/3 of its file size as a JS string, twice (IPC structured clone, then
the DOM attribute). There is no thumbnailing, no streaming and no zoom or pan.

## 4. Security: the panel's perimeter

### The artifact CSP

[artifactSecurity.ts:9-25](../../../ui/desktop/src/utils/artifactSecurity.ts):

```
default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob:; style-src 'unsafe-inline';
img-src data: blob:; connect-src 'none'; font-src data:; frame-src 'none'; worker-src blob:;
media-src data: blob:; navigate-to 'none'; form-action 'none'; base-uri 'none'; object-src 'none'
```

Frames carry `sandbox="allow-scripts allow-downloads"` — no `allow-same-origin`, and
`allow-popups` is withheld deliberately (ArtifactViewer.tsx:891-893: with it, figure HTML could
`window.open()` a `data:` URL that the main window's handler turns into a real `BrowserWindow`
inheriting the preload bridge). A test pins its absence.

Notebook outputs and spreadsheet sheets use a stricter twin with `sandbox=""` —
NotebookPreview.tsx:62, DocumentPreview.tsx:311-312.

### No displayed content has ever reached the network

The app's answer to "this content needs a remote resource" is never "let it fetch". The main
process fetches from a **seven-entry exact-URL allowlist**
([artifactCdnAssets.ts:26-37](../../../ui/desktop/src/utils/artifactCdnAssets.ts), all
`cdn.jsdelivr.net`), splices the bytes in, and *then* applies `connect-src 'none'`.

### Frame navigation is a closed set of two literals

[permissionPolicy.ts:56-58](../../../ui/desktop/src/utils/permissionPolicy.ts):

```ts
export function isAllowedArtifactFrameNavigation(candidate: string): boolean {
  return candidate === 'about:srcdoc' || candidate === 'about:blank';
}
```

Enforced on `will-frame-navigate` at main.ts:1387. `permissionPolicy.test.ts:48` explicitly
asserts `https://example.test/exfiltrate` is `false`. **This is the ruling the live-website
feature must not reverse.**

`frame-src` now says the same thing from the other end. Both policies that govern this document —
the `<meta>` in index.html:23 and the header `main.ts` attaches in `onHeadersReceived` — carry
**`frame-src 'self'`** and nothing else, since 2026-09-09. They read `'self' blob: https: http:`
until then; every one of those three was dead. Every frame the renderer creates is `srcdoc`
(`ArtifactViewer.tsx`, `DocumentPreview.tsx`, `NotebookPreview.tsx`), the blob URLs in this
renderer are `<img>` sources, downloads and `window.open` — none of which `frame-src` governs — a
PDF renders onto a canvas through pdf.js, and the live browser is a `WebContentsView`, a top-level
context outside this policy entirely. `http:` had been recorded here as load-bearing for the MCP
apps proxy iframe, which ran at the daemon's origin and was removed in September 2026
(`docs/history/mcp-apps-removal/`); `blob:` and `https:` were never traced to a frame at all.

`src/frameSrcCsp.test.ts` pins both policies **and** scans every `<iframe>` the renderer writes,
because a policy assertion alone is satisfied by a policy nobody can use: the day a component
frames a URL instead of `srcdoc`, the failure belongs at the frame rather than in a bug report
about a preview that renders blank. The narrowing itself was measured in the running dev app — an
Auto Visualiser figure, an Agent Drafter preview card, a PDF, a notebook and a workbook all
rendered with zero `securitypolicyviolation` reports — because jsdom has no CSP engine and can
prove nothing about a policy.

### What is *not* guarded

- **`.biorouterignore` is never consulted on the desktop panel path.** It is enforced only in the
  Rust backend (`crates/biorouter-mcp/src/secret_guard.rs`). The panel's only gates are
  `isAllowedFilePath` (main.ts:230) → `allowedFileRoots()` + `isSensitivePreviewPath`. In
  **Completely Autonomous** mode the containment check is skipped for anything not on the
  sensitive-prefix list. If the panel's reach widens, `.biorouterignore` will not narrow it.
- ~~**`frame-src 'self' blob: https: http:`** in both CSP sources.~~ **Closed 2026-09-09** —
  narrowed to `'self'` in both, with the reasoning and the guard recorded under *Frame navigation
  is a closed set of two literals* above. The line reference this bullet carried (`main.ts:4825`)
  was already stale when it was written; grep the directive rather than trusting a line number.
- ~~No `webviewTag`, no `<webview>`, no `BrowserView`, no `WebContentsView` anywhere in the
  desktop source.~~ **Stale as of the live-browser work**: `utils/embeddedBrowser.ts` hosts a
  `WebContentsView`. That is what makes the `frame-src` narrowing above safe rather than lucky —
  the live browser is a top-level browsing context, so it never needed a frame source.
- No `capturePage` and no `desktopCapturer` anywhere. Screenshotting is entirely new surface.

### Pre-existing asymmetries found while surveying

Not introduced by any planned work, and worth fixing on their own schedule:

- `read-file` and `delete-file` call `isAllowedFilePath()`; **`write-file` (main.ts:3076),
  `ensure-directory` (:3099), `list-files` (:3111), `list-skill-dirs` (:3153) and
  `open-directory-in-explorer` (:5374) do not.**
- The launcher window (main.ts:1917) and the Agent-Drafter app window (main.ts:5513) carry the
  full preload with **no** `setWindowOpenHandler`, `will-navigate` or `will-frame-navigate`
  guard. There is no `app.on('web-contents-created')` catch-all, so guards are per-window.
- `window.electron.on(channel, cb)` (preload.ts:585) accepts any channel with no allowlist.

## 5. Caps

| Cap | Value | Where |
|---|---|---|
| `ARTIFACT_PREVIEW_MAX_BYTES` | 16 MiB | main.ts:288, applied :3000 |
| `MAX_TABLE_ROWS` | 500 | ArtifactViewer.tsx:77 |
| `MAX_LINE_NUMBERED_LINES` | 5,000 | ArtifactViewer.tsx:80 |
| `RENDER_ERROR_TEXT_LIMIT` | 2,000 chars | ArtifactViewer.tsx:240 |
| `MAX_BROWSER_URL_BYTES` | 8 KiB | artifactUtils.ts:45 |
| `MAX_DIRECTORY_TREE_ENTRIES` / depth | 10,000 / 32 | artifactDirectory.ts:13-14 |
| Pasted-image data URL | 4 MiB string, 3 MiB decoded | main.ts:2616, :2640 |
| Images per message | 5 | ChatInput.tsx:66-67 |

Markdown length, notebook cell count and CSV *parsing* are uncapped — only CSV *rendering* is.

## 6. How an annotation would reach the model

The image path needs **no schema change on either side**:

```
canvas.toDataURL() → window.electron.saveDataUrlToTemp() → { path, kind: 'image' }
  → createUserMessage() → { type: 'image', data: <raw base64>, mimeType }
```

`types/message.ts:12-39`, main.ts:2602-2660. Note the main-process regex at main.ts:2622 admits
only `png|jpeg|jpg|gif|webp` — **not `svg`** — and the 4 MiB/3 MiB caps apply.

For text, the precedent is the `<biorouter-ref>` chip system (issue #65):
`resourceRefs.ts:49-52` (`RefKind = 'skill' | 'extension' | 'knowledge_base'`),
`composerRefs.ts`, `ResourceRefChip.tsx:76`, and the rail at ChatInput.tsx:2366-2379 — placed
*above* the textarea deliberately, because a reference qualifies the whole message while the row
below is the attachments area. The payload lives **inside the message string** so that draft
save/restore, `?prompt=` deep links, the queue, steering and local history cannot each forget it
(composerRefs.ts:3-24).

There is no `getSelection`, no `Range` capture, no highlight overlay and no image-crop code in
the renderer today. All of it is new.

**Any channel out of a preview frame must reuse the existing provenance gate** —
ArtifactViewer.tsx:543-544 accepts a `message` event only when
`event.source === trustedFrameRef.current?.contentWindow`, because (:537-542) an untrusted frame
"would otherwise be able to inject instructions into a session that holds shell and file tools."

## 7. Test coverage, and its holes

`ArtifactViewer.test.tsx` holds 38 tests; the artifact directory plus `artifactSecurity`,
`pathContainment` and `BaseChat.artifacts` run **197 tests across 7 files**, all green at
`edfd6325`.

The holes that matter here:

- **No test renders the `image` branch, the `binary` branch, or the `document` branch** from
  `ArtifactViewer`. Documents are covered separately in `DocumentPreview.test.tsx`; images and
  binaries are covered nowhere.
- **No `useArtifactPanel.test.ts` exists.** The panel's geometry and open/close machine are
  tested only indirectly, through `yieldLadder.test.ts` and `BaseChat.artifacts.test.ts`.
- jsdom evaluates no `@container` query, runs no Tailwind and has no layout engine, so panel
  geometry regressions are invisible to the suite. `ui/desktop/.artifact-harness/` mounts the
  real `ArtifactViewer` in a browser for exactly this reason.

## Related documentation

- [Preview panel expansion plan](expansion-plan.md) — what this survey was gathered for.
- [Where a generated artifact is displayed](../artifact-display-surfaces.md) — the one-surface rule, and what the removed inline renderer cost.
- [How an Auto Visualiser figure's libraries reach the renderer](../artifact-cdn-assets.md) — the CDN pre-fetch and inline mechanism.
- [Renderer testing traps](../renderer-testing-traps.md) — why a frontend test passes while the code it covers is broken.
