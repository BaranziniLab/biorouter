import { useEffect, useLayoutEffect, useRef, useState, type SVGProps } from 'react';
import { useResolvedTheme } from '../../contexts/ThemeContext';

/**
 * The BioRouter WORDMARK — the horizontal brand logo.
 *
 * "Bio" in UCSF navy, "Router" in coral, over a short two-tone underline that
 * sits BETWEEN the o and the R (not a rule under the whole word) and splits
 * navy/coral at the o|R seam. On a dark surface the navy would vanish, so the
 * navy role (the "Bio" letters + the navy half of the bar) becomes UCSF teal.
 *
 * The approved design (logo-wordmark-studio.html, D-39):
 *   weight 600 · letter-spacing 0 · underline gap 2% of cap height ·
 *   underline thickness 10% of cap height · underline width 100% (the "oR" pair)
 *
 * Geometry is MEASURED at runtime from the rendered text (getBBox +
 * getStartPositionOfChar / getEndPositionOfChar), exactly as the studio does,
 * rather than baked as fixed path coordinates. This keeps the underline correct
 * regardless of the font the platform resolves for the label and re-measures
 * once webfonts settle. The mark is set in the app's own sans (the same face as
 * every other label), so it needs no embedded font.
 */

/**
 * DELIBERATELY FIXED — the wordmark does NOT follow the theme family.
 *
 * These are module constants on purpose, and a reviewer who notices that the BR
 * *mark* reads its inks from the active family (BioRouterMark.tsx) will be
 * tempted to "fix" this the same way. Do not. The two are different things:
 *
 *   BioRouterMark  — the BR monogram, treated as an in-app graphic. Roche Limit
 *                    explicitly rebrands it (roche-limit-theme.md), so it is
 *                    family-aware and reads GENERATED_THEMES[family][mode].mark.
 *   BioRouterWordmark — the "BioRouter" brand lockup. It stays in the brand's
 *                    own colours whichever palette the app is wearing, exactly
 *                    as a logo does on a coloured page.
 *
 * The light/dark flip below is a DIFFERENT axis and is kept: navy is unreadable
 * on a dark ground, so the primary ink lifts to the brand teal. That is a
 * legibility rule, not a theming rule.
 *
 * Locked by BioRouterWordmark.test.tsx, which asserts these render identically
 * across every theme family.
 */
const NAVY = '#052049';
const CORAL = '#b85a32';
const TEAL = '#18a3ac';

// Approved ratios — see logo-wordmark-studio.html.
const WEIGHT = 600;
const GAP_K = 0.02; // letters -> underline, as a fraction of cap height
const THICK_K = 0.1; // underline thickness, as a fraction of cap height
const UL_WIDTH = 1.0; // 1.0 = underline exactly the "oR" pair
const WORK_SIZE = 100; // font-size in SVG user units; the viewBox scales it to `className`
const BIO_LEN = 3; // "Bio" is the first three characters

/**
 * Every drawn part of the lockup (the two words and both halves of the bar)
 * carries this class, and nothing else. The inks above are SVG `fill`
 * attributes, which Chromium does NOT force under `forced-colors: active`
 * (measured: both words and both bars kept their authored fills on a black
 * canvas, so the navy "Bio" read at about 1.3:1). `styles/main.css`
 * ("wordmark forced colours", WS-ICONS) takes the whole lockup to `CanvasText`
 * there: a person who asked for their own palette gets one legible ink, not two
 * brand colours that may sit on any ground they chose. A logo may be one ink.
 */
const INK_CLASS = 'br-wordmark-ink';

import { BRAND_SANS as SANS } from './brandFont';

type Geo = {
  vbX: number;
  vbY: number;
  vbW: number;
  vbH: number;
  barLeft: number;
  splitX: number;
  barRight: number;
  ulTop: number;
  thick: number;
};

interface BioRouterWordmarkProps extends Omit<SVGProps<SVGSVGElement>, 'ref' | 'title'> {
  /** Force the dark (teal) treatment; defaults to the resolved app theme. */
  dark?: boolean;
  title?: string;
}

