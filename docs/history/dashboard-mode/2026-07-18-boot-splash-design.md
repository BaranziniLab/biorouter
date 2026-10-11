# Boot splash — a centred BR mark that assembles itself

> **What this is.** The design for the desktop app's boot splash: a centred `BR` monogram that
> assembles itself out of blur over a theme-correct ground, replacing a static mark and the
> word "Loading…" in the corner.
> **Status:** Historical record (completed 2026-07-18). Approved and built on branch
> `feat/boot-splash-mark-cascade`; the splash markup and its per-family CSS are generated into
> `index.html` by `npm run themes`. **Superseded 2026-10-11:** the splash no longer draws the BR
> monogram or cascades it in. It draws the Biorouter wordmark at rest on the page canvas (first-run
> vision FR-R1-7, R-29); the current design is under "The boot splash" in the root `design.md`.
> **Audience:** developers working on the desktop app's boot path, and on theme generation.

The splash is pre-React markup in `index.html`, because that is the only placement that can
cover backend startup — the renderer awaits the daemon's host and port before React runs at all.
It appears only when the wait is real: nothing paints for the first 400 ms, so a fast boot shows
no splash rather than flashing one.

**Design tool:** [`docs/design/boot-splash-studio.html`](../../design/boot-splash-studio.html)
(self-contained; open it in a browser to replay the animation, switch theme
families, and drag every timing value)

## 1. Summary

Replace the app's boot loader — a static BR mark and the word "Loading..." in
the lower-left corner — with a centred BR monogram that assembles itself out of
blur, over a ground that matches the screen it hands off to.

The screen only appears when the wait is real: nothing paints for the first
**400 ms**, so a fast boot shows no splash at all rather than flashing one for
80 ms.

Three problems with the old loader motivated this:

1. **It never covered the slow case.** It was a React `<Suspense>` fallback, but
   `renderer.tsx` awaits `getBiorouterdHostPort()` *before* calling
   `createRoot().render()`. During backend startup — the actual slow window —
   React has not run and nothing is painted at all. The fallback only ever
   covered the much shorter `import('./App')` chunk load.
2. **It was broken in dark mode.** It mounted *above* `ConfigProvider`, so
   `useResolvedTheme()` always returned `light` and the mark rendered UCSF navy
   on a dark ground.
3. **It had no animation.** Its `page-transition` class had been deleted from
   `main.css` and survived only in a stale `dist/` bundle.

## 2. Decisions and why

### 2.1 The screen is pre-React markup in `index.html`

This is the only placement that can cover backend startup. A React fallback
structurally cannot — see §1.1. `index.html` already runs an inline script that
stamps `data-theme` and the `dark` class before first paint, so the splash reads
that state and is theme-correct from frame one, which also fixes §1.2 for free.

The CSP already permits `'unsafe-inline'` for both `script-src` and `style-src`,
so no policy change was needed.

### 2.2 Nothing on screen but the mark and a sweep bar

No wordmark and no status text. Two rounds of review removed both:

- An uppercase "BIOROUTER" set in Inter was **not part of the brand**. There is
  a real wordmark (`BioRouterWordmark.tsx`, "Bio" navy + "Router" coral), but it
  and the BR monogram carry the *same* two-tone underline — stacking them prints
  the motif twice. They are alternatives, not a pair.
- The status line went next, on the same "less on screen" reasoning.

### 2.3 The cascade moved into the mark

The original ask was to reuse the per-character reveal the chat greeting had
before `010bf68e` retired it. With all text gone there was nothing left to
apply it to, so it now runs on the mark's own four parts — `B`, `R`, then the
navy and coral halves of the underline — staggered left to right. Same
keyframes, same easing. The mark assembles rather than fading in flat.

**Stagger is 70 ms here, not the greeting's 12 ms.** Twelve was tuned for ~16
glyphs of running text; across four parts it is imperceptible.

### 2.4 The mark is static SVG, not measured at runtime

