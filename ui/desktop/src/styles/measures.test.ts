import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { SIDEBAR_COMPACT_WIDTH } from '../components/Layout/yieldLadder';
// Imported rather than re-parsed out of the component's source, which is what
// this file used to do: the sidebar's bounds now live in a pure module with no
// React and no DOM, so the values can be read directly and the regex that stood
// between this assertion and the number it asserts is gone.
import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from '../components/ui/sidebarWidth';

/**
 * The reading measures. **The two are governed by opposite rules, and that is
 * the whole point of this file.**
 *
 * `--measure-page` must stay FLUID. It governs the document-shaped views —
 * sessions, extensions, skills, schedules, workflows, applications — where a
 * wider window genuinely buys content: more table columns, more cards per row.
 * It was once a flat cap, and the symptom was reported as "the app doesn't
 * rescale with the window": dragging the window wider bought margin rather than
 * content.
 *
 * ⚠ **Settings is no longer one of them** (operator decision, 2026-09-07), and
 * this paragraph named it first until that date. Settings is a column of
 * labelled rows rather than a document, so the extra width a wide window hands
 * it lands BETWEEN each label and the control it names — margin again, just
 * distributed differently. It reads the chat measure now, which is why the
 * last describe block in this file guards that at the source.
 *
 * `--measure-chat` must stay FLAT at 760px. It was briefly widened into a clamp
 * on the same reasoning, and that was wrong for this measure specifically: a
 * 1180px composer is not a more capable composer, it is a line of prose the eye
 * has to track back across, and it drags the toolbar's controls to opposite ends
 * of the window. The design of record names "the 760px column" as Biorouter's
 * identity as an instrument (docs/design/astryx-adoption/astryx-ui-adoption-design.md
 * §1), so 760 is the measure, not the floor of one.
 *
 * ⚠ **jsdom cannot catch either direction.** It has no layout engine and never
 * runs Tailwind, so nothing that renders a component can measure a column's
 * width — a change to either declaration renders identically in every other
 * suite in this repo and ships green. The only thing assertable here is the
 * declaration itself, so that is what is asserted, at the source.
 *
 * Measured in a real browser against the built stylesheet: with the flat chat
 * measure, the composer and the transcript column both sit at 760px at every
 * window width above 760, and below it they are simply pane-wide — `max-width`
 * cannot force a box wider than its parent.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const READABLE = readFileSync(join(__dirname, '../components/Layout/ReadableContent.tsx'), 'utf8');
const MAIN = readFileSync(join(__dirname, '../main.ts'), 'utf8');
const SETTINGS_VIEW = readFileSync(
  join(__dirname, '../components/settings/SettingsView.tsx'),
  'utf8'
);

function declaration(name: string): string {
  const match = CSS.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1].trim();
}

describe('the chat measure is a flat 760px', () => {
  /**
   * Asserted as a whole-string equality rather than a pattern, because the
   * failure this guards against is a *widening* — and every loose matcher
   * (`/760px/`, `/^760/`) is satisfied by `clamp(760px, 78%, 1180px)`, which is
   * precisely the value being ruled out.
   */
  it('is exactly 760px, with no clamp and no percentage term', () => {
    expect(declaration('measure-chat')).toBe('760px');
  });

  /**
   * The composer and the transcript column read the SAME token, and must, or
   * the input would sit at a different width from the messages above it. This
   * asserts the shared key rather than the width, so the two can never drift
   * even if the number changes again.
   */
  it('is the one token the chat column and the composer both key off', () => {
    expect(READABLE).toContain("chat: 'max-w-measure-chat'");
  });
});

