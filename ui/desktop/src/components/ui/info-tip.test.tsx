import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { InfoTip, INFO_TIP_OPEN_DELAY_MS, useInfoTipId } from './info-tip';
import { Badge } from './badge';

const HELP = 'Checks spelling in the chat input.';

const surface = () => document.querySelector('[data-slot="info-tip-content"]');
const trigger = () => screen.getByRole('button', { name: 'About Spellcheck' });

function Page() {
  return (
    <div>
      <button type="button">Before</button>
      <span>Spellcheck</span>
      <InfoTip label="Spellcheck" help={HELP} />
    </div>
  );
}

describe('InfoTip', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('is named "About {label}" and described by a hidden node that exists while closed', () => {
    render(<Page />);
    expect(trigger()).toHaveAccessibleName('About Spellcheck');
    expect(trigger()).toHaveAccessibleDescription(HELP);
    const describedBy = trigger().getAttribute('aria-describedby');
    expect(describedBy).toBeTruthy();
    const node = document.getElementById(describedBy!);
    expect(node).toHaveTextContent(HELP);
    expect(node).toHaveClass('sr-only');
    expect(surface()).toBeNull();
    // A copy of the text stays findable, so tests that read the old paragraph keep passing.
    expect(screen.getByText(HELP)).toBeInTheDocument();
  });

  it('does not open before 200ms of hover, and opens after', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS - 10);
    });
    expect(surface()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(20);
    });
    expect(surface()).toHaveTextContent(HELP);
  });

  it('closes after the pointer leaves, with a short grace', () => {
    vi.useFakeTimers();
    render(<Page />);
    fireEvent.pointerEnter(trigger(), { pointerType: 'mouse' });
    act(() => {
      vi.advanceTimersByTime(INFO_TIP_OPEN_DELAY_MS + 10);
    });
    expect(surface()).not.toBeNull();
    fireEvent.pointerLeave(trigger(), { pointerType: 'mouse' });
    expect(surface()).not.toBeNull();
    act(() => {
      vi.advanceTimersByTime(150);
    });
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');
  });

  it('opens at once when Tab moves focus onto it', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(trigger()).toHaveFocus();
    expect(surface()).toHaveTextContent(HELP);
  });

  it('stays shut when a program focuses it (a focus restore)', async () => {
    render(<Page />);
    act(() => trigger().focus());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(surface()).toBeNull();
  });

  it('toggles on click and closes on Escape', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await user.click(trigger());
    expect(surface()).toHaveTextContent(HELP);
    await user.click(trigger());
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');

    await user.click(trigger());
    expect(surface()).not.toBeNull();
    await user.keyboard('{Escape}');
    expect(surface()?.getAttribute('data-state') ?? 'closed').toBe('closed');
  });

  it('does not let a click reach a row that handles clicks', async () => {
    const user = userEvent.setup();
    const onRowClick = vi.fn();
    render(
      <div onClick={onRowClick}>
        <InfoTip label="Spellcheck" help={HELP} />
      </div>
    );
    await user.click(trigger());
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it('lets the explained control point at the same description with useInfoTipId', () => {
    function Row() {
      const id = useInfoTipId();
      return (
        <div>
          <button type="button" aria-describedby={id}>
            Spellcheck switch
          </button>
          <InfoTip label="Spellcheck" help={HELP} id={id} />
        </div>
      );
    }
    render(<Row />);
    expect(screen.getByRole('button', { name: 'Spellcheck switch' })).toHaveAccessibleDescription(
      HELP
    );
  });

  it('makes a badge the focusable trigger with asChild, keeping its own name', () => {
    render(
      <InfoTip label="Legacy" help="Made before the format existed." asChild>
        <Badge>Legacy</Badge>
      </InfoTip>
    );
    const badge = screen.getByText('Legacy');
    expect(badge).toHaveAttribute('tabindex', '0');
    expect(badge).toHaveClass('br-info-tip-target');
    expect(badge).toHaveAccessibleDescription('Made before the format existed.');
  });

  it('accepts the help as children', () => {
    render(<InfoTip label="Spellcheck">{HELP}</InfoTip>);
    expect(trigger()).toHaveAccessibleDescription(HELP);
  });
});