`BioRouterMark.tsx` measures itself with `getBBox()` and re-measures once
webfonts settle. On the boot path there is no time to wait for fonts before a
measurement pass, and the geometry is deterministic for Inter 800 anyway — so
the splash emits the resulting numbers directly. This also keeps it testable in
jsdom, which has no `getBBox`.

To regenerate after a brand change: render `<BioRouterMark/>` in a browser and
copy the resulting `viewBox` and child coordinates into `index.html`.

### 2.5 Ground is `background-muted`, not `background-app`

`#faf8f3` (Parchment) rather than `#ffffff`. It is the same ground the
post-boot Hub uses, so the handoff has no colour jump.

## 3. Animation specification

Per mark part, `i` = 0..3 in reveal order (B, R, underline navy, underline coral):

```js
part.animate(
  [
    { opacity: 0,   transform: 'translateX(-0.18em)', filter: 'blur(2px)' },
    { opacity: 0.7, transform: 'translateX(-0.04em)', filter: 'blur(0.6px)' },
    { opacity: 1,   transform: 'translateX(0)',       filter: 'blur(0px)' },
  ],
  {
    duration: 420,
    easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
    delay: i * 70,
    fill: 'forwards',
  }
);
```

| Value | Setting | Note |
|---|---|---|
| Threshold | 400 ms | below this, no splash is shown at all |
| Part duration | 420 ms | inherited from the greeting animator |
| Stagger | 70 ms | 4 parts, so wider than the greeting's 12 ms |
| Settle | 630 ms | `3 × 70 + 420` |
| Total from launch | 1030 ms | threshold + settle |
| Mark | 84 px square | |
| Sweep bar | fades in at 470 ms | `settle − 160` |
| Fade out | 260 ms | on dismiss |

**Units inside the SVG.** A CSS length inside an SVG is a user-space unit, not a
device pixel. `slide` and `blur` are therefore converted against the viewBox at
runtime (`165.672 / 84`); a literal `blur(2px)` would scale with the mark and
read differently at every size. A test pins this specifically.

## 4. Theme matrix

| Family | Mode | Ground | Mark ink | Accent | Sweep track |
|---|---|---|---|---|---|
| Parchment | light | `#faf8f3` | `#052049` | `#b85a32` | `#e8e1d2` |
| Parchment | dark | `#282217` | `#18a3ac` | `#b85a32` | `#3a3223` |
| Alma Mater | light | `#f2f3f4` | `#052049` | `#b85a32` | `#e1e3e5` |
| Alma Mater | dark | `#0d2a50` | `#18a3ac` | `#b85a32` | `#17386a` |
| Roche Limit | light | `#f4f4f2` | `#1f1e1c` | `#ee6c1a` | `#e4e4e0` |
| Roche Limit | dark | `#232320` | `#ededea` | `#ee6c1a` | `#302f2c` |

Parchment and Alma Mater keep the UCSF mark, where navy becomes teal on a dark
surface exactly as `BioRouterMark.tsx` does. **Roche Limit rebrands the mark
itself** — theme ink instead of navy, bright orange instead of coral — so mark
colour is a per-family decision, not a brand constant.

Each combined `html.dark[data-theme='…']` selector is deliberately more specific
than the single-condition rules, so it wins regardless of source order.

### 4.1 Why these are literals and not theme tokens 🚩

The obvious simplification — `--br-bg: var(--background-muted)`, so any new
family is correct for free — **does not work here, and fails silently.**

Tailwind v4's `@theme inline` *compiles these tokens away*: they are substituted
into utility classes at build time and do not exist as runtime custom
properties. A probe element styled `background: var(--background-muted, magenta)`
computes **magenta** in every theme; `--text-default` resolves in light but not
dark (its dark value is `var(--color-neutral-100)`, and the Tailwind palette
variables are gone too, making it invalid-at-computed-value-time).

So a token reference here does not fall back loudly — it paints the *Parchment
light* literal on every theme, which at worst means a cream flash on a dark
ground. The values must be literal, and every family needs its own rule.

### 4.2 How a new family is kept from regressing this screen

Three lists have to agree and none can import the others, because the splash
paints before React: the splash CSS, the `FAMILIES` allow-list in the
pre-React theme script, and `THEME_FAMILIES` in `ThemeContext.tsx`.

