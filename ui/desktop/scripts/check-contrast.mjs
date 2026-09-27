#!/usr/bin/env node
/**
 * Contrast guard for the Biorouter design system.
 *
 * Parses the real token declarations out of src/styles/main.css, resolves the
 * var() chains, and asserts the WCAG 2.x contrast ratios that design.md
 * promises. Fails the build if any pair regresses.
 *
 *   node scripts/check-contrast.mjs
 *
 * THEME FAMILIES ARE DISCOVERED, NOT LISTED. The scope table comes from
 * scripts/lib/theme-tokens.mjs, which sweeps the stylesheet for
 * `[data-theme='...']` blocks. Adding a family therefore needs no edit here —
 * it is audited automatically the moment its tokens exist. The previous
 * version hardcoded six scopes built by hand, and matched their selectors by
 * exact string equality: a block written with different quoting or spacing
 * silently yielded `{}`, the scope fell back to pure Parchment, and ~40
 * assertions passed while measuring the wrong theme.
 *
 * It also asserts the CROSS-FILE duplications. Several values are necessarily
 * written twice — xterm paints to canvas and react-syntax-highlighter takes a
 * JS object, so neither can read a CSS var — and nothing used to check that the
 * copies agreed. That is how a syntax palette came to be verified against a
 * surface the app never painted, rendering `comment` at 4.15:1 with everything
 * green.
 *
 * See design.md §3.1 (Colour) and §3.8 (Focus).
 */
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import {
  buildScopes,
  discoverFamilies,
  assertBlockOrder,
  blend,
  resolveHex,
  resolveRaw,
  contrast as ratioOf,
  deltaE00,
} from './lib/theme-tokens.mjs';
import { AVATAR_HUE_COUNT } from './lib/theme-contract.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const CSS_PATH = join(here, '..', 'src', 'styles', 'main.css');

const css = await readFile(CSS_PATH, 'utf8');
const SCOPES = buildScopes(css);
const FAMILIES = discoverFamilies(css);

const resolve = (name, scope) => resolveHex(name, scope);
const ratio = ratioOf;

let failures = 0;
let checks = 0;
const rows = [];

function assert(label, fg, bg, min, scope) {
  const f = resolve(fg, scope);
  const b = resolve(bg, scope);
  if (!f || !b) {
    failures++;
    rows.push(['UNRESOLVED', '', label, `${!f ? fg : bg} does not resolve to a hex`]);
    return;
  }
  const r = ratio(f, b);
  checks++;
  const ok = r >= min;
  if (!ok) failures++;
  rows.push([ok ? 'pass' : 'FAIL', `${r.toFixed(2)}:1`, label, `${f} on ${b} (need ${min})`]);
}

/**
 * Assert against a TRANSLUCENT fill, composited over the ground it sits on.
 *
 * Everything above measures opaque token pairs, which is blind to Tailwind's
 * `/NN` opacity modifiers — and those are what several surfaces actually paint.
 * The reference chip (issue #65) is `bg-background-accent/12`: with accent ink
 * on it the label measured 3.08:1 in `alma-mater:light` inside a user bubble
 * while every assertion here stayed green, because no token named that colour.
 */
function assertOverTint(label, fg, fill, alpha, ground, min, scope) {
  const fillHex = resolve(fill, scope);
  const groundHex = resolve(ground, scope);
  if (!fillHex || !groundHex) {
    failures++;
    rows.push(['UNRESOLVED', '', label, `${!fillHex ? fill : ground} does not resolve to a hex`]);
    return;
  }
  const composited = blend(fillHex, alpha, groundHex);
  const f = resolve(fg, scope);
  if (!f) {
    failures++;
    rows.push(['UNRESOLVED', '', label, `${fg} does not resolve to a hex`]);
    return;
  }
  const r = ratio(f, composited);
  checks++;
  const ok = r >= min;
  if (!ok) failures++;
  rows.push([
    ok ? 'pass' : 'FAIL',
    `${r.toFixed(2)}:1`,
    label,
    `${f} on ${fillHex}@${alpha} over ${groundHex} = ${composited} (need ${min})`,
  ]);
}

// Grounds that body text can legitimately land on.
const TEXT_GROUNDS = [
  '--background-app',
  // The main-panel ground. Distinct from `--background-app` (the window ground
  // behind everything): this is what the conversation and every top-level view
  // actually paint, so body text lands on it constantly.
  '--background-canvas',
  '--background-default',
  '--background-muted',
  '--sidebar',
];
// Focus is a surface shift (D-15); the ring is only drawn under `prefers-contrast:
// more` / `forced-colors`. When it IS drawn it sits outside the control, so it is
// measured against the page ground rather than the control's own fill.
const RING_GROUNDS = [...TEXT_GROUNDS, '--background-medium'];

