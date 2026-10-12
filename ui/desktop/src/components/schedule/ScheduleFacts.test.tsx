import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DefinitionRow, FactRow, ScheduleFacts, ScheduleFactsSkeleton } from './ScheduleFacts';

afterEach(cleanup);

/**
 * The schedule detail's facts are one filled card (owner message 13, R-27). The look is authored
 * in `schedule.css` and measured in `Layout/viewsFill.browser.test.tsx`; what jsdom can pin is
 * the structure that look hangs on.
 */
describe('ScheduleFacts', () => {
  it('is the card, and a filled surface so a button inside rests on the page ground', () => {
    const { container } = render(
      <ScheduleFacts>
        <FactRow label="Running">
          <button type="button">Open chat</button>
        </FactRow>
      </ScheduleFacts>
    );

    const card = container.firstElementChild!;
    expect(card).toHaveClass('br-schedule-facts', 'br-fill-surface');
    expect(card).toContainElement(screen.getByRole('button', { name: 'Open chat' }));
  });

  it('keeps a fact static: it is not a row, so it takes no hover wash or focus edge', () => {
    const { container } = render(
      <ScheduleFacts>
        <DefinitionRow label="Runs">Every day at 2:00 AM</DefinitionRow>
      </ScheduleFacts>
    );

    const fact = container.querySelector('.br-schedule-fact')!;
    expect(fact.className).not.toMatch(/biorouter-(list|settings)-row/);
    expect(fact).toHaveTextContent('RunsEvery day at 2:00 AM');
  });

  it('shows a failed value in danger ink and keeps the label muted', () => {
    render(
      <ScheduleFacts>
        <DefinitionRow label="Last error" tone="danger">
          Provider returned 429
        </DefinitionRow>
      </ScheduleFacts>
    );

    expect(screen.getByText('Provider returned 429')).toHaveClass('text-text-danger');
    expect(screen.getByText('Last error')).toHaveClass('text-text-muted');
  });

  it('rests the loading bars one step deeper than the card, and hides the card from readers', () => {
    const { container } = render(<ScheduleFactsSkeleton />);

    const card = container.firstElementChild!;
    expect(card).toHaveClass('br-schedule-facts', 'br-fill-surface');
    expect(card).toHaveAttribute('aria-hidden', 'true');
    const bars = container.querySelectorAll('[data-slot="skeleton"]');
    expect(bars.length).toBe(4);
    for (const bar of bars) {
      expect(bar).toHaveClass('bg-background-fill-strong');
      expect(bar).not.toHaveClass('bg-background-muted');
    }
  });
});
