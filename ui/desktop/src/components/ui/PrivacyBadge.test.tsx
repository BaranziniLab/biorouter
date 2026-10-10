import { readFileSync } from 'node:fs';
import path from 'node:path';
import type { ReactElement } from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PrivacyBadge } from './PrivacyBadge';
import type { SessionClassification } from '../../api';

/**
 * The badge reads the master switch itself (issue #56, DR-15) rather than
 * making nine call sites remember a prop. Mounting a real `ConfigProvider`
 * here would drag in the daemon client, the provider list and the extension
 * sync; the hook is the seam, so the hook is what is stubbed.
 */
const configMocks = vi.hoisted(() => ({ enforced: true }));
vi.mock('../ConfigContext', () => ({
  usePrivacyTiersEnabled: () => configMocks.enforced,
}));

/** What the daemon currently says, for the two tests that care. */
const withPrivacyTiers = (enforced: boolean) => {
  configMocks.enforced = enforced;
};

afterEach(cleanup);
// Every other test in this file was written against an enforcing machine, which
// is the shipped default; pinning it here keeps the two tests that move it from
// leaking into the rest.
beforeEach(() => withPrivacyTiers(true));

/** vitest runs with `ui/desktop` as its root — same idiom as MentionPopover.test.tsx. */
const read = (...p: string[]) => readFileSync(path.join(process.cwd(), ...p), 'utf8');

/**
 * Every custom property `main.css` declares, anywhere in the file.
 *
 * This is the join `check-contrast.mjs` cannot make on its own: it reads
 * `main.css` and never opens this component, so a label written the way the
 * design's §14.1 specified it — an arbitrary value naming a token that exists
 * in no theme — passes every one of its assertions, raises no error, and
 * silently inherits whatever colour it lands on. That spelling is deliberately
 * not repeated here: `src/components` is grepped for it and owes zero hits, and
 * a comment is not a use. It is in the commit that added these assertions.
 */
const DECLARED_TOKENS = new Set(
  [...read('src', 'styles', 'main.css').matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1])
);

/**
 * The tokens the guard's four `privacy …` assertions actually measure.
 *
 * Hard-coding the expected pairs in the guard proves the *theme* holds them at
 * ratio; it does not prove this component paints them. Deriving the set from
 * the guard's own source and checking the rendered classes against it closes
 * that loop in the one direction that was open: a badge repainted in a token
 * nobody measured now fails here, not in a screenshot review six months on.
 */
const PRIVACY_ASSERTION_LINES = read('scripts', 'check-contrast.mjs')
  .split('\n')
  // `assert(` as well as the label, so prose cannot widen the allowed set. The
  // block's own explanatory comment says "privacy pills" and would otherwise be
  // read as an assertion — harmless today, but a comment that happened to quote
  // a token would have silently permitted it.
  .filter((l) => l.includes('privacy ') && l.trimStart().startsWith('assert('));
const MEASURED_TOKENS = new Set(
  PRIVACY_ASSERTION_LINES.flatMap((l) => [...l.matchAll(/'(--[\w-]+)'/g)].map((m) => m[1]))
);

/**
 * The colour tokens a rendered `className` names.
 *
 * Tailwind colour utilities here are `<prefix>-<token>`, where `<token>` is the
 * custom property minus its `--` (`bg-background-muted` → `--background-muted`,
 * `bg-text-default` → `--text-default`). Arbitrary values name their token
 * inside a `var()`, which is scanned separately — that form is exactly how the
 * design's non-existent token would have reached the DOM. `text-[11px]` is a
 * font size, not a colour, and is correctly ignored: its remainder names no
 * colour family.
 */
const COLOUR_FAMILY = /^(?:text|background|border|sidebar|ring)(?:-|$)/;
function colourTokensOf(className: string): string[] {
  const out: string[] = [];
  for (const cls of className.split(/\s+/)) {
    const m = /^(?:bg|text|border|ring|fill|stroke)-(.+)$/.exec(cls);
    if (!m) continue;
    const token = m[1].replace(/\/\d+$/, '');
    if (COLOUR_FAMILY.test(token)) out.push(`--${token}`);
  }
  for (const m of className.matchAll(/var\((--[\w-]+)\)/g)) out.push(m[1]);
  return [...new Set(out)];
}