describe('the page measure scales with the window', () => {
  it('is a clamp, not a fixed cap', () => {
    const value = declaration('measure-page');
    expect(value).toMatch(/^clamp\(/);
    // A clamp of three pixel values would satisfy the line above and still not
    // move: the middle term is what tracks the window.
    expect(value).toMatch(/%/);
  });

  /**
   * ⚠ **A percentage, never `vw`.** It resolves against the containing block,
   * which is the content pane. `vw` is the whole viewport and would over-count
   * by the sidebar's width — widening the column at the exact moment the
   * sidebar opened and took the room away.
   */
  it('tracks the pane, not the viewport', () => {
    expect(declaration('measure-page')).not.toMatch(/\dvw/);
  });

  /**
   * The floor must not regress below what shipped, or narrow windows would get
   * NARROWER than they were — the opposite of the complaint.
   */
  it('keeps the old fixed value as its floor', () => {
    expect(declaration('measure-page')).toMatch(/clamp\(\s*1120px/);
  });

  /**
   * ReadableContent's OTHER three sizes (text / wide / graph) are page measures
   * and stay fluid. `chat` is excluded — it names the flat token above, so it
   * legitimately carries no `clamp(`.
   */
  it('leaves ReadableContent with no fixed pixel cap of its own', () => {
    const caps = READABLE.match(/max-w-\[[^\]]+\]/g) ?? [];
    expect(caps.length).toBeGreaterThan(0);
    for (const cap of caps) expect(cap).toContain('clamp(');
  });
});

/**
 * The window's minimum width is the one place a measure escapes the stylesheet.
 * It exists because the Home view's usage heatmap is the only element whose
 * size is COMPUTED rather than declared: it fits its cells to the box it is
 * given, so a window narrow enough to squeeze the reading column squeezes the
 * grid with it. The floor is therefore not a taste call — it is sidebar +
 * column, the width at which the reading column first reaches its own measure.
 *
 * ⚠ **The sidebar's DEFAULT, not its minimum.** The sidebar became
 * user-resizable, and the floor was briefly derived from the bottom of that
 * range on the argument that the minimum is the only width in it that is a
 * property of the app rather than of a preference. That gets the direction
 * backwards, and this file was rewritten to agree with it rather than catching
 * it: a floor of `SIDEBAR_MIN_WIDTH + 760` is a promise about a width no
 * install has until someone drags the edge, and at the width every install
 * ships with it leaves the column 976 − 288 = 688px — under the very measure
 * the floor exists to protect. The default is the sidebar the window must be
 * able to seat.
 *
 * The wide end of the range is not left unguarded — it is closed by
 * construction, and the second test below pins the identity that closes it.
 *
 * ⚠ Not the regression in docs/desktop-ui/window-scaling-regressions.md. That
 * one is a flat `max-width` that stops a WIDE window buying content. This is a
 * floor under a NARROW one and does nothing above it.
 *
 * Asserted as arithmetic, not as separate literals, so that changing the
 * sidebar's bounds or the chat measure fails here instead of silently leaving
 * the window able to compress the heatmap again.
 */
describe('the minimum window width is derived from the sidebar and the chat measure', () => {
  const px = (value: string): number =>
    value.endsWith('rem') ? parseFloat(value) * 16 : parseFloat(value);

  it('is exactly the default sidebar plus the reading column', () => {
    const minWidth = MAIN.match(/^\s*minWidth: (\d+),$/m);
    if (!minWidth) throw new Error('the main window declares no minWidth');

    expect(Number(minWidth[1])).toBe(SIDEBAR_DEFAULT_WIDTH + px(declaration('measure-chat')));

    // The PROPERTY the equality above exists to produce, spelled out rather
    // than left to be inferred from the arithmetic. The equality is the strict
    // form and subsumes this line today; it is written out because the equality
    // alone says only that three numbers add up, and a reader deciding which of
    // them to move needs to see WHICH WAY the relation has to hold. If the
    // exact equality is ever relaxed — a floor with slack in it would fail the
    // line above while breaking nothing — this is the assertion that must
    // survive, and swapping its constant for a narrower one is deleting the
    // property, not adjusting a number.
    expect(Number(minWidth[1]) - SIDEBAR_DEFAULT_WIDTH).toBeGreaterThanOrEqual(
      px(declaration('measure-chat'))
    );
  });

  /**
   * The wide end of the range, which the floor above deliberately says nothing
   * about: the widest the user can drag the sidebar, plus the chat measure, is
   * exactly rung 1 of the yield ladder, below which the sidebar auto-collapses
   * to an overlay and takes nothing from the chat at all.
   *
   * Asserted here rather than left as a comment because raising
   * SIDEBAR_MAX_WIDTH without moving the ladder would let a dragged-open
   * sidebar eat into the measure at every width the ladder still gives it a
   * column at. That must fail loudly rather than be rediscovered by measuring
   * the running app.
   */
  it('leaves the reading column whole even at the widest sidebar', () => {
    expect(SIDEBAR_MAX_WIDTH + px(declaration('measure-chat'))).toBe(SIDEBAR_COMPACT_WIDTH);
  });

  /** The default has to sit inside the bounds the two tests above reason about. */
  it('keeps the default width inside the resizable range', () => {
    expect(SIDEBAR_DEFAULT_WIDTH).toBeGreaterThanOrEqual(SIDEBAR_MIN_WIDTH);
    expect(SIDEBAR_DEFAULT_WIDTH).toBeLessThanOrEqual(SIDEBAR_MAX_WIDTH);
  });

  /**
   * `useContentSize` is what makes the arithmetic above comparable at all: it
   * makes `minWidth` a CONTENT width, the same coordinate space the renderer's
   * sidebar and column live in. Without it the number would be off by the
   * platform's window frame.
   */
  it('is expressed in content coordinates', () => {
    expect(MAIN).toMatch(/useContentSize: true/);
  });
});

/**
 * Settings reads the CHAT measure, not the page measure (operator decision,
 * 2026-09-07). The reasoning is in the header of this file and in the
 * `--measure-page` note in main.css; what is asserted here is only that the
 * view has not drifted back.
 *
 * ⚠ **Asserted at the SOURCE, for the same reason every other assertion in
 * this file is.** jsdom has no layout engine and never runs Tailwind, so a test
 * that renders SettingsView and reads a column's `getBoundingClientRect()` sees
 * zero whatever the size prop says — the widths this guards are only real in a
 * browser against the built stylesheet. The neighbouring
 * `SettingsView.test.tsx` asserts the rendered `data-size` attribute and its
 * count, which is the strongest statement a DOM test can make; this one closes
 * the case that test cannot see, a `<ReadableContent` added to the JSX with no
 * `size` at all, since the prop DEFAULTS to the page measure and so a
 * forgotten size is silently the wrong one.
 */
describe('Settings sits on the chat measure', () => {
  const OPENING_TAGS = SETTINGS_VIEW.match(/<ReadableContent\b[^>]*>/g) ?? [];

  /**
   * Guards against the vacuous pass: with no matches the loop below asserts
   * nothing, and renaming or removing the component would look like success.
   */
  it('renders the reading column at all', () => {
    expect(OPENING_TAGS.length).toBeGreaterThan(0);
  });

  /**
   * Every one, not "the body one". The header, the tab strip and the scrolling
   * body are three separate boxes sharing one left edge, so a size on one and
   * not its siblings is a visible step in that edge.
   */
  it('gives every reading column the chat size, none left on the default', () => {
    for (const tag of OPENING_TAGS) expect(tag).toContain('size="chat"');
  });
});
