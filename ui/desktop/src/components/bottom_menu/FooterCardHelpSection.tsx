import * as React from 'react';
import { InfoTip } from '../ui/info-tip';
import { FooterCardSection } from './FooterCard';

/**
 * A footer card section whose label carries an InfoTip: the label says what the section is, and
 * the sentences that explain it wait behind the glyph beside it (owner, 2026-11-13: one line up
 * front, the detail on hover or on click, never a block of muted prose under the reading). With no
 * `help` it is a plain `FooterCardSection`, so a section draws the glyph only when there is
 * something to explain.
 *
 * The label and the glyph sit side by side in a row (`data-slot="footer-card-head"`), and the
 * label's `<p>` holds only its words. Inside the `<p>`, the glyph's name and its hidden description
 * would join the name of a list the label names (`labelId`), and a section label's text would no
 * longer be its label alone.
 */
export function FooterCardHelpSection({
  label,
  labelId,
  about,
  help,
  children,
}: {
  label: string;
  /** The label's id, for a list in the section that is named by it. */
  labelId?: string;
  /** What the InfoTip is about: its name reads "About {about}". */
  about: string;
  /** The InfoTip's text, at most two sentences. Empty or absent, no glyph is drawn. */
  help?: string;
  children: React.ReactNode;
}) {
  if (!help) {
    return (
      <FooterCardSection label={label} labelId={labelId}>
        {children}
      </FooterCardSection>
    );
  }
  return (
    <section className="br-footer-card__section">
      {/* `data-slot` names the row for a test of the card's shape, which takes a label here or
          directly in its section as the same piece. */}
      <div className="flex min-w-0 items-center" data-slot="footer-card-head">
        <p id={labelId} className="br-footer-card__label">
          {label}
        </p>
        <InfoTip label={about} help={help} />
      </div>
      {children}
    </section>
  );
}
