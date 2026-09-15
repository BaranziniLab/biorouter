import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  PREVIEW_MAX_WIDTH,
  PREVIEW_MIN_WIDTH,
  PREVIEW_PREFERRED_CHAT_WIDTH,
  PREVIEW_SIDE_STRIP_HEIGHT,
  PREVIEW_SIDE_WIDTH,
  PREVIEW_STACK_EDGE_HEIGHT,
  PREVIEW_STACK_MIN_HEIGHT,
  PREVIEW_STACK_RATIO,
  PREVIEW_STACK_STRIP_HEIGHT,
  PREVIEW_TRANSCRIPT_MIN_HEIGHT,
  PREVIEW_DEFAULT_WIDTH_RATIO,
  READABLE_CHAT_WIDTH,
  SIDEBAR_COMPACT_WIDTH,
} from '../components/Layout/yieldLadder';
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
 * `--measure-page` must stay FLUID, and it is still a clamp for that reason: it
 * was once a flat cap, and the symptom was reported as "the app doesn't rescale
 * with the window" — dragging the window wider bought margin rather than
 * content. That rule has not changed and this file still guards it.
 *
 * ⚠ **What HAS changed is that nothing reads it any more** (operator decision,
 * 2026-09-07). This paragraph used to say the token "governs the
 * document-shaped views — extensions, skills, schedules, workflows,
 * applications", and named Settings and sessions before that. Every one of
 * those has since been measured and moved: none is a document, each is a column
 * of labelled rows, so the extra width a wide window handed it landed BETWEEN
 * each label and the thing it names — margin again, just distributed
 * differently. In Settings that separated a control from its label; in Chat
 * history it left the per-chat counts about 700px from the chat they count,
 * measured at 1440; in the component views it opened a gap between a row's
 * title and its own actions.
 *
 * So `CHAT_MEASURE_VIEWS` below is now every view in `components/`, and the
 * page measure's readers number zero. Neither the token nor `ReadableContent`'s
 * `text` size is deleted for that: `text` is still the DEFAULT, which is
 * precisely why the guard names each view at the SOURCE instead of trusting
 * it, and a view that genuinely is a document should still have a measure to
 * reach for.
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

/**
 * Every view that reads the CHAT measure and is not the chat itself. Settings
 * joined on 2026-09-07 (#172); the three chat-history surfaces joined with it,
 * one PR later; the Scheduler and then the five component views joined in the
 * two PRs after that. They are listed together because the rule is one rule — a
 * `<ReadableContent` here without `size` silently takes the PAGE measure — and
 * a per-file copy of it is how three of the four would come to say something
 * slightly different. (It had already started: the Scheduler arrived with its
 * own describe block asserting the same thing in its own words, and that block
 * is folded in here.)
 *
 * ⚠ **After this list, `--measure-page` has NO reader left in `components/`.**
 * The token and `ReadableContent`'s `text` size both stay: deleting either is a
 * separate decision, not a consequence of this one, and `text` remains the
 * default a new view gets when it says nothing — which is exactly why the
 * source rule below has to name every view rather than trusting the default.
 */
const CHAT_MEASURE_VIEWS = [
  'settings/SettingsView.tsx',
  'sessions/SessionListView.tsx',
  'sessions/SessionHistoryView.tsx',
  'sessions/SharedSessionView.tsx',
  'schedule/SchedulesView.tsx',
  'schedule/ScheduleDetailView.tsx',
  'workflows/WorkflowsView.tsx',
  'extensions/ExtensionsView.tsx',
  'skills/SkillsView.tsx',
  'applications/ApplicationsView.tsx',
  // The catch-all's page. It is a route view like the rest and shares the
  // hairline with whatever the user came from, so it takes the same column.
  'NotFoundView.tsx',
].map((rel) => ({
  rel,
  source: readFileSync(join(__dirname, '../components', rel), 'utf8'),
}));

