# Biorouter website

The static site served at <https://biorouter.ucsf.edu/>. It is published from this folder as it is, with no build step, by [`.github/workflows/deploy-landing.yml`](../.github/workflows/deploy-landing.yml) on every push to `main` that touches `landing/`.

## Pages

| File | Page |
| ---- | ---- |
| `index.html` | Introduction: the model routing hub, the three principles, a Slack band and six capability tiles |
| `download.html` | Download: the detected installer, every platform, first steps, the command line and servers |
| `docs.html` | Documentation assembled from the tracked shell and fourteen fragments in `docs/website/` |
| `baam.html` | BAAM, the Biorouter AI Agent Marketplace: extensions, skills and workflows |
| `about.html` | About: the team, collaborators, inspirations and updates |
| `intro.html`, `skills.html` | Redirects kept for old links (to `index.html` and `baam.html`) |

## Shared files

- `site.css`: every design token (they mirror the desktop app's light Parchment theme) and the shared parts: header with the Slack invite, footer, buttons, text links, section rules, privacy pills, tables, artifact frames.
- `site.js`: header state, mobile menu, reveal on scroll, copy buttons, the release version (`data-version` slots refreshed from the GitHub releases API), and the artifact registry.
- `home.css`, `download.css`, `about.css`, `docs.css`, `baam.css`: one stylesheet per page.
- `art/<name>.js` and `art/<name>.css`: the small animations. Each registers with `BR.art('<name>', mount)` and fills a `<div data-art="<name>">`. `site.js` starts it when it is on screen and stops it when it leaves, and each one draws a still frame when the visitor asks for reduced motion.
- `icon.svg`, `icon-transparent.svg`, `icon.png`: the BR mark. Do not edit them; `scripts/check-brand-consistency.sh` compares them with the app's copies.
- `registry.json`: the marketplace catalog, generated from `baam.html`'s static cards. The app compiles it in.
- `marketplace-search.js`: the BAAM search matcher, shared with a Rust test. Leave its formatting alone.
- `shared.css`, `theme.js`, `app-mockups.css`, `app-mockups.js`: the previous design's files. No page loads them now; `video/reel` still does, and `scripts/check-consistency.mjs` reads `app-mockups.js`.

## Rules the checks enforce

- **BAAM.** `baam.html`'s static cards are the input of `scripts/build-registry.mjs`, which writes `registry.json`, the desktop app's bundled copy and the Rust privacy list. After any card change run `node landing/scripts/build-registry.mjs`; in CI `--check` fails if the three copies differ. The browser tests in `scripts/baam-privacy-facet.test.mjs` and `scripts/baam-search.test.mjs` hold the rendered page to the same contract.
- **Docs.** Edit the sources in [`docs/website/`](../docs/website/README.md), then run `node landing/scripts/assemble-docs.mjs` from the repository root. Do not hand edit `docs.html`. `scripts/check-docs-privacy.mjs` reads the "Extension agents in the marketplace" table, and `../scripts/check-crew-manual.mjs` reads the Crew page and the browser table. Both match markup byte for byte.
- **Versions.** `scripts/release.sh` rewrites the version in `index.html`, `download.html` and `docs.html` only, and expects `about.html`'s `news-list` to start with the new release.

## Look rules

These keep the site on the desktop app's scale. No check enforces them, so a new rule goes in `site.css` rather than in a page.

- **Sizes** come from the `--fs-*` tokens in `site.css`: 12, 13, 14, 16, 17 and 24, plus two clamps for titles. A page stylesheet never writes a pixel size. Code is `--fs-sm`, and `--fs-xs` inside a table.
- **Weights** are 400, and 700 only for `strong` in the docs. Arial draws nothing between, so never write 500 or 600. Inter 600 sets the wordmark and nothing else.
- **Controls** are `--h-control` (32px) for small actions and `--h-cta` (40px) for the one call to action of a view. The 44px menu button is a touch target.
- **Ink**: small text uses `--subtle`. `--faint` is for the quiet second line of a large headline and for disabled rows.
- **Motion** uses `--dur-*` with `--ease`. Reveals take 525ms and travel 8px.
- **Headlines**: one two-tone headline per page, the first. Slack stays in the header of every page and in one community band.

## Writing for the site

Spell the product Biorouter and the marketplace BAAM. Keep sentences short and plain, with no dashes as punctuation and no marketing words. Do not name model versions outside the docs.

## Local preview

```bash
python3 -m http.server 8080
```

Then open <http://localhost:8080/>.