`boot-splash.test.ts` enforces the lockstep: it parses the canonical family list
out of `ThemeContext.tsx` and fails if any family lacks both a light and a dark
splash rule, or if the theme script's list has drifted. Adding a family without
touching this screen therefore breaks the build rather than shipping a wrong
ground. Mutation-checked: adding a phantom fourth family fails two tests.

## 5. Lifecycle

```text
t=0             page parsed; splash markup present but [hidden]
t=400ms         threshold crossed → splash unhidden, cascade runs
t=1030ms        settled; sweep bar looping
App mounts      DismissBootSplash effect calls window.__brBootSplash.dismiss()
                → fade out 260ms → element removed from the DOM
```

`dismiss()` is idempotent and handles both directions:

- **Never shown** (fast boot): cancels the pending show timer and removes the
  element outright, so it cannot appear after the app is already up.
- **Already shown**: adds `.br-out`, waits for the fade, then removes and
  cancels every in-flight animation so none outlive the element.

`DismissBootSplash` sits *inside* the `<Suspense>` boundary. React commits the
whole boundary in one pass, so it mounts exactly when `<App>` does — dismissing
any earlier would uncover a blank page.

**Failure path.** When `getBiorouterdHostPort()` returns null, React never
renders, so `renderer.tsx` dismisses the splash explicitly before the alert.
Otherwise the user would be left staring at a splash that nothing can remove.

## 6. Accessibility

With no visible text, a slow boot would be completely silent to a screen
reader. A visually-hidden `role="status"` announces "Loading BioRouter…".

Under `prefers-reduced-motion: reduce` the mark renders whole and immediate —
no cascade, no sweep — matching how the old greeting animator bailed out.

## 7. Scope — files this touches

| File | Change |
|---|---|
| `ui/desktop/index.html` | splash markup, scoped CSS, inline controller |
| `ui/desktop/src/renderer.tsx` | `fallback={null}`, `DismissBootSplash`, dismiss on the backend-failure path |
| `ui/desktop/src/suspense-loader.tsx` | **deleted** — the splash covers this window |
| `ui/desktop/src/suspense-loader.test.tsx` | **deleted** with it |
| `ui/desktop/src/boot-splash.test.ts` | new — 12 tests |
| `docs/design/boot-splash-studio.html` | new — the interactive design tool |

## 8. Verification

`ui/desktop/src/boot-splash.test.ts` extracts and executes the **real** markup
and script out of `index.html` rather than a copy, so the test cannot drift from
what ships. Extraction throws if the delimiters are renamed, instead of quietly
testing nothing.

Covered: four animatable parts · the `role="status"` announcement · hidden
before the threshold · shown after it · dismissed-before-threshold never appears
· dismissed-after-shown fades then removes · idempotent dismiss · animations
cancelled on teardown · stagger/duration/easing/fill per part · blur converted
to user units · reduced motion renders settled.

Mutation-checked: changing `STAGGER_MS` from 70 to 5 fails exactly one test.

## 9. Rejected alternatives

- **Keep it as a `<Suspense>` fallback.** Cannot cover backend startup (§1.1).
- **Revive `use-text-animator.tsx`.** It depends on SplitType. Pulling an
  animation library into the boot path to draw the loading screen is backwards;
  the splash needs zero dependencies.
- **Show the wordmark as well as the mark.** Doubles the underline motif (§2.2).
- **Measure the mark with `getBBox()` like the React component.** Needs a font
  wait the boot path cannot afford, and breaks in jsdom (§2.4).
- **Always show the splash with a minimum hold.** Adds a floor to every launch,
  including fast ones. The threshold gives the same brand moment only when
  there is genuinely a wait to fill.

## Related documentation

- [Dashboard mode](README.md) — the folder index this design is filed under.
- [Theme system architecture](../../design/theming/theme-system-architecture.md) — the generator that emits the splash's per-family CSS into `index.html`, and why the splash grounds are derived rather than authored.
- [Design](../../design/README.md) — the folder holding the boot-splash studio this design was drawn in.
