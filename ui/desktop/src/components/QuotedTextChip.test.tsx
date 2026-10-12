import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { QuotedTextChip } from './QuotedTextChip';
import type { QuoteReference } from '../utils/quotedText';

const quote: QuoteReference = {
  kind: 'quote',
  label: 'Response',
  value: 'The first line.\nThe second line.',
  sourceLocator: 'chat/42',
  start: 0,
  end: 0,
};

describe('QuotedTextChip', () => {
  it('says where the words came from, then the words', () => {
    render(<QuotedTextChip quote={quote} />);
    const chip = screen.getByTestId('quoted-text-chip');
    expect(chip).toHaveTextContent('Response');
    expect(chip).toHaveTextContent('The first line.');
    expect(chip).toHaveTextContent('The second line.');
  });

  it('is a rule beside the words, never a box in the card or the bubble', () => {
    render(<QuotedTextChip quote={quote} />);
    const chip = screen.getByTestId('quoted-text-chip');
    expect(chip).toHaveClass('br-quote-chip');
    // No fill, no outline and no padding utility: the 2px rule is authored CSS
    // (`composer.css`, measured in `composerGeometry.browser.test.ts`).
    expect(chip.className).not.toMatch(/\b(bg-|border|rounded|p[xy]?-)/);
  });

  it('keeps the source locator in a tooltip, not on the line', () => {
    render(<QuotedTextChip quote={quote} />);
    expect(screen.getByText('Response')).toHaveAttribute('title', 'chat/42');
    expect(screen.queryByText('chat/42')).toBeNull();
  });

  it('offers a remove control only where the quote can be removed', () => {
    const onRemove = vi.fn();
    const { rerender } = render(<QuotedTextChip quote={quote} />);
    expect(screen.queryByRole('button')).toBeNull();

    rerender(<QuotedTextChip quote={quote} onRemove={onRemove} />);
    fireEvent.click(screen.getByRole('button', { name: 'Remove quote from Response' }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});