// Source order is load-bearing: `:root[data-theme=X]` and `.dark[data-theme=X]`
// have identical specificity (0,2,0), so only document order separates them.
// Swap them and dark mode renders light tokens with every ratio still passing.
for (const problem of assertBlockOrder(css)) {
  rows.push(['FAIL', '', 'block order', problem]);
  failures++;
}

for (const [theme, scope] of Object.entries(SCOPES)) {
  rows.push(['', '', `── ${theme} ──`, '']);
  for (const g of TEXT_GROUNDS) {
    assert(`${theme}: text-default on ${g}`, '--text-default', g, 4.5, scope);
    assert(`${theme}: text-muted on ${g}`, '--text-muted', g, 4.5, scope);
    assert(`${theme}: text-subtle on ${g}`, '--text-subtle', g, 4.5, scope);
  }
  for (const g of RING_GROUNDS) assert(`${theme}: focus ring on ${g}`, '--ring', g, 3.0, scope);

  assert(
    `${theme}: text-on-accent ON accent`,
    '--text-on-accent',
    '--background-accent',
    4.5,
    scope
  );
  assert(`${theme}: text-accent as text`, '--text-accent', '--background-app', 4.5, scope);

  for (const s of ['danger', 'success', 'warning', 'info']) {
    assert(`${theme}: text-${s} on app`, `--text-${s}`, '--background-app', 4.5, scope);
    assert(
      `${theme}: text-on-status ON ${s} fill`,
      '--text-on-status',
      `--background-${s}`,
      4.5,
      scope
    );
  }

  // The `Badge` accent tone — the app's one chip primitive, and what a
  // `<biorouter-ref …>` reference chip paints (issue #65). The label is 11px,
  // so it is small text and owes 4.5:1; the tint and the glyph are affordances
  // and owe 3:1. Measured on the two grounds a chip lands on: the composer
  // surface and the user bubble, whose own `--background-medium` sits under the
  // tint and is the harsher of the pair.
  for (const g of ['--background-default', '--background-medium']) {
    assertOverTint(
      `${theme}: chip label on accent/12 over ${g}`,
      '--text-default',
      '--background-accent',
      0.12,
      g,
      4.5,
      scope
    );
    assertOverTint(
      `${theme}: chip glyph on accent/12 over ${g}`,
      '--text-accent',
      '--background-accent',
      0.12,
      g,
      3.0,
      scope
    );
    assertOverTint(
      `${theme}: chip remove control on accent/12 over ${g}`,
      '--text-muted',
      '--background-accent',
      0.12,
      g,
      3.0,
      scope
    );
  }

  // Issue #56. The two badge fills, and the unfilled dense-surface mark.
  //
  // The dense mark was a filled dot and is now the padlock glyph — same token,
  // same 3:1 floor, because a stroked glyph carrying meaning owes SC 1.4.11
  // exactly what a filled one did. The two grounds below are kept as a pair:
  // the mark rides the composer's model chip today (`--background-default`),
  // and `--sidebar` is the stricter of the two in several scopes, so holding
  // both keeps the floor honest if the mark moves back onto a chat list.
  assert(`${theme}: privacy Private label`, '--text-default', '--background-muted', 4.5, scope);
  assert(`${theme}: privacy Public label`, '--text-muted', '--background-muted', 4.5, scope);
  assert(`${theme}: privacy mark on sidebar`, '--text-default', '--sidebar', 3.0, scope);
  assert(`${theme}: privacy mark on chip`, '--text-default', '--background-default', 3.0, scope);

  // `--background-medium`, which the guard has only ever looked at for the
  // focus ring. Issue #56 needs it because both privacy pills sit on rows that
  // shift to it, and nothing has ever asserted a text ratio against it.
  //
  // It is asserted OPAQUE, which is the conservative reading of two different
  // surfaces:
  //   - Painted opaque for real, by the user bubble (`UserMessage`), inline
  //     code chips and the `<biorouter-ref>` fallback (`MarkdownContent`), the
  //     tab-strip overflow button, and popover hover rows. These render exactly
  //     the ratio measured here.
  //   - `biorouter-list-row:hover` — the History/extension/settings rows — is
  //     NOT this token but `color-mix(in srgb, var(--background-medium) 42%,
  //     transparent)` over the page ground (main.css). That mix measures
  //     STRICTLY BETTER than the opaque token in all six scopes, so the floor
  //     asserted here covers the rows too. Do not quote these numbers as a
  //     row's rendered hover ratios; they are the harsher opaque case.
  //
  // ⚠ Two tokens, NOT the TEXT_GROUNDS triple. `--text-subtle` on opaque
  // `--background-medium` is sub-AA in three of the six scopes — parchment:dark
  // 3.75, alma-mater:light 4.45, alma-mater:dark 4.28 (measured with this
  // script's own resolver). That is a PRE-EXISTING theme gap, not something #56
  // introduces, and it is real on the opaque surfaces above:
  // `BrowseExtensionsModal` and `BrowseSkillsModal` both paint 10px
  // `text-text-subtle` on a `bg-background-medium` chip today, and CI has never
  // looked. It is NOT real on the hover rows — on their 42% mix `--text-subtle`
  // measures 4.54–5.93 and clears AA everywhere. Folding the token in here
  // would turn this feature red on arrival with only a theme edit, outside this
  // change, to fix it. Audit the two tokens this feature actually uses, and
  // leave the third to the a11y backlog that owns it.
  for (const t of ['--text-default', '--text-muted']) {
    assert(`${theme}: ${t.slice(2)} on --background-medium`, t, '--background-medium', 4.5, scope);
  }

  // A hairline must be perceivable against its own ground, though it is not "text".
  assert(`${theme}: border-subtle vs app`, '--border-subtle', '--background-app', 1.25, scope);
  assert(`${theme}: border-strong vs subtle`, '--border-strong', '--border-subtle', 1.1, scope);

  // `--background-muted` must actually be a step off the page it sits on.
  //
  // This is not hypothetical. Before the neutral set was shared, canvas and
  // muted were BYTE-IDENTICAL in three of the six scopes — parchment:light
  // (#faf8f3), parchment:dark (#282217) and alma-mater:dark (#0d2a50) — so any
  // element that used `bg-background-muted` to separate itself from the page
  // vanished there. The composer's chips did exactly that. Nothing failed,
  // because no assertion had ever compared two SURFACES to each other; every
  // check in this file measured ink against a ground.
  //
  // Roche Limit was the one family that kept them apart, and taking its values
  // for all three is what fixed it: 1.10:1 in light, 1.18:1 in dark, in every
  // scope. The floor is deliberately well BELOW that. The bug this catches is a
  // step of ZERO, and pinning the floor to today's measurement would fail the
  // moment someone nudged a neutral by one unit for an unrelated reason —
  // turning a real guard into noise. 1.05 flags a collapse and nothing else.
  //
  // NOT ASSERTED, for the same reason it is worth writing down: `--sidebar` vs
  // `--background-canvas`, the two-tone step, measures 1.0363:1 in every scope
  // today. It is a deliberately quiet step and it is below this floor, so
  // folding it in would fail the whole suite on arrival. It is also not the
  // same bug: the sidebar is a large region with its own border, not a chip
  // relying on fill alone.
  assert(
    `${theme}: background-muted is a step off the canvas`,
    '--background-muted',
    '--background-canvas',
    1.05,
    scope
  );

  // The code ground (design.md §5.1, P6 "the monospace layer is part of the
  // design system"). This guard previously never looked at it, which is how
  // dark code blocks came to paint --background-muted (#282217) while their
  // syntax palette was verified against #16120c: `comment` rendered at 4.15:1,
  // under AA, and nothing failed. --background-code now names the ground the
  // palette was measured on; these assertions keep the two from drifting apart
  // again. The per-token syntax stops are asserted in codeTheme.test.ts.
  // (No border-vs-code-ground assertion: a code block's hairline is perceived
  // against the PAGE ground it sits on, which is already asserted above — not
  // against its own fill. In dark, --border-subtle and --background-muted are
  // both neutral-800, so the block is delimited by its ground change alone,
  // exactly as P1 intends.)
  assert(
    `${theme}: text-default on code ground`,
    '--text-default',
    '--background-code',
    4.5,
    scope
  );
  assert(`${theme}: text-muted on code ground`, '--text-muted', '--background-code', 4.5, scope);
  assert(`${theme}: text-subtle on code ground`, '--text-subtle', '--background-code', 4.5, scope);

  // The artifact panel's paper (main.css, "Preview paper"). Inline code and
  // fenced blocks sit in `--background-well` on the `--background-default`
  // page, so the well must carry body ink AND be a visible step off the page —
  // a well equal to its page is no well, which is the case `--background-code`
  // is in dark and the reason the well is its own token. Same 1.05 floor as the
  // muted/canvas step above, for the same reason: it catches a collapse.
  assert(`${theme}: text-default on paper well`, '--text-default', '--background-well', 4.5, scope);
  assert(`${theme}: text-muted on paper well`, '--text-muted', '--background-well', 4.5, scope);
  assert(
    `${theme}: paper well is a step off the page`,
    '--background-well',
    '--background-default',
    1.05,
    scope
  );

  // Selection (main.css, `::selection`). A translucent Biorouter-orange tint
  // that deliberately leaves the ink alone, so what the eye reads is each ink
  // on the tint composited over its ground. Selected body text owes 4.5:1;
  // muted runs (a caption, a table note) owe 3:1 — selection is a transient,
  // user-driven state, and holding muted to 4.5 would force the tint so pale
  // it stops reading as a selection in dark. A link (`--text-accent`) is held
  // to the same 3:1: in Alma Mater the accent is teal, and a teal link on an
  // orange tint is exactly the pair nobody would have measured by eye. Measured
  // on the three grounds a selection lands on in the panel.
  {
    const alphaRaw = resolveRaw('--selection-alpha', scope);
    const alpha = alphaRaw && /^\d+(\.\d+)?%$/.test(alphaRaw) ? parseFloat(alphaRaw) / 100 : null;
    if (alpha === null) {
      failures++;
      rows.push([
        'UNRESOLVED',
        '',
        `${theme}: selection alpha`,
        `--selection-alpha is ${alphaRaw}`,
      ]);
    } else {
      for (const g of ['--background-default', '--background-well', '--background-muted']) {
        assertOverTint(
          `${theme}: text-default under selection over ${g}`,
          '--text-default',
          '--selection-hue',
          alpha,
          g,
          4.5,
          scope
        );
        assertOverTint(
          `${theme}: text-muted under selection over ${g}`,
          '--text-muted',
          '--selection-hue',
          alpha,
          g,
          3.0,
          scope
        );
        assertOverTint(
          `${theme}: text-accent under selection over ${g}`,
          '--text-accent',
          '--selection-hue',
          alpha,
          g,
          3.0,
          scope
        );
      }
    }
  }

  // Focus (D-15) is a surface shift. Text must stay legible on the focused fill,
  // and the focused edge must be distinguishable from the resting one.
  assert(
    `${theme}: text-default on background-focus`,
    '--text-default',
    '--background-focus',
    4.5,
    scope
  );
  assert(
    `${theme}: border-focus vs background-focus`,
    '--border-focus',
    '--background-focus',
    3.0,
    scope
  );
  assert(
    `${theme}: background-focus vs medium (hover)`,
    '--background-focus',
    '--background-medium',
    1.1,
    scope
  );

  // The focus EDGE (live QA 2026-09-24, T-16). The fill above is deliberately
  // soft — measured 1.10–1.44:1 against the resting control — so it is not an
  // indicator on its own; every focused control also draws a 2px inset edge in
  // `--border-focus` (main.css: the D-15 base rule, `.biorouter-focus-surface`),
  // a focused tab rings its label in it (Q2-49), and a focused sidebar resize
  // handle paints its 8px target with it. SC 1.4.11 asks 3:1 against every
  // colour the edge touches: the focus fill inside it, and every ground and
  // row fill the control can sit on outside it — including the sidebar's
  // active and hover rows, where the app nav's selected item lives.
  //
  // ⚠ `--border-accent` was the audit's suggestion and is deliberately NOT the
  // token: Roche Limit light's is `#ee6c1a`, 2.33:1 on the focus fill and
  // 2.88:1 on the sidebar. Asserting the neutral edge here is what keeps a
  // future swap from shipping that.
  for (const g of [
    '--background-focus',
    ...RING_GROUNDS,
    '--background-card',
    '--sidebar-hover',
    '--sidebar-active',
  ]) {
    assert(`${theme}: focus edge (border-focus) on ${g}`, '--border-focus', g, 3.0, scope);
  }
  // A solid accent control cannot take the neutral edge (1.00:1 on Parchment's
  // hover fill), so `.biorouter-focus-surface-accent` draws its edge in the
  // label ink instead. It sits on the hover fill (the focused state) and
  // replaces the resting fill's pixels, so it owes 3:1 against both.
  assert(
    `${theme}: accent focus edge (text-on-accent) on accent-hover`,
    '--text-on-accent',
    '--background-accent-hover',
    3.0,
    scope
  );

  // Status ink ON ITS OWN WASH (live QA 2026-09-24, T-18). A `Note`
  // (`ui/note.tsx`), a status chip and the tinted destructive button paint
  // `--text-X` on `--wash-X` — the hue at a fraction over whatever ground they
  // sit on — and the wash darkens (light) or lifts (dark) the ground under the
  // text. The assertion above measures the ink on `--background-app` alone,
  // which is how warning and danger shipped at 4.31:1 and 4.49:1 in Parchment
  // light with every row here green: axe caught it on the Crew banners.
  //
  // The mix is READ from the stylesheet, not assumed: the wash must stay
  // "this same ink at N% over transparent", and N is what is composited. A
  // wash rewritten into another shape fails here as UNRESOLVED rather than
  // being measured as something it no longer is. Grounds are the body-text
  // grounds, because a Note lands wherever body text does.
  for (const s of ['danger', 'success', 'warning', 'info']) {
    const raw = resolveRaw(`--wash-${s}`, scope);
    const m = raw?.match(
      /^color-mix\(in [a-z]+,\s*var\((--[\w-]+)\)\s+(\d+(?:\.\d+)?)%,\s*transparent\)$/
    );
    if (!m || m[1] !== `--text-${s}`) {
      failures++;
      rows.push([
        'UNRESOLVED',
        '',
        `${theme}: text-${s} on its wash`,
        `--wash-${s} is ${JSON.stringify(raw)}; expected color-mix(in …, var(--text-${s}) N%, transparent)`,
      ]);
      continue;
    }
    const alpha = parseFloat(m[2]) / 100;
    for (const g of TEXT_GROUNDS) {
      assertOverTint(
        `${theme}: text-${s} on its wash over ${g}`,
        `--text-${s}`,
        `--text-${s}`,
        alpha,
        g,
        4.5,
        scope
      );
    }
  }

  // Nav icons are graphical objects, not text: WCAG SC 1.4.11 asks 3:1, and it
  // asks it against every row the icon can sit on — the resting sidebar, the
  // hover fill, and the active fill. The darkest row is what binds. Alma Mater
  // paints these in the brand teal (the one place it appears at reading size),
  // so an accent change that only checked the resting sidebar could ship an
  // icon that disappears the moment a row is selected. Parchment passes these
  // trivially because its --sidebar-icon is a pass-through to the label ink.
  for (const g of ['--sidebar', '--sidebar-hover', '--sidebar-active']) {
    assert(`${theme}: sidebar icon on ${g}`, '--sidebar-icon', g, 3.0, scope);
  }

  // Person avatar hues (D-AVATAR). Three properties, each a way the set could
  // quietly stop working:
  //   - the initials are small text (11px at 20, 12 at 24, 13 at 32), so each
  //     ink owes 4.5:1 on ITS OWN fill — never measured on a neutral, because
  //     a pair is only ever painted together (`.biorouter-avatar[data-hue]`);
  //   - a fill must be a step off every ground a tile sits on, or a coloured
  //     avatar dissolves into the page. 1.1 flags a collapse, not a taste:
  //     today's minimum is 1.24 in light and 1.57 in dark;
  //   - the eight fills must stay apart, or two people read as one. ΔE00 8 is
  //     well under the ~12 they measure, so a nudge passes and a copy-paste
  //     (two slots with one value) fails. This is normal colour vision only;
  //     under a dichromacy some pairs converge, which is accepted because the
  //     initials and the @username beside the tile carry the identity.
  {
    const fills = [];
    for (let n = 1; n <= AVATAR_HUE_COUNT; n++) {
      const bg = `--avatar-hue-${n}-bg`;
      assert(`${theme}: avatar ${n} initials on its fill`, `--avatar-hue-${n}-fg`, bg, 4.5, scope);
      for (const g of TEXT_GROUNDS) assert(`${theme}: avatar ${n} fill vs ${g}`, bg, g, 1.1, scope);
      fills.push([n, resolve(bg, scope)]);
    }
    let closest = null;
    for (let i = 0; i < fills.length; i++) {
      for (let j = i + 1; j < fills.length; j++) {
        const [a, ha] = fills[i];
        const [b, hb] = fills[j];
        if (!ha || !hb) continue;
        const d = deltaE00(ha, hb);
        if (!closest || d < closest.d) closest = { a, b, d };
      }
    }
    checks++;
    const apart = closest !== null && closest.d >= 8;
    if (!apart) failures++;
    rows.push([
      apart ? 'pass' : 'FAIL',
      closest ? `ΔE ${closest.d.toFixed(1)}` : '',
      `${theme}: the ${AVATAR_HUE_COUNT} avatar fills are distinguishable`,
      closest
        ? `closest pair: ${closest.a} and ${closest.b} (need ΔE00 >= 8)`
        : 'no avatar fill resolved to a hex',
    ]);
  }

  // NOT ASSERTED: --accent-bar. It is tempting to hold the active-nav rail to
  // SC 1.4.11's 3:1, and Roche Limit's design doc used to guarantee it. The
  // measured picture, per family, light mode:
  //
  //                    on --sidebar-active   on --background-strong
  //   parchment  #cf6d47        2.53                  2.18
  //   alma-mater #16a0ac        2.23                  2.10
  //   roche      #d95b08        3.19                  2.80
  //
  // The rail's own ground is --sidebar-active (the active row paints it), where
  // Parchment and Alma Mater fail and Roche passes; on --background-strong all
  // three fail. So this is not one theme regressing — two of three have never
  // met the bar, and the rule has never been enforced. The rail reinforces a
  // background change the active row already makes, so it is not the sole cue
  // and 1.4.11 does not bite. Asserting it here would fail the default theme on
  // day one. Revisit if the rail ever becomes the only affordance.
  //
  // (An earlier draft of this comment quoted "2.80 / 2.23 / 2.18 on
  // --sidebar-active" — three numbers taken from two different grounds. The
  // table above is recomputed; do not reintroduce a figure without a ground.)
}

