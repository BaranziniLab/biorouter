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