/**
 * Every view that must mount the SHARED page header rather than write its own.
 *
 * This is the assertion that stops the drift coming back, and it has to be a
 * source assertion: eight views each had their own copy of the header before
 * `PageHeader` existed, and the copies disagreed about the hairline, the
 * description's type role, the padding and where the actions went — none of
 * which any render test noticed, because each view rendered exactly what it
 * meant to. What no view can now do is mean something different.
 *
 * `ScheduleDetailView` and the two transcripts are deliberately ABSENT: they
 * are drill-in surfaces with a Back-button header, which is a different object
 * from a page header and keeps its own shape.
 */
const PAGE_HEADER_VIEWS = [
  'settings/SettingsView.tsx',
  'sessions/SessionListView.tsx',
  'schedule/SchedulesView.tsx',
  'workflows/WorkflowsView.tsx',
  'extensions/ExtensionsView.tsx',
  'skills/SkillsView.tsx',
  'applications/ApplicationsView.tsx',
  'NotFoundView.tsx',
].map((rel) => ({
  rel,
  source: readFileSync(join(__dirname, '../components', rel), 'utf8'),
}));

/**
 * The source with every comment removed, so a rule can ban a class NAME without
 * banning the sentence that explains why it is banned. `SessionHistoryView.tsx`
 * names `max-w-4xl` in prose precisely to stop it coming back, and a plain
 * `text.includes('max-w-4xl')` would read that as the defect.
 *
 * String and template literals are copied through intact rather than scanned,
 * so a `//` inside one cannot start a comment — which is the direction that
 * matters, since a className is a string. The residual limitation is a bare
 * `//` in JSX *text* (a URL, say): it would swallow the rest of that line and
 * could hide a class written after it on the same line. Prettier puts
 * `className` on its own line, so that arrangement does not occur here, and the
 * self-check below pins the stripper against a real comment rather than
 * trusting this paragraph.
 */
function codeWithoutComments(source: string): string {
  let out = '';
  let index = 0;
  while (index < source.length) {
    const pair = source.slice(index, index + 2);
    if (pair === '//') {
      while (index < source.length && source[index] !== '\n') index += 1;
      continue;
    }
    if (pair === '/*') {
      index += 2;
      while (index < source.length && source.slice(index, index + 2) !== '*/') index += 1;
      index += 2;
      continue;
    }
    const character = source[index];
    if (character === '"' || character === "'" || character === '`') {
      out += character;
      index += 1;
      while (index < source.length) {
        if (source[index] === '\\') {
          out += source.slice(index, index + 2);
          index += 2;
          continue;
        }
        out += source[index];
        index += 1;
        if (source[index - 1] === character) break;
      }
      continue;
    }
    out += character;
    index += 1;
  }
  return out;
}

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
 * Settings and the three chat-history surfaces read the CHAT measure, not the
 * page measure (operator decision, 2026-09-07). The reasoning is in the header
 * of this file and in the `--measure-page` note in main.css; what is asserted
 * here is only that the views have not drifted back.
 *
 * ⚠ **Asserted at the SOURCE, for the same reason every other assertion in
 * this file is.** jsdom has no layout engine and never runs Tailwind, so a test
 * that renders one of these views and reads a column's
 * `getBoundingClientRect()` sees zero whatever the size prop says — the widths
 * this guards are only real in a browser against the built stylesheet. The
 * neighbouring component tests assert the rendered `data-size` attribute and
 * its count, which is the strongest statement a DOM test can make; this one
 * closes the case those cannot see, a `<ReadableContent` added to the JSX with
 * no `size` at all, since the prop DEFAULTS to the page measure and so a
 * forgotten size is silently the wrong one.
 */
