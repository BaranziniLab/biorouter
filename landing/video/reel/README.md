# Feature reel (standalone slideshow generator)

> The **canonical** landing-site videos are produced by the Hyperframes
> pipeline in [`../`](../) (`release.html`, `tour.html`, `hero`, etc.). This
> `reel/` folder is a **standalone alternate**: a simple, dependency-light way
> to render a "what's new" slideshow straight from the site's own app mockups,
> kept here for reference. It is **not** wired into any page.

`reel.html` is a self-contained 1920×1080 slide deck (intro → one slide per
feature, each mounting a real `[data-screen]` app mockup → outro). It reuses the
site's own `../../shared.css`, `../../app-mockups.css`, `../../app-mockups.js`,
and `../../icon.svg`, and exposes a deterministic capture API
(`window.__slideCount`, `window.__goto(i)`).

## Regenerate `biorouter-reel.mp4`

```bash
cd video/reel

# 1. Capture one PNG per slide into ./frames (needs Playwright + Chrome).
#    Install Playwright here (npm i playwright) or point NODE_PATH at one:
NODE_PATH=/path/to/node_modules node capture-reel.cjs

# 2. Stitch the frames into biorouter-reel.mp4 (+ poster) with crossfades.
python3 encode.py
```

Outputs `biorouter-reel.mp4` and `biorouter-reel-poster.jpg` in this folder.
Edit slide text/order in `reel.html`; edit per-slide durations in `encode.py`.