/**
 * THE SIX-SCOPE `--background-muted` IDENTITY — the CSS fact the knowledge-graph
 * palette rests on.
 *
 * The graph pane paints `--background-muted`, and all 35 of its hexes are
 * contrast ratios solved against it. `GRAPH_PALETTE` is therefore emitted ONCE,
 * at module scope in `themes.generated.ts`, rather than per family — and that is
 * only legitimate while the three families genuinely share the token in each
 * mode.
 *
 * Nothing else in the repo enforces the shared neutral set. Every assertion
 * above audits a family AGAINST ITSELF, so a diverged neutral passes all of
 * them while silently invalidating the palette for two families out of three.
 * That is the same class of hole the canvas/muted collapse was: no check had
 * ever compared two families' resolutions of the same token to each other.
 *
 * The generator asserts this too, from the definitions it is about to write.
 * This one asserts it from the STYLESHEET — including the hand-authored
 * `:root` / `.dark` base block, which the generator never emits and which is
 * exactly where a well-meaning neutral tweak would land.
 */
{
  const byMode = { light: new Map(), dark: new Map() };
  for (const id of ['parchment', ...FAMILIES]) {
    for (const mode of ['light', 'dark']) {
      const scope = SCOPES[`${id}:${mode}`];
      const hex = scope ? resolve('--background-muted', scope) : null;
      if (!hex) {
        rows.push([
          'UNRESOLVED',
          '',
          'graph ground',
          `--background-muted does not resolve to a hex in ${id}:${mode}`,
        ]);
        failures++;
        continue;
      }
      byMode[mode].set(hex, [...(byMode[mode].get(hex) ?? []), id]);
    }
  }
  rows.push(['', '', '── graph ground (--background-muted) ──', '']);
  for (const mode of ['light', 'dark']) {
    const shared = byMode[mode].size === 1;
    checks++;
    if (!shared) failures++;
    rows.push([
      shared ? 'pass' : 'FAIL',
      '',
      `${mode}: every family shares --background-muted`,
      shared
        ? `all families resolve ${[...byMode[mode].keys()][0]}`
        : `graph palette is emitted once because the three families share ` +
          `--background-muted; they no longer do — move GRAPH_PALETTE per-family ` +
          `or re-derive it. Resolved: ` +
          [...byMode[mode]].map(([hex, ids]) => `${hex} (${ids.join(', ')})`).join('; '),
    ]);
  }
}

const w = Math.max(...rows.map((r) => r[2].length));
for (const [status, r, label, note] of rows) {
  if (!status) {
    console.log(`\n${label}`);
    continue;
  }
  console.log(`  ${status.padEnd(10)} ${r.padStart(8)}  ${label.padEnd(w)}  ${note}`);
}

console.log(
  `\n${failures ? `FAIL — ${failures} contrast assertion(s) regressed` : `OK — all ${checks} contrast assertions pass`}`
);
process.exit(failures ? 1 : 0);