describe.each(CHAT_MEASURE_VIEWS)('$rel sits on the chat measure', ({ source }) => {
  /**
   * ⚠ Comments stripped FIRST, and this is not hypothetical tidiness: several of
   * these views quote the string `<ReadableContent` in a comment explaining why
   * every one of theirs carries the size. Matched raw, that sentence is a tag
   * with no `size=` in it and the rule fails on the file that documents itself
   * best. The same trap `codeWithoutComments` was written for one rule below.
   */
  const OPENING_TAGS = codeWithoutComments(source).match(/<ReadableContent\b[^>]*>/g) ?? [];

  /**
   * Guards against the vacuous pass: with no matches the loop below asserts
   * nothing, and renaming or removing the component would look like success.
   */
  it('renders the reading column at all', () => {
    expect(OPENING_TAGS.length).toBeGreaterThan(0);
  });

  /**
   * Every one, not "the body one". A view's header, tab strip and scrolling
   * body are separate boxes sharing one left edge, so a size on one and not its
   * siblings is a visible step in that edge.
   */
  it('gives every reading column the chat size, none left on the default', () => {
    for (const tag of OPENING_TAGS) expect(tag).toContain('size="chat"');
  });

  /**
   * The 896px "replay fork", closed. `SessionHistoryView` drew its transcript
   * in a `max-w-4xl` box NESTED inside the page's reading column, so the saved
   * conversation had two ceilings and the inner one won — a conversation
   * rendered at a width the live chat never uses. Deleting it is only half the
   * fix: any `max-w-*` written inside one of these views takes precedence over
   * the column again, silently, and looks like a local tweak rather than a
   * second measure. `ReadableContent` is the one box allowed to carry a
   * ceiling, and it carries it as a token.
   */
  it('declares no second measure of its own', () => {
    expect(codeWithoutComments(source)).not.toMatch(/\bmax-w-(?:3xl|4xl|5xl|6xl|7xl)\b/);
  });
});

/**
 * The instrument, checked before the rule that depends on it. `max-w-4xl` is
 * named in prose in `SessionHistoryView.tsx` — deliberately, so the fork cannot
 * come back unexplained — and a stripper that silently did nothing would turn
 * the rule above into a permanent failure, while one that stripped too much
 * would turn it into a permanent pass. Both directions are pinned here.
 */
describe('the comment stripper the measure rule depends on', () => {
  const HISTORY = CHAT_MEASURE_VIEWS.find((view) =>
    view.rel.endsWith('SessionHistoryView.tsx')
  )!.source;

  it('removes a class named in prose', () => {
    expect(HISTORY).toContain('max-w-4xl');
    expect(codeWithoutComments(HISTORY)).not.toContain('max-w-4xl');
  });

  /**
   * Deliberately NOT `size="chat"`, which the rule above already asserts: a
   * self-check that fails for the same reason as the rule it underwrites tells
   * you nothing about the instrument. These two classes are applied, are not
   * the subject of any other assertion here, and one of them (`px-6`) sits on
   * the very line a too-eager stripper would eat.
   */
  it('keeps the classes that are actually applied', () => {
    const code = codeWithoutComments(HISTORY);
    expect(code).toContain('biorouter-page-header');
    expect(code).toContain('px-6');
  });
});

/**
 * The Scheduler joined the chat measure on 2026-09-07, for the same reason
 * Settings did: both of its surfaces are columns of rows — a schedule and its
 * status, a label and the fact it names — so width past the measure lands
 * between the two halves of every row rather than showing more. Both files are
 * in `CHAT_MEASURE_VIEWS` above; what is left here is the one assertion that is
 * about the detail view and nothing else.
 */
describe('the schedule detail sizes itself from its parent', () => {
  /**
   * `ScheduleDetailView` used to size itself `h-screen w-full`, which
   * `MainPanelLayout`'s own comment names as the anti-pattern that breaks an
   * embedded pane: it forces viewport height whatever the parent's rect is, so
   * a 420px chat pane rendered a ~1050px panel. The rebuild puts the view on
   * `MainPanelLayout`, whose `h-full` fills the container instead.
   *
   * Comments are stripped first: the file's own docblock names `h-screen` while
   * explaining why it is gone, and a raw substring search reads that as the
   * defect. The same technique `settingsVocabulary.test.ts` uses for its
   * banned-class rule.
   */
  it('never forces the viewport height', () => {
    const detail = CHAT_MEASURE_VIEWS.find(({ rel }) => rel.endsWith('ScheduleDetailView.tsx'));
    if (!detail) throw new Error('ScheduleDetailView.tsx is not in CHAT_MEASURE_VIEWS');
    const code = codeWithoutComments(detail.source);
    expect(code).not.toContain('h-screen');
    expect(code).toContain('<MainPanelLayout>');
  });
});

