import type { CSSProperties, HTMLAttributes } from 'react';
import glyphUrl from '../../images/glyph.svg';

export function BioRouter({ className = '', style, ...props }: HTMLAttributes<HTMLSpanElement>) {
  // The url() MUST be quoted. Vite inlines glyph.svg as a `data:` URI whose
  // markup uses single quotes and parentheses (`version='1.0'`,
  // `transform='rotate(-90 434.5 177.25)'`). An UNQUOTED url() token may not
  // contain `(`, `'` or whitespace — the tokenizer produces a bad-url-token,
  // which invalidates the whole declaration, so the browser dropped mask-image
  // on the floor. The mark paints `currentColor` and relies on the mask to cut
  // its shape, so it rendered as a SOLID BLOCK of the accent colour — a
  // failure that looks deliberate, which is how it survived.
  //
  // This is also why the old `--biorouter-glyph` indirection failed: a custom
  // property carrying a bad-url-token is itself invalid, so it never reached
  // the style attribute at all and `var(--biorouter-glyph)` resolved to
  // nothing. Quoting fixes both; the indirection bought nothing and is gone.
  const maskImage = `url("${glyphUrl}")`;
  const markStyle = {
    ...style,
    display: 'block',
    backgroundColor: 'currentColor',
    WebkitMaskImage: maskImage,
    maskImage,
    WebkitMaskPosition: 'center',
    maskPosition: 'center',
    WebkitMaskRepeat: 'no-repeat',
    maskRepeat: 'no-repeat',
    WebkitMaskSize: 'contain',
    maskSize: 'contain',
  } as CSSProperties;

  return (
    <span
      role="img"
      aria-label="Biorouter logo"
      className={className}
      {...props}
      style={markStyle}
    />
  );
}
