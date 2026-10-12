import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DisclosureProse } from './DisclosureProse';

afterEach(cleanup);

/**
 * The served long form is rendered by one component for the dialog and for Settings, App, Privacy
 * (issue #56, DR-17 requirement 3), and `**…**` marks the clauses that say what is NOT protected.
 *
 * V4-R7-6, class only (privacy UI, ruling 15): a bare `<strong>` computes to `bolder`, 700, and the
 * type system draws only 400 and 600. Each emphasised clause wears `font-semibold`;
 * `privacyEmphasisWeight.browser.test.tsx` measures the computed weight in Chromium.
 */
const TEXT = [
  'Biorouter stops three things.',
  'It does **not** stop a model reading **files on this computer**.',
  'A last paragraph with nothing marked.',
].join('\n\n');

describe('DisclosureProse', () => {
  it('sets every emphasised clause at the strong weight, 600', () => {
    const { container } = render(<DisclosureProse text={TEXT} />);
    const emphasised = [...container.querySelectorAll('strong')];
    expect(emphasised.map((el) => el.textContent)).toEqual(['not', 'files on this computer']);
    for (const el of emphasised) expect(el).toHaveClass('font-semibold');
  });

  it('prints no marker and keeps the words and the paragraphs as served', () => {
    const { container } = render(<DisclosureProse text={TEXT} paragraphClassName="x" />);
    const paragraphs = [...container.querySelectorAll('p')];
    expect(paragraphs).toHaveLength(3);
    expect(paragraphs.every((p) => p.className === 'x')).toBe(true);
    expect(paragraphs[1].textContent).toBe(
      'It does not stop a model reading files on this computer.'
    );
    expect(container.textContent).not.toContain('**');
  });
});