/**
 * One page header, mounted — not eight near-copies of one.
 *
 * ⚠ Asserted at the SOURCE, and it has to be. jsdom renders each of the eight
 * old headers perfectly well; what it cannot see is that they disagreed with
 * each other. Nor can a `PageHeader.test.tsx` assertion, which proves the
 * primitive is right and says nothing about whether a view uses it.
 */
describe.each(PAGE_HEADER_VIEWS)('$rel mounts the shared page header', ({ source }) => {
  const code = codeWithoutComments(source);

  it('imports PageHeader and renders it', () => {
    expect(code).toMatch(/import \{[^}]*\bPageHeader\b[^}]*\} from '[^']*Layout\/PageHeader'/);
    expect(code).toContain('<PageHeader');
  });

  /**
   * The header it replaced, banned by its parts. A view that mounted
   * `PageHeader` and then left its old block in place beside it would satisfy
   * the assertion above — that is not hypothetical, it is the shape a partial
   * conversion takes.
   *
   * Two fingerprints, both chosen for being unambiguous. `<h1` because a page
   * has exactly one title and the primitive owns it. `pt-12` because that is
   * the page header's top inset and nothing else in these files has a reason
   * to be inset from a hairline that is not there.
   *
   * ⚠ `border-b border-border-subtle` — the pair all eight copies wrote, and
   * the obvious thing to ban — is NOT usable, and the reason generalises: it is
   * also how you draw a divider between two ROWS. `SessionListView`'s loading
   * skeleton uses it correctly, so banning the string reports a view that did
   * exactly what it was asked. Ban a shape only where the shape has one
   * meaning.
   */
  it('keeps no second copy of the header it replaced', () => {
    expect(code).not.toContain('<h1');
    expect(code).not.toMatch(/\bpt-12\b/);
  });

  /**
   * The action row is the primitive's, not the call site's. `flex gap-3 mt-5`
   * is what four of the views wrote; `mt-5` alone is what the strip carries, so
   * the ban is on the hand-rolled flex row rather than on the offset.
   */
  it('hand-rolls no action row of its own', () => {
    expect(code).not.toMatch(/className="flex gap-3 mt-5"/);
  });
});

/**
 * RUNG 2 — THE PREVIEW SPLIT, pinned at the source.
 *
 * The decision lives in `yieldLadder.ts` and is unit-tested there on both sides
 * of every threshold. What this block guards is the OTHER half, which no
 * component test can see: jsdom never loads `main.css`, never lays out a grid and
 * never evaluates a custom property, so a render test of the split box passes
 * whether the stylesheet agrees with the ladder or not. So the stylesheet's
 * literals are asserted against the ladder's constants, the rules are asserted
 * unlayered and in order, and the hosts are asserted to mount the panel the way
 * the no-remount guarantee needs.
 */
const PANEL_HOOK = readFileSync(
  join(__dirname, '../components/artifacts/useArtifactPanel.ts'),
  'utf8'
);
const VIEWER = readFileSync(join(__dirname, '../components/artifacts/ArtifactViewer.tsx'), 'utf8');
const BASE_CHAT = readFileSync(join(__dirname, '../components/BaseChat.tsx'), 'utf8');
const PREVIEW_HOSTS = [
  'BaseChat.tsx',
  'sessions/SessionHistoryView.tsx',
  'sessions/SharedSessionView.tsx',
].map((rel) => ({ rel, source: readFileSync(join(__dirname, '../components', rel), 'utf8') }));

/** The stylesheet with comments blanked (same length), so offsets survive. */
const CSS_CODE = CSS.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));