const badgeOf = (ui: ReactElement) =>
  render(ui).container.querySelector('[data-testid="privacy-badge"]');

/** Every state that renders something. Public + dense renders nothing by design. */
const VISIBLE_STATES: { name: string; tier: SessionClassification; dense: boolean }[] = [
  { name: 'private pill', tier: 'private', dense: false },
  { name: 'public pill', tier: 'public', dense: false },
  { name: 'private dense mark', tier: 'private', dense: true },
];

describe('PrivacyBadge', () => {
  it('renders through the app badge primitive and adds no geometry of its own', () => {
    const { container } = render(<PrivacyBadge tier="private" />);
    const el = container.querySelector('[data-testid="privacy-badge"]')!;
    expect(el).toHaveTextContent('Private');
    expect(el.className).toContain('rounded-inner'); // from Badge, not hand-rolled
    expect(el.querySelector('svg')).not.toBeNull(); // never colour alone: shape + glyph + word
  });

  it('renders nothing in dense mode for a public session', () => {
    const { queryByTestId } = render(<PrivacyBadge tier="public" dense />);
    // No padlock means public. Deliberately not an OPEN padlock: a mark on
    // every public row would put the two tiers at the same visual weight and
    // train people past both.
    expect(queryByTestId('privacy-badge')).toBeNull();
  });

  // ── The styling itself, not just the text ──
  //
  // The two assertions above pass unchanged against a Public pill painted the
  // design's way — `border-border-subtle bg-transparent text-text-subtle` —
  // which measures 1.00:1 against its own ground in parchment:dark (the
  // hairline and the surface are literally the same colour) and drops the
  // label under AA on three of six scopes' hover rows. So does the whole
  // contrast guard, which never opens this file. These do not.

  it('paints Private as the marked state: the fill, the strong ink, the glyph, the word', () => {
    const el = badgeOf(<PrivacyBadge tier="private" />)!;
    expect(el.getAttribute('data-privacy')).toBe('private');
    expect(el).toHaveTextContent('Private');
    expect(el.className).toContain('bg-background-muted');
    expect(el.className).toContain('text-text-default');
    expect(el.className).not.toContain('text-text-muted');
    expect(el.querySelector('svg')).not.toBeNull();
  });

  it('paints Public as the quiet state: same fill, quiet ink, no glyph, no outline', () => {
    const el = badgeOf(<PrivacyBadge tier="public" />)!;
    expect(el.getAttribute('data-privacy')).toBe('public');
    expect(el).toHaveTextContent('Public');
    expect(el.className).toContain('bg-background-muted');
    expect(el.className).toContain('text-text-muted');
    expect(el.className).not.toContain('text-text-default');
    // Quiet, not absent: a badge on everything trains people to stop seeing
    // badges, but Public still names itself in words.
    expect(el.querySelector('svg')).toBeNull();
    // And it is a FILL, never an outline. No neutral resting border token in
    // this system clears 1.6:1 against either badge ground, so the design's
    // hairline pill would have shipped invisible.
    const classes = el.className.split(/\s+/);
    expect(classes.filter((c) => c.startsWith('border'))).toEqual([]);
    expect(classes).not.toContain('bg-transparent');
  });

  it('keeps the two tiers visually distinct, so the badge is not decoration', () => {
    const priv = badgeOf(<PrivacyBadge tier="private" />)!;
    const pub = badgeOf(<PrivacyBadge tier="public" />)!;
    expect(priv.className).not.toEqual(pub.className);
    expect(priv.textContent).not.toEqual(pub.textContent);
  });

  it('rides Badge for the pill geometry in both tiers, restating none of it', () => {
    const source = read('src', 'components', 'ui', 'PrivacyBadge.tsx');

    // DERIVED from badge.tsx's own declaration, not transcribed from it. This
    // list used to be five literals, and the Astryx migration renamed two of
    // them (`rounded-sm` → `rounded-inner`, `text-[11px]` → `text-chip`). Both
    // copies then failed — the containment check AND the "must not restate"
    // regex — for a token rename rather than for a regression, which is the
    // most expensive kind of red: it looks like the thing under test broke.
    // Reading the primitive means a future rename moves both sides at once,
    // while the assertion still fails if PrivacyBadge stops inheriting the
    // geometry or starts writing it down a second time.
    const badgeGeometry = (() => {
      const badgeSource = read('src', 'components', 'ui', 'badge.tsx');
      const base = badgeSource.match(/cn\(\s*'([^']+)'/);
      expect(
        base,
        'badge.tsx no longer opens its cn() with a base class string — this ' +
          'derivation is broken and every assertion below would pass vacuously'
      ).not.toBeNull();
      return base![1].split(/\s+/).filter(Boolean);
    })();
    // The control: an empty list would make the loop below assert nothing.
    expect(badgeGeometry.length).toBeGreaterThan(3);

    for (const el of [
      badgeOf(<PrivacyBadge tier="private" />)!,
      badgeOf(<PrivacyBadge tier="public" />)!,
    ]) {
      for (const geometry of badgeGeometry) {
        expect(el.className, `the pill lost Badge's ${geometry}`).toContain(geometry);
      }
    }

    // …and none of it is written down a second time here, which is the drift
    // badge.tsx's own doc-comment exists to prevent. The dense branch's own
    // classes are excluded by construction rather than by exception: it uses
    // `inline-block` + `shrink-0`, neither of which is in badge.tsx's base list
    // — and that is a live constraint, not an accident. The obvious spelling of
    // that branch is the one `AffiliationBadge` uses (a flex box that centres
    // its glyph), and it would trip this assertion on two counts, because those
    // are Badge's words. The dense mark shrink-wraps a `block` glyph instead,
    // which needs no alignment of its own.
    expect(source).toMatch(/from '\.\/badge'/);
    for (const geometry of badgeGeometry) {
      expect(
        source,
        `PrivacyBadge restates Badge's ${geometry} instead of inheriting it`
      ).not.toContain(geometry);
    }
  });

  // ── The tokens, checked against the stylesheet and against the guard ──

  it('names only colour tokens main.css actually declares', () => {
    for (const { name, tier, dense } of VISIBLE_STATES) {
      const tokens = colourTokensOf(badgeOf(<PrivacyBadge tier={tier} dense={dense} />)!.className);
      expect(tokens.length, `${name} names no colour token at all`).toBeGreaterThan(0);
      for (const t of tokens) {
        expect(DECLARED_TOKENS.has(t), `${name} names ${t}, which main.css never declares`).toBe(
          true
        );
      }
    }
  });

  it('names only colour tokens check-contrast.mjs measures for this badge', () => {
    expect(
      PRIVACY_ASSERTION_LINES.length,
      'the guard no longer carries assertions labelled `privacy …`'
    ).toBeGreaterThanOrEqual(4);
    for (const { name, tier, dense } of VISIBLE_STATES) {
      for (const t of colourTokensOf(
        badgeOf(<PrivacyBadge tier={tier} dense={dense} />)!.className
      )) {
        expect(
          MEASURED_TOKENS.has(t),
          `${name} paints ${t}, which no privacy assertion in check-contrast.mjs measures`
        ).toBe(true);
      }
    }
  });

  // ── The dense mark ──
  //
  // ⚠ **It was a filled dot and is now the padlock**, because the app marked one
  // fact — the issue-#56 private tier — with three unrelated figures: a
  // padlocked speech bubble on a private chat (`chatKind.ts`), a shield on a
  // private extension (the pill, above), and this anonymous dot on a private
  // model. The dot is the one that carried no figure at all, so nothing
  // connected it to the other two. The assertions below therefore moved from
  // "is a filled circle" to "is the padlock, at the same size and ink the pill
  // uses" — they pin the SAME properties (a real box, a shrink guard, an
  // accessible name), against the mark the app actually draws now.

  it('gives the dense mark a box, so it cannot render at zero size', () => {
    const badge = badgeOf(<PrivacyBadge tier="private" dense />)!;
    const classes = badge.className.split(/\s+/);
    // `width` and `height` do not apply to a non-replaced INLINE element, and a
    // `<span>` is inline by default — so a wrapper left at its default display
    // gets its geometry from line-height rather than from its child. That is
    // this component's own failure mode (an indicator that is silently
    // mis-sized, passing every screenshot review) handed to its callers.
    expect(classes).toContain('inline-block');
    // Dense surfaces are tight by definition — that is why they are dense. A
    // flex child with no shrink guard is the first thing squeezed to nothing,
    // and these are exactly the rows the mark was made for.
    expect(classes).toContain('shrink-0');
    // The box now lives on the glyph, so that is where the size is asserted.
    // `block` as well as the size: an inline svg sits on the text baseline and
    // reserves a descender's worth of space below itself, which is how a 12px
    // mark silently grows the row it is dropped into.
    const glyph = badge.querySelector('svg')!;
    expect(glyph).not.toBeNull();
    const glyphClasses = (glyph.getAttribute('class') ?? '').split(/\s+/);
    expect(glyphClasses).toContain('block');
    expect(glyphClasses).toContain('h-3');
    expect(glyphClasses).toContain('w-3');
  });

  it('marks a dense Private row with a padlock that carries its own name', () => {
    const mark = badgeOf(<PrivacyBadge tier="private" dense />)!;
    expect(mark.getAttribute('data-privacy')).toBe('private');
    expect(mark.textContent).toBe('');
    expect(mark.className).toContain('text-text-default');
    // Shape alone carries the meaning at this size, so the mark owes an
    // accessible name. A bare <span> maps to role `generic`, which takes none —
    // `title` by itself would leave it both unannounced and (on an <svg>, where
    // the tooltip mechanism is a `<title>` child element) untooltipped.
    expect(mark.getAttribute('role')).toBe('img');
    expect(mark.getAttribute('aria-label')).toBe('Private chat');
    expect(mark.getAttribute('title')).toMatch(/^Private\. Biorouter only lets a private model/);
    // The glyph itself must stay decorative, or a screen reader announces the
    // padlock twice — once from the span's label and once from the svg.
    expect(mark.querySelector('svg')!.getAttribute('aria-hidden')).toBe('true');
  });

  /**
   * ⚠ **The consistency this change exists to create, asserted rather than
   * eyeballed.** The dense mark and the pill's glyph are the same figure, so a
   * later edit cannot quietly send one of them back to a dot or a shield while
   * the other stays a padlock — which is exactly how the three-figure state
   * arose in the first place.
   *
   * The class lucide stamps on the svg (`lucide-<kebab-name>`) is the only
   * identity a rendered icon has in jsdom; comparing the two rendered glyphs to
   * each other, rather than to a literal, means a deliberate move to some third
   * glyph still passes as long as BOTH forms move together.
   */
  it('draws the same Private glyph in the dense mark and the pill', () => {
    const denseGlyph = badgeOf(<PrivacyBadge tier="private" dense />)!.querySelector('svg')!;
    const denseIcon = (denseGlyph.getAttribute('class') ?? '')
      .split(/\s+/)
      .filter((c) => c.startsWith('lucide-'));
    cleanup();
    const pillGlyph = badgeOf(<PrivacyBadge tier="private" />)!.querySelector('svg')!;
    const pillIcon = (pillGlyph.getAttribute('class') ?? '')
      .split(/\s+/)
      .filter((c) => c.startsWith('lucide-'));

    expect(
      denseIcon.length,
      'lucide no longer stamps an icon class — this assertion is vacuous'
    ).toBeGreaterThan(0);
    expect(denseIcon).toEqual(pillIcon);
    // And it is the padlock, the same figure as the lock badge every private
    // chat glyph wears (`withPrivateBadge`). Stated once, here, so the
    // vocabulary has one written-down anchor.
    expect(denseIcon).toContain('lucide-lock');
  });

  it('passes a caller className through in both dense and full mode', () => {
    expect(badgeOf(<PrivacyBadge tier="public" className="ml-2" />)!.className).toContain('ml-2');
    expect(badgeOf(<PrivacyBadge tier="private" dense className="ml-2" />)!.className).toContain(
      'ml-2'
    );
  });

  // ── DR-15: the badge does not hide itself when nothing enforces it ──

  it('badges stay visible while enforcement is off, and say so', () => {
    render(<PrivacyBadge tier="private" enforcementOff />);
    // Still there. Hiding it is the tidy-looking answer and the worst one: it
    // makes an UNPROTECTED machine indistinguishable from a machine with no
    // private material on it, at exactly the moment the distinction matters.
    expect(screen.getByText(/Private/)).toBeInTheDocument();
    // And it no longer states something untrue. A pill reading plain "Private"
    // while nothing enforces it is worse than no badge, because the user acts
    // on it.
    expect(screen.getByText(/enforcement off/i)).toBeInTheDocument();
  });

  it('says it on the Public pill too, and on the dense mark where there is no room for words', () => {
    // Public is a tier claim as much as Private is; with enforcement off it is
    // no more true than the other one.
    const pill = badgeOf(<PrivacyBadge tier="public" enforcementOff />)!;
    expect(pill.getAttribute('data-enforcement')).toBe('off');
    expect(pill.textContent).toMatch(/enforcement off/i);

    // The dense mark has no words, so the accessible name carries it.
    const mark = badgeOf(<PrivacyBadge tier="private" dense enforcementOff />)!;
    expect(mark.getAttribute('data-enforcement')).toBe('off');
    expect(mark.getAttribute('aria-label')).toBe('Private chat, enforcement off');
  });

  it('says nothing about enforcement when enforcement is on', () => {
    // The default must not be the loud state, or the suffix stops meaning
    // anything on the surface where it matters.
    const pill = badgeOf(<PrivacyBadge tier="private" />)!;
    expect(pill.getAttribute('data-enforcement')).toBe('on');
    expect(pill.textContent).not.toMatch(/enforcement/i);
    expect(badgeOf(<PrivacyBadge tier="private" dense />)!.getAttribute('data-enforcement')).toBe(
      'on'
    );
  });

  /**
   * The finding this closes: `enforcementOff` shipped once with NO production
   * consumer. All nine call sites render the badge with no such prop, so the
   * presentation above was reachable only from this file — the person the
   * suffix is for, the one who turned the feature off in March, would never
   * have seen it on any surface in the app.
   *
   * So the badge asks. These two assert the ASKING, not the presentation: a
   * badge with no prop at all must follow the daemon's value in both
   * directions.
   */
  it('with no prop at all, takes its answer from the daemon', () => {
    withPrivacyTiers(false);
    const off = badgeOf(<PrivacyBadge tier="private" />)!;
    expect(off.getAttribute('data-enforcement')).toBe('off');
    expect(off.textContent).toMatch(/enforcement off/i);

    cleanup();
    withPrivacyTiers(true);
    const on = badgeOf(<PrivacyBadge tier="private" />)!;
    expect(on.getAttribute('data-enforcement')).toBe('on');
    expect(on.textContent).not.toMatch(/enforcement/i);
  });

  it('an explicit prop still wins, in both directions', () => {
    // `??`, not `||`: a caller that means "enforcement is on, whatever the
    // daemon says" — a settings screen previewing both states — must not have
    // its `false` swallowed.
    withPrivacyTiers(false);
    expect(
      badgeOf(<PrivacyBadge tier="private" enforcementOff={false} />)!.getAttribute(
        'data-enforcement'
      )
    ).toBe('on');

    cleanup();
    withPrivacyTiers(true);
    expect(
      badgeOf(<PrivacyBadge tier="private" enforcementOff />)!.getAttribute('data-enforcement')
    ).toBe('off');
  });
});
