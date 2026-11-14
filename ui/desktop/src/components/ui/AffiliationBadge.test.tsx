import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { AffiliationBadge } from './AffiliationBadge';
import {
  affiliationExplanation,
  affiliationPresentation,
  type ProviderAffiliation,
} from '../privacy/providerAffiliation';

afterEach(cleanup);

const ucsf: ProviderAffiliation = {
  kind: 'institutions',
  institutions: [{ id: 'ucsf', display_name: 'UCSF' }],
};
const local: ProviderAffiliation = { kind: 'local', institutions: [] };
const unstated: ProviderAffiliation = { kind: 'unstated', institutions: [] };

describe('AffiliationBadge', () => {
  /**
   * A public model has no affiliation, so the badge is not merely blank — it is
   * absent, and the surrounding row collapses. A blank chip would read as a
   * constraint on a model that has none.
   */
  it('renders nothing at all when there is no affiliation', () => {
    const { container } = render(<AffiliationBadge affiliation={null} />);
    expect(container).toBeEmptyDOMElement();
    cleanup();
    const undefinedCase = render(<AffiliationBadge affiliation={undefined} />);
    expect(undefinedCase.container).toBeEmptyDOMElement();
  });

  it('names the institution and marks the kind for a covered model', () => {
    render(<AffiliationBadge affiliation={ucsf} />);
    const badge = screen.getByTestId('affiliation-badge');
    expect(badge).toHaveAttribute('data-affiliation', 'institutions');
    expect(badge).toHaveTextContent('UCSF');
  });

  /**
   * ⚠ **The inversion, at the badge.** `local` must not render in the
   * institution's visual family: it is the MOST permissive affiliation, and a
   * landmark-marked chip beside `UCSF` invites "an institution, but narrower".
   * The three kinds are distinguishable by their `data-affiliation` marker AND
   * by carrying different glyph elements.
   */
  it('marks local as its own kind, distinct from an institution', () => {
    const { container: localMark } = render(<AffiliationBadge affiliation={local} />);
    const localBadge = screen.getByTestId('affiliation-badge');
    expect(localBadge).toHaveAttribute('data-affiliation', 'local');
    expect(localBadge).toHaveTextContent('On this machine');
    const localGlyph = localMark.querySelector('svg')?.getAttribute('class');
    cleanup();

    const { container: instMark } = render(<AffiliationBadge affiliation={ucsf} />);
    const instGlyph = instMark.querySelector('svg')?.getAttribute('class');

    expect(localGlyph).toBeTruthy();
    expect(instGlyph).toBeTruthy();
    expect(localGlyph).not.toEqual(instGlyph);
  });

  /**
   * ⚠ **No tone, weight or colour varies across the three kinds**, because any
   * variation is read as a ranking and the ranking people infer — "more
   * specific means more restricted" — is backwards. The chips must be the same
   * surface; only the glyph and the word differ.
   */
  it('gives the three affiliations the same chip surface, never a scale', () => {
    const classes = [local, ucsf, unstated].map((affiliation) => {
      cleanup();
      render(<AffiliationBadge affiliation={affiliation} />);
      return screen.getByTestId('affiliation-badge').getAttribute('class');
    });
    expect(new Set(classes).size).toBe(1);
  });

  /**
   * `unstated` is a fact, not an absence — it is the least-reaching private
   * model, and an empty-looking chip would show it as the least constrained.
   */
  it('states an unstated institution rather than rendering an empty chip', () => {
    render(<AffiliationBadge affiliation={unstated} />);
    const badge = screen.getByTestId('affiliation-badge');
    expect(badge).toHaveAttribute('data-affiliation', 'unstated');
    expect(badge.textContent?.trim()).toBe('No stated institution');
  });

  /**
   * The dense form is what the composer chip can fit. A glyph alone is silent to
   * a screen reader unless it is given a role that takes an accessible name —
   * the same trap `PrivacyBadge`'s dense dot documents.
   */
  it('keeps the dense glyph announced and per-kind', () => {
    render(<AffiliationBadge affiliation={ucsf} dense />);
    const badge = screen.getByTestId('affiliation-badge');
    expect(badge).toHaveAttribute('role', 'img');
    expect(badge.getAttribute('aria-label')).toContain('UCSF');
    expect(badge).toHaveAttribute('data-affiliation', 'institutions');
    // No visible text in the dense form; the words live in the label and the InfoTip.
    expect(badge.textContent?.trim()).toBe('');
    expect(badge).not.toHaveAttribute('title');
    expect(badge).toHaveAccessibleDescription(/^UCSF\. .*Compliance does not transfer/);
  });

  /**
   * The explanation is HELP, so it is an InfoTip on the badge, never a `title=`
   * (spec 2.6): a title on a span is hover-only, and no key ever reached it. The
   * words are unchanged: the pill's is the presentation's title, the dense
   * mark's leads with the label it does not show.
   */
  it('explains the pill and the dense mark with an InfoTip a keyboard reaches', async () => {
    const user = userEvent.setup();
    const words = affiliationPresentation(ucsf)!;
    render(<AffiliationBadge affiliation={ucsf} />);
    const pill = screen.getByTestId('affiliation-badge');
    expect(pill).not.toHaveAttribute('title');
    expect(pill).toHaveAttribute('tabindex', '0');
    expect(pill).toHaveClass('br-info-tip-target');
    expect(pill).toHaveAccessibleDescription(affiliationExplanation(words));
    await user.click(pill);
    expect(pill).toHaveAttribute('data-pinned', 'true');
    expect(document.querySelector('[data-slot="info-tip-content"]')).toHaveTextContent(
      affiliationExplanation(words)
    );

    cleanup();
    render(<AffiliationBadge affiliation={ucsf} dense />);
    const mark = screen.getByTestId('affiliation-badge');
    expect(mark).toHaveAttribute('tabindex', '0');
    expect(mark).toHaveAccessibleDescription(`${words.label}. ${affiliationExplanation(words)}`);
  });

  /**
   * Where the sentence the tip would say is already on screen beside the badge
   * (Switch model's affiliation row), the badge draws no InfoTip: no tab stop,
   * no description, no hidden copy of the sentence and no `title=`, so the words
   * are seen and heard once. The badge itself, its name and its word stay.
   */
  it('draws no InfoTip when its words are already on screen', () => {
    const words = affiliationPresentation(ucsf)!;
    for (const dense of [false, true]) {
      cleanup();
      const { container } = render(
        <AffiliationBadge affiliation={ucsf} dense={dense} describedOnScreen />
      );
      const badge = screen.getByTestId('affiliation-badge');
      expect(
        badge,
        `dense ${dense}: no tab stop for a tip that repeats the words on screen`
      ).not.toHaveAttribute('tabindex');
      expect(badge).not.toHaveAttribute('title');
      expect(badge).not.toHaveAttribute('aria-describedby');
      expect(badge).not.toHaveAttribute('data-slot', 'info-tip-trigger');
      expect(badge).not.toHaveClass('br-info-tip-target');
      expect(container.querySelector('[data-slot="info-tip-description"]')).toBeNull();
      expect(container.textContent ?? '').not.toContain(words.title);
      expect(badge.getAttribute('aria-label')).toBe('Model affiliation: UCSF');
      expect(badge).toHaveAttribute('data-affiliation', 'institutions');
    }
    expect(screen.getByTestId('affiliation-badge')).toHaveAttribute('role', 'img');
  });

  /**
   * Inside a control (Crew's model picker trigger is a button) the badge is part
   * of that control: no tab stop, no tip of its own, and no title either.
   */
  it('stays a plain badge inside a button or a menu row', () => {
    for (const dense of [false, true]) {
      for (const host of ['button', 'menuitem'] as const) {
        cleanup();
        const badgeElement = <AffiliationBadge affiliation={ucsf} dense={dense} />;
        render(
          host === 'button' ? (
            <button type="button">{badgeElement}</button>
          ) : (
            <div role="menuitem">{badgeElement}</div>
          )
        );
        const badge = screen.getByTestId('affiliation-badge');
        expect(badge, `${host}, dense ${dense}`).not.toHaveAttribute('tabindex');
        expect(badge).not.toHaveAttribute('title');
        expect(badge).not.toHaveAttribute('aria-describedby');
        expect(badge.getAttribute('aria-label')).toContain('UCSF');
      }
    }
  });
});