/** Every rule whose selector list, whitespace-collapsed, equals `selector`. */
function rulesFor(selector: string): { index: number; body: string }[] {
  const wanted = selector.replace(/\s+/g, ' ').trim();
  const found: { index: number; body: string }[] = [];
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  for (let match = pattern.exec(CSS_CODE); match; match = pattern.exec(CSS_CODE)) {
    const selectors = match[1].replace(/\s+/g, ' ').trim();
    if (selectors === wanted) found.push({ index: match.index, body: match[2] });
  }
  return found;
}

function onlyRule(selector: string): { index: number; body: string } {
  const found = rulesFor(selector);
  if (found.length !== 1)
    throw new Error(`expected exactly one rule for ${selector}, got ${found.length}`);
  return found[0];
}

function property(body: string, name: string): string | null {
  const match = body.match(new RegExp(`(?:^|;|\\s)${name}:\\s*([^;]+);`));
  return match ? match[1].replace(/\s+/g, ' ').trim() : null;
}

/** Brace depth at an offset: 0 means a top-level, unlayered rule. */
function depthAt(index: number): number {
  let depth = 0;
  for (let i = 0; i < index; i += 1) {
    if (CSS_CODE[i] === '{') depth += 1;
    else if (CSS_CODE[i] === '}') depth -= 1;
  }
  return depth;
}

const SIDE = "[data-preview-split][data-preview-layout='side']";
const STACK = "[data-preview-split][data-preview-layout='stack']";

