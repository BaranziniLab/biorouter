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
- **Versions.** `scripts/release.sh` rewrites the version in `index.html`, `download.html` and `docs.html` only, and expects `about.html`'s `news-list` to start with the new release. That list holds the newest five rows, so adding a row means moving the oldest one into `.news-list-earlier`; `check-consistency.mjs` counts the five. A row's date is the day GitHub shows for the release (UTC), the same day the Download page shows.

## Look rules

These keep the site on the desktop app's scale. `node --test landing/scripts/look-rules.test.mjs` reads the stylesheets and fails on a size, weight, tracking, control height, easing or `--faint` use that breaks them (the pre-redesign stylesheets show 307 violations). A new shared rule goes in `site.css` rather than in a page.

- **Sizes** come from the `--fs-*` tokens in `site.css`: 12, 13, 14, 16, 17 and 24, plus two clamps for titles. A page stylesheet never writes a pixel size. Code is `--fs-sm`, and `--fs-xs` inside a table.
- **Weights** are 400, and 700 only for `strong` in the docs. Arial draws nothing between, so never write 500 or 600. Inter 600 sets the wordmark and nothing else.
- **Controls** are `--h-control` (32px) for small actions and `--h-cta` (40px) for the one call to action of a view. The 44px menu button is a touch target.
- **Ink**: small text uses `--subtle`. `--faint` is for the quiet second line of a large headline and for disabled rows.
- **Fills before lines** (owner message 13): a card, field, button, chip, note or diagram box rests on the subtle fill, `--fill`, with a transparent 1px edge, and steps to `--fill-strong` under the pointer, as the app does. A chip or field inside a filled box rests on the page ground (`--paper`), because a fill never sits on a fill. The art frame is a white window with a soft shadow, and what the animations draw inside it follows the app. Lines stay as section rules, table rules, the dividers between list rows, a band's edge, the coral rails and focus edges, and on floating surfaces (the menus, pickers and dialogs the animations draw keep a hairline and a shadow). Crew's own composer and fields keep the outline the Crew view still draws. The composer card the animations draw (hub, wall, skills) is the one box the fill rule does not reach (owner message 17, R-32): the page ground with the app's resting hairline (`--line-composer`, `--border-subtle` at 60%), its edge turning coral where the animation shows the box focused, and what sits in it, the idle Send, on `--fill`. A drawn border together with a radius fails the guard.
- **Nothing moves under the pointer.** A BAAM row's GitHub link, download button and chevron sit on the name's line, so an open row grows downward and the chevron just pressed stays where it was (below 861px the chevron is pinned there and the other two sit in the tag row). In the animations, a composer's model chip keeps its name and ellipsizes it, as the app's chip does, and drops the + before it drops the name, and a tool row is one line that ends in an ellipsis with its chevron after it, never a wrapped "Ran" over the extension's name. `node --test landing/scripts/layout-stability.browser.test.mjs` measures all three in Chromium.
- **Motion** uses `--dur-*` with `--ease`. Reveals take 525ms and travel 8px.
- **Headlines**: one two-tone headline per page, the first. Slack stays in the header of every page and in one community band.

## Writing for the site

Spell the product Biorouter and the marketplace BAAM. Keep sentences short and plain, with no dashes as punctuation and no marketing words. Do not name model versions outside the docs.

## Local preview

```bash
python3 -m http.server 8080
```

Then open <http://localhost:8080/>.
