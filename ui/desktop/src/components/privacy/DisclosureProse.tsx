import { Fragment, type ReactNode } from 'react';

/**
 * The served long-form disclosure, rendered (issue #56, DR-17 requirement 3).
 *
 * ⚠ **One renderer, because there is one copy.** The dialog and Settings →
 * Privacy both show `COPY_LONG`, and a second implementation of "split the
 * paragraphs, honour the emphasis" is how one surface ends up printing `**` at
 * the user while the other does not. This component contains no product prose of
 * its own — every word comes from `disclosureCopy.ts`, which got it from
 * `crates/biorouter/src/privacy/disclosure.rs` over the wire.
 *
 * ⚠ **The emphasis is load-bearing, not decoration.** `**…**` marks the two
 * clauses that say what is *not* protected. Ordering alone does not carry the
 * ruling: three paragraphs of identical weight let a skimmer take the middle one
 * — the flattering half, the three things Biorouter does stop — as the summary
 * and conclude the machine is opaque to a public model, which is the reading
 * DR-17 forbids. Stripping the markers would be as wrong as printing them.
 */
export function DisclosureProse({
  text,
  paragraphClassName,
}: {
  text: string;
  paragraphClassName?: string;
}) {
  return (
    <>
      {text.split('\n\n').map((paragraph) => (
        <p key={paragraph.slice(0, 48)} className={paragraphClassName}>
          {emphasise(paragraph)}
        </p>
      ))}
    </>
  );
}

/**
 * Odd segments between `**` markers are emphasised, even ones are plain.
 *
 * The Rust side asserts the markers are balanced, so the last segment is always
 * a plain one; an unbalanced marker would show as an unemphasised tail rather
 * than as a stray `**`.
 */
function emphasise(paragraph: string): ReactNode[] {
  return paragraph.split('**').map((segment, index) =>
    index % 2 === 1 ? (
      <strong key={`${index}-${segment.slice(0, 24)}`} className="font-semibold">
        {segment}
      </strong>
    ) : (
      <Fragment key={`${index}-${segment.slice(0, 24)}`}>{segment}</Fragment>
    )
  );
}