describe('rung 2 — the preview split in main.css agrees with the ladder', () => {
  it('is one grid that clips what the flattened body used to clip', () => {
    const grid = onlyRule('[data-preview-split][data-preview-layout]');
    expect(property(grid.body, 'display')).toBe('grid');
    // `clip`, never `hidden`: a hidden box is a scroll container, and the tab
    // strip's scrollIntoView scrolled the whole split 20px sideways.
    expect(property(grid.body, 'overflow')).toBe('clip');
    const flattened = onlyRule(
      "[data-preview-split][data-preview-layout] > [data-preview-area='column'], [data-preview-split][data-preview-layout] > [data-preview-area='column'] > [data-preview-area='body']"
    );
    expect(property(flattened.body, 'display')).toBe('contents');
  });

  it('pins the ladder’s widths', () => {
    expect(PREVIEW_MIN_WIDTH).toBe(360);
    expect(READABLE_CHAT_WIDTH).toBe(440);
    expect(PREVIEW_SIDE_WIDTH).toBe(800);
    expect(PREVIEW_SIDE_WIDTH).toBe(PREVIEW_MIN_WIDTH + READABLE_CHAT_WIDTH);
    expect(PREVIEW_PREFERRED_CHAT_WIDTH).toBe(640);
    expect(PREVIEW_MAX_WIDTH).toBe(920);
    expect(PREVIEW_DEFAULT_WIDTH_RATIO).toBe(0.48);
  });

  it('pins the ladder’s heights to the tokens they mirror', () => {
    expect(declaration('dock-height')).toBe(`${PREVIEW_STACK_STRIP_HEIGHT}px`);
    expect(declaration('chrome-height')).toBe(`${PREVIEW_SIDE_STRIP_HEIGHT}px`);
    expect(PREVIEW_STACK_STRIP_HEIGHT).toBe(36);
    expect(PREVIEW_SIDE_STRIP_HEIGHT).toBe(44);
    expect(PREVIEW_STACK_MIN_HEIGHT).toBe(200);
    expect(PREVIEW_TRANSCRIPT_MIN_HEIGHT).toBe(146);
    expect(PREVIEW_STACK_RATIO).toBe(0.5);
  });

  it('seats the side column’s conversation at exactly READABLE_CHAT_WIDTH', () => {
    const side = onlyRule(SIDE);
    expect(property(side.body, 'grid-template-columns')).toBe(
      `minmax(${READABLE_CHAT_WIDTH}px, 1fr) var(--preview-panel-width)`
    );
    expect(property(side.body, 'grid-template-areas')).toBe(
      "'header preview' 'subheader preview' 'transcript preview' 'composer preview'"
    );
  });

  it('stacks header, sheet, transcript, composer — the composer on the bottom edge', () => {
    const stack = onlyRule(STACK);
    expect(property(stack.body, 'grid-template-columns')).toBe('minmax(0, 1fr)');
    expect(property(stack.body, 'grid-template-rows')).toBe(
      'auto auto minmax(var(--dock-height), var(--preview-stack-height)) 1fr auto'
    );
    expect(property(stack.body, 'grid-template-areas')).toBe(
      "'header' 'subheader' 'preview' 'transcript' 'composer'"
    );
  });

  it('holds the transcript’s floor at PREVIEW_TRANSCRIPT_MIN_HEIGHT below the 8px edge', () => {
    expect(PREVIEW_STACK_EDGE_HEIGHT).toBe(8);
    const transcript = onlyRule(
      `${STACK}:not([data-preview-measuring]) [data-preview-area='transcript']`
    );
    expect(property(transcript.body, 'padding-top')).toBe(`${PREVIEW_STACK_EDGE_HEIGHT}px`);
    expect(property(transcript.body, 'min-height')).toBe(
      `calc(${PREVIEW_TRANSCRIPT_MIN_HEIGHT}px + ${PREVIEW_STACK_EDGE_HEIGHT}px)`
    );
    // Placed explicitly in BOTH layouts: auto-placed beside the resize edge, a
    // replay's column was pushed into an implicit second column and its sheet
    // collapsed to 0px wide.
    for (const layout of [SIDE, STACK]) {
      const placed = onlyRule(`${layout} > [data-preview-area='conversation']`);
      expect(property(placed.body, 'grid-column'), layout).toBe('1');
    }
    const replay = onlyRule(
      `${STACK}:not([data-preview-measuring]) > [data-preview-area='conversation']`
    );
    expect(property(replay.body, 'padding-top')).toBe(`${PREVIEW_STACK_EDGE_HEIGHT}px`);
    expect(property(replay.body, 'min-height')).toBe('var(--preview-chat-floor)');
  });

  it('gives a stacked sheet the dock strip and a real bottom edge', () => {
    const strip = onlyRule(`${STACK} > [data-testid='artifact-viewer'] > .br-tabstrip`);
    expect(property(strip.body, 'height')).toBe('var(--dock-height)');
    expect(property(strip.body, 'background')).toBe('var(--background-default)');
    const sheet = onlyRule(`${STACK} > [data-testid='artifact-viewer']`);
    expect(property(sheet.body, 'border-bottom')).toBe('1px solid var(--border-default)');
  });

  /**
   * The edge is the panel's SIBLING, placed on the seam: the transcript's top 8px
   * under a sheet, the panel's left 8px beside it. Inside the panel it could only
   * have lain over the preview's content (the panel clips its own paint).
   */
  it('places the resize edge on the seam, 8px, covering neither side’s content', () => {
    const stack = onlyRule(`${STACK} > .br-preview-resize-handle`);
    expect(property(stack.body, 'grid-area')).toBe('transcript');
    expect(property(stack.body, 'align-self')).toBe('start');
    expect(property(stack.body, 'height')).toBe(`${PREVIEW_STACK_EDGE_HEIGHT}px`);
    expect(property(stack.body, 'cursor')).toBe('row-resize');
    const side = onlyRule(`${SIDE} > .br-preview-resize-handle`);
    expect(property(side.body, 'grid-area')).toBe('preview');
    expect(property(side.body, 'justify-self')).toBe('start');
    expect(property(side.body, 'width')).toBe('8px');
    expect(property(side.body, 'cursor')).toBe('col-resize');
    const hover = onlyRule(
      '[data-preview-split][data-preview-layout] > .br-preview-resize-handle:hover::after'
    );
    expect(property(hover.body, 'background')).toBe('var(--border-strong)');
    expect(onlyRule(`${STACK} > .br-preview-resize-handle`).index).toBeGreaterThan(
      onlyRule(`${SIDE} > .br-preview-resize-handle`).index
    );
  });

  it('declares the stack rules AFTER the side rules they override', () => {
    expect(onlyRule(STACK).index).toBeGreaterThan(onlyRule(SIDE).index);
    expect(onlyRule(`${STACK} > [data-preview-area='conversation']`).index).toBeGreaterThan(
      onlyRule(`${SIDE} > [data-preview-area='conversation']`).index
    );
    // The measuring template overrides the stack template at equal-or-higher
    // specificity, so it must come after it as well.
    expect(onlyRule(`${STACK}[data-preview-measuring]`).index).toBeGreaterThan(
      onlyRule(STACK).index
    );
  });

  it('is unlayered, so it beats the utilities on the same elements', () => {
    for (const selector of [
      '[data-preview-split][data-preview-layout]',
      SIDE,
      STACK,
      `${STACK} > [data-testid='artifact-viewer']`,
      `${STACK}[data-preview-measuring] > [data-testid='artifact-viewer']`,
      '.br-preview-measure',
    ]) {
      expect(depthAt(onlyRule(selector).index), selector).toBe(0);
    }
  });

  /**
   * The seam is decided in JS (`previewPanelMode`), because a `@container`
   * condition cannot read a custom property and the split box's width is what
   * both the grid and the ladder must agree on. A container or media query that
   * crept into these rules would be a SECOND seam, free to drift from 800.
   */
  it('has no container or media condition of its own', () => {
    const start = CSS_CODE.indexOf('[data-preview-split][data-preview-layout] {');
    const end = CSS_CODE.indexOf('body.biorouter-window-resizing .biorouter-sidebar-inset-depth', start);
    expect(start).toBeGreaterThan(0);
    expect(end).toBeGreaterThan(start);
    const block = CSS_CODE.slice(start, end);
    expect(block).not.toMatch(/@container|@media/);
    for (const condition of block.match(/@(?:container|media)[^{]*/g) ?? []) {
      expect(condition).toContain(`${PREVIEW_SIDE_WIDTH}px`);
    }
  });

  it('transitions nothing geometric (the motion pass owns motion)', () => {
    const start = CSS_CODE.indexOf('[data-preview-split][data-preview-layout] {');
    const end = CSS_CODE.indexOf('body.biorouter-window-resizing .biorouter-sidebar-inset-depth', start);
    expect(CSS_CODE.slice(start, end)).not.toMatch(/transition|animation/);
  });

  it('holds a fresh sheet back with no room and no pointer events while it measures', () => {
    const template = onlyRule(`${STACK}[data-preview-measuring]`);
    expect(property(template.body, 'grid-template-rows')).toBe('auto auto 0px 1fr auto');
    const sheet = onlyRule(`${STACK}[data-preview-measuring] > [data-testid='artifact-viewer']`);
    expect(property(sheet.body, 'opacity')).toBe('0');
    expect(property(sheet.body, 'pointer-events')).toBe('none');
    expect(property(sheet.body, 'height')).toBe('var(--preview-provisional-height)');
  });
});

describe('rung 2 — the panel stays mounted across a crossing', () => {
  it.each(PREVIEW_HOSTS)('$rel renders exactly one ArtifactViewer, unkeyed', ({ source }) => {
    const code = codeWithoutComments(source);
    const tags = code.match(/<ArtifactViewer\b[^>]*>/g) ?? [];
    expect(tags).toHaveLength(1);
    expect(tags[0]).not.toMatch(/\bkey=/);
    expect(code).toMatch(/\{\.\.\.artifactPanel\.splitPaneProps\}/);
  });

  it('BaseChat marks every piece the grid places, and flattens rather than re-parents', () => {
    const code = codeWithoutComments(BASE_CHAT);
    for (const area of ['column', 'body', 'header', 'subheader', 'composer']) {
      expect(code.match(new RegExp(`data-preview-area="${area}"`, 'g')) ?? [], area).toHaveLength(
        1
      );
    }
    // The clean conversation and the transcript: one or the other is rendered.
    expect(code.match(/data-preview-area="transcript"/g) ?? []).toHaveLength(2);
    expect(code.match(/data-preview-transcript=""/g) ?? []).toHaveLength(2);
  });

  /**
   * Each host marks the box rung 2 measures as its transcript, or the
   * conversation's chrome reads as nothing and a replay's page header is not
   * counted in its floor. SessionHistoryView renders its OWN transcript
   * component rather than SessionViewComponents', which is how its marker was
   * missed once.
   */
  it.each([
    ['BaseChat.tsx', 2],
    ['sessions/SessionHistoryView.tsx', 1],
    ['sessions/SessionViewComponents.tsx', 1],
  ])('%s marks its transcript for measurement', (rel, count) => {
    const source = readFileSync(join(__dirname, '../components', rel), 'utf8');
    expect(codeWithoutComments(source).match(/data-preview-transcript=""/g) ?? []).toHaveLength(
      count
    );
  });

  it('hands the viewer no layout-dependent class or style', () => {
    const viewerProps = PANEL_HOOK.slice(PANEL_HOOK.indexOf('viewerProps: {'));
    expect(viewerProps).not.toMatch(/\bclassName:|\bstyle:/);
  });

  it('never renders a different element for the other layout inside the panel', () => {
    const code = codeWithoutComments(VIEWER);
    expect(code).not.toMatch(/layout\s*===\s*'(?:stack|side)'\s*\?\s*\(?\s*</);
    expect(code).not.toMatch(/layout\s*===\s*'(?:stack|side)'\s*&&\s*\(?\s*</);
    expect(code).toContain("aria-orientation={layout === 'stack' ? 'horizontal' : 'vertical'}");
  });

  it('keeps the scroll anchor scoped to the live chat', () => {
    expect(codeWithoutComments(BASE_CHAT)).toContain(
      'anchorBottomOnResize={artifactPanel.isStacked}'
    );
    for (const { rel, source } of PREVIEW_HOSTS.slice(1)) {
      expect(source, rel).not.toContain('anchorBottomOnResize');
    }
  });

  it('animates the existing preview body rather than the grid box', () => {
    expect(VIEWER).toContain('usePreviewMotion(previewBodyRef');
    expect(VIEWER).not.toContain("'transition-[opacity,translate,transform]'");
  });
});

/**
 * THE PREVIEW'S TEXT MEASURE: text in the panel reads at the transcript's own
 * 760px column, and nothing that needs width is held to it.
 */
describe('the preview text measure is the chat measure', () => {
  it('caps the content at 760px with responsive gutters outside the text measure', () => {
    const rule = onlyRule('.br-preview-measure');
    expect(property(rule.body, 'max-width')).toBe('var(--measure-chat)');
    expect(property(rule.body, 'box-sizing')).toBe('content-box');
    expect(property(rule.body, 'margin-inline')).toBe('auto');
    expect(property(rule.body, 'padding-inline')).toBe('var(--paper-gutter)');
  });

  it('keeps intrinsic sizing on extracted prose, table and code components', () => {
    const prose = readFileSync(
      join(__dirname, '../components/artifacts/MarkdownDocument.tsx'),
      'utf8'
    );
    const table = readFileSync(
      join(__dirname, '../components/artifacts/DelimitedTable.tsx'),
      'utf8'
    );
    expect(prose).toContain('data-preview-intrinsic=""');
    expect(table).toContain('data-preview-intrinsic=""');
    expect(table).toContain('data-preview-scroller=""');
    expect(VIEWER).toContain('data-preview-intrinsic="code"');
    expect(VIEWER).toContain('data-preview-scroller=""');
  });

  it('aligns the status strip to the reading column while preserving its full-width rule', () => {
    const rule = onlyRule('.br-preview-measure-strip');
    expect(property(rule.body, 'padding-inline')).toBe(
      'max(14px, calc((100% - var(--measure-chat)) / 2))'
    );
    expect(depthAt(rule.index)).toBe(0);
    expect(codeWithoutComments(VIEWER).match(/br-preview-measure-strip/g) ?? []).toHaveLength(1);
  });
});
