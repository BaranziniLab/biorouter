import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatusPill } from './StatusPill';

// FR-S-10: every first-run status line drew its own 6px dot, beside the catalog's 8px shared one,
// and none of the hand-drawn dots was hidden from a screen reader.
describe('StatusPill', () => {
  it('is the shared status dot beside its word, hidden from assistive tech', () => {
    render(<StatusPill tone="warning">Not installed</StatusPill>);
    const pill = screen.getByText('Not installed');
    const dot = pill.querySelector('[data-slot="status-dot"]');
    expect(dot).not.toBeNull();
    expect(dot).toHaveAttribute('data-tone', 'warning');
    expect(dot).toHaveAttribute('aria-hidden');
    expect(pill).toHaveClass('br-status-pill');
  });

  it('draws a state that is not a fault with the neutral dot', () => {
    render(<StatusPill tone="info">Signed in with an API key</StatusPill>);
    const dot = screen.getByText('Signed in with an API key').querySelector('[data-slot]');
    expect(dot).toHaveAttribute('data-tone', 'neutral');
  });
});
