# Biorouter landing-site videos (Hyperframes)

Motion assets for the Biorouter landing site, authored as plain HTML/CSS +
[GSAP](https://gsap.com) and rendered to deterministic MP4/WebM with
[Hyperframes](https://github.com/heygen-com/hyperframes) (Chrome + FFmpeg).

Everything here renders to `../assets/videos/` (`.mp4` + `.webm` + a poster
`.jpg`), which the pages reference. **Design truth lives in
[`frame.md`](./frame.md)** — read it before editing any composition.

## What's here

| File              | Deliverable | Length | Where it's used on the site                 |
|-------------------|-------------|--------|---------------------------------------------|
| `index.html`      | #1 Hero     | 32s    | `index.html` hero — "Biorouter in motion"   |
| `tour.html`       | #3 Tour     | 48s    | `index.html` Product-tour player (chaptered)|
| `baam.html`       | #4 BAAM     | 16s    | `baam.html` — composable-agent motion graphic|
| `release.html`    | #2 Release  | 17s    | `about.html` — "What's new" (variable-driven)|
| `frame.md`        | #5 Spec     | —      | the motion design system                    |

Shared tokens/components live in `assets/theme.css`; GSAP is vendored at
`assets/gsap.min.js` (no CDN at render time = deterministic, offline renders).

## Prerequisites

- Node 22+ and FFmpeg (`brew install ffmpeg`). Verify with
  `npx hyperframes doctor`. Docker is *not* required (we render locally).
- Compositions render with the **system Helvetica/Arial** stack to match the
  site. `hyperframes lint` warns "font_family_without_font_face" — that is a
  false positive for local macOS renders (Chrome supplies the fonts); don't
  pass `--strict`.

## Regenerate the videos

The app-UI scenes are **not hand-approximated** — `index.html` and `tour.html`
vendor the site's own `assets/app-mockups.js` + `assets/app-mockups.css` and
mount the real `[data-screen]` mockups (Home / Chat / Tabs / Knowledge /
Workflows / Models), so the UI in the videos is byte-identical to the live app.
A small `fitMock()` in each composition scales each `.bw` window to fit the
frame. To refresh them, copy `../app-mockups.js` and `../app-mockups.css` over
the copies in `assets/`, and copy the light `:root` tokens of `../shared.css`
into the `.bw` block of `assets/theme.css`, which the mockups read. `assets/icon.svg` (also copied to `video/icon.svg`) is the real logo used
in the intro, the HUD wordmarks, and the mockups' sidebar footer.

Videos are rendered at **4K** and downscaled (Lanczos) to a crisp **1440p**
(2560×1440) delivery — supersampling keeps fine UI text sharp.

```bash
cd video

# Verify a composition before rendering (PNG key frames into snapshots/)
npx hyperframes snapshot --at 2,7,12          # uses index.html
#   to snapshot a non-index file, temporarily copy it to index.html

# Render one composition at 4K, then downscale to 1440p delivery + WebM + poster
M=$(mktemp).mp4
npx hyperframes render -c tour.html --resolution landscape-4k --quality high --fps 30 -o "$M"
ffmpeg -y -i "$M" -vf scale=2560:1440:flags=lanczos \
  -c:v libx264 -crf 20 -preset slow -pix_fmt yuv420p -movflags +faststart -an \
  ../assets/videos/biorouter-tour.mp4
ffmpeg -y -i "$M" -vf scale=2560:1440:flags=lanczos \
  -c:v libvpx-vp9 -b:v 0 -crf 31 -row-mt 1 -an ../assets/videos/biorouter-tour.webm
ffmpeg -y -ss 5 -i "$M" -vf scale=1920:-1:flags=lanczos -frames:v 1 -q:v 2 \
  ../assets/videos/posters/biorouter-tour.jpg
rm -f "$M"
```

Output filenames the pages expect:
`biorouter-hero`, `biorouter-tour`, `biorouter-baam`,
`biorouter-release-v<version>` (each as `.mp4`, `.webm`, and a poster
`.jpg` under `../assets/videos/posters/`).

## #2 — generate a release video for a new version

The release clip is **data-driven** — no HTML editing per release:

1. Copy `releases/v1.86.0.json` to `releases/v<X.Y.Z>.json` and edit the
   `version`, `date`, `headline`, and up to **eight** `feat*` strings (empty
   `feat*` rows are hidden automatically). `feat1`–`feat4` fill the first
   feature scene (titled by `featsTitleA`, default "What changed"); `feat5`–
   `feat8` fill an optional second scene (`featsTitleB`, default "And more")
   that is skipped entirely — with no blank tail — when those four are empty.
   Use the full eight for a milestone "everything since vX" recap (see
   `releases/v1.86.1.json`); use four for a normal point release.
2. Run the generator:

   ```bash
   scripts/make-release-video.sh <X.Y.Z>
   ```

   It renders `release.html` with those variables and writes
   `biorouter-release-v<X.Y.Z>.{mp4,webm}` + poster.
3. Point the About-page `<video>` (`about.html`, the `.release-video` block)
   at the new filenames, and refresh the headline/`<h2>`.

## Editing tips

- Compositions are **monolithic**: one `#root` clip, one paused GSAP timeline,
  scenes gated by `opacity` (no overlapping tracks). See `frame.md` §4–5.
- Stick to the animatable property allowlist (`opacity, x, y, scale, rotation,
  color, backgroundColor, borderRadius`) — never animate layout
  (`width/height/top/left/display`). Caret/float loops use **finite** repeats.
- After editing, re-run `snapshot` and eyeball the frames before a full render.
- `npx hyperframes preview` opens the in-browser Studio timeline editor.