export function BioRouterWordmark({
  className,
  style,
  dark: darkProp,
  // The accessible name mirrors the rendered wordmark on purpose: the tspans
  // below draw `Bio` and `Router` in two inks, so a sighted user reads
  // "BioRouter". Lowercasing this alone would give one control two names, and
  // recasing the glyphs is a brand call. Left as-is by BR-TEXT-2 (#83).
  title = 'BioRouter',
  ...rest
}: BioRouterWordmarkProps) {
  const textRef = useRef<SVGTextElement>(null);
  const [geo, setGeo] = useState<Geo | null>(null);
  const resolved = useResolvedTheme();
  const dark = darkProp ?? resolved === 'dark';
  const navy = dark ? TEAL : NAVY;

  const measure = () => {
    const t = textRef.current;
    // getBBox does not exist in jsdom (it throws, not returns 0), so under unit
    // tests the mark stays unmeasured — invisible, but its accessible name and
    // text nodes still render. Real browsers always have it.
    if (!t || typeof t.getBBox !== 'function') return;
    const bb = t.getBBox();
    if (bb.width === 0 || bb.height === 0) return;
    const cap = bb.height;
    const left = bb.x;

    // Anchor the short bar to the real glyph positions: the 'o' is the last
    // letter of "Bio" (index 2), the 'R' the first of "Router" (index 3).
    let seamX = left + bb.width * 0.33;
    let oStart = seamX - cap * 0.3;
    let rEnd = seamX + cap * 0.3;
    try {
      seamX = t.getStartPositionOfChar(BIO_LEN).x;
      oStart = t.getStartPositionOfChar(BIO_LEN - 1).x;
      rEnd = t.getEndPositionOfChar(BIO_LEN).x;
    } catch {
      // getStartPositionOfChar can throw in some engines; keep the fallbacks.
    }

    const pairCenter = (oStart + rEnd) / 2;
    const barW = (rEnd - oStart) * UL_WIDTH;
    const barLeft = pairCenter - barW / 2;
    const barRight = pairCenter + barW / 2;
    const splitX = Math.min(Math.max(seamX, barLeft), barRight);

    const gap = cap * GAP_K;
    const thick = cap * THICK_K;
    const ulTop = bb.y + bb.height + gap;
    const ulBottom = ulTop + thick;

    const pad = cap * 0.06;
    const next: Geo = {
      vbX: left - pad,
      vbY: bb.y - pad,
      vbW: bb.width + pad * 2,
      vbH: ulBottom - bb.y + pad * 2,
      barLeft,
      splitX,
      barRight,
      ulTop,
      thick,
    };
    // Bail if unchanged — measure() runs after every render, so returning the
    // previous object keeps React from re-rendering into an infinite loop once
    // the geometry has settled.
    setGeo((prev) => (prev && JSON.stringify(prev) === JSON.stringify(next) ? prev : next));
  };

  // Latest measure(), read through a ref so the mount-only font effect never
  // lists it as a dependency.
  const measureRef = useRef(measure);
  measureRef.current = measure;

  // Measure once on mount, and again once webfonts settle. NOT on every render:
  // getBBox returns sub-pixel-different values between calls, so measuring in a
  // dependency-less effect re-set state forever ("Maximum update depth
  // exceeded"). The geometry is in viewBox units, so it never needs a re-measure
  // on resize — the SVG scales itself.
  useLayoutEffect(() => {
    measureRef.current();
  }, []);
  useEffect(() => {
    if (!document.fonts?.ready) return;
    let alive = true;
    document.fonts.ready.then(() => {
      if (alive) measureRef.current();
    });
    return () => {
      alive = false;
    };
  }, []);

  const viewBox = geo
    ? `${geo.vbX.toFixed(2)} ${geo.vbY.toFixed(2)} ${geo.vbW.toFixed(2)} ${geo.vbH.toFixed(2)}`
    : `0 -${WORK_SIZE} ${WORK_SIZE * 5} ${WORK_SIZE * 1.4}`;

  return (
    <svg
      {...rest}
      className={className}
      style={style}
      viewBox={viewBox}
      role="img"
      aria-label={title}
      // Until the first measurement lands, keep it invisible rather than flash
      // an unpositioned bar.
      opacity={geo ? 1 : 0}
    >
      <title>{title}</title>
      <text
        ref={textRef}
        x={0}
        y={0}
        fontFamily={SANS}
        fontWeight={WEIGHT}
        fontSize={WORK_SIZE}
        letterSpacing="0"
      >
        <tspan className={INK_CLASS} fill={navy}>
          Bio
        </tspan>
        <tspan className={INK_CLASS} fill={CORAL}>
          Router
        </tspan>
      </text>
      {geo && (
        <>
          {/* The navy rect is the WHOLE bar and the coral one is drawn over its right half,
              not two rects meeting at splitX. Rects that meet at a fractional x leave a faint
              background-coloured hairline where their anti-aliased edges join (measured at 1x
              as a 3 to 7% dip in one column of solid pixels); it is invisible between two
              inks and shows once forced colours paint both halves one colour. Layered, the
              seam pixel is covered by the navy rect in full whatever the scale. */}
          <rect
            className={INK_CLASS}
            x={geo.barLeft}
            y={geo.ulTop}
            width={Math.max(0, geo.barRight - geo.barLeft)}
            height={geo.thick}
            fill={navy}
          />
          <rect
            className={INK_CLASS}
            x={geo.splitX}
            y={geo.ulTop}
            width={Math.max(0, geo.barRight - geo.splitX)}
            height={geo.thick}
            fill={CORAL}
          />
        </>
      )}
    </svg>
  );
}

export default BioRouterWordmark;
