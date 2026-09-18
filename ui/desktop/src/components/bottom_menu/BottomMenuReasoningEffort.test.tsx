import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { BottomMenuReasoningEffort } from './BottomMenuReasoningEffort';
import {
  getReasoningEffort,
  reasoningEffortForRequest,
  REASONING_EFFORT_LABELS,
  resetReasoningEffortForTests,
  setReasoningEffort,
} from '../../store/reasoningEffort';

describe('BottomMenuReasoningEffort (BR-63)', () => {
  beforeEach(() => {
    localStorage.clear();
    resetReasoningEffortForTests();
  });

  it('starts on the default and shows no level chip', () => {
    render(<BottomMenuReasoningEffort scope="session:test" />);

    const trigger = screen.getByLabelText('Reasoning effort: Normal');
    expect(trigger).toBeInTheDocument();
    expect(trigger).not.toHaveAttribute('title');
    expect(trigger.querySelector('svg')).toHaveClass('size-[17px]');
    expect(screen.queryByText('Normal')).not.toBeInTheDocument();
  });

  /**
   * The glyph is three rising bars filled up to the level, so it carries the
   * VALUE rather than the concept — which is the whole reason it replaced the
   * `Gauge`, whose three states were pixel-identical. Asserting the fill count
   * is asserting the only thing that makes the swap worth anything: a glyph that
   * rendered the same at quick and deep would pass every other test in this file.
   */
  it.each([
    ['quick', 1],
    ['normal', 2],
    ['deep', 3],
  ] as const)('fills %s to %i of three bars', (level, expected) => {
    setReasoningEffort('session:test', level);
    render(<BottomMenuReasoningEffort scope="session:test" />);

    const bars = Array.from(
      screen
        .getByLabelText(`Reasoning effort: ${REASONING_EFFORT_LABELS[level]}`)
        .querySelectorAll('rect')
    );
    expect(bars).toHaveLength(3);
    expect(bars.filter((bar) => bar.getAttribute('opacity') === '1')).toHaveLength(expected);
  });

  it('keeps every option aligned while showing only the selected tick', () => {
    render(<BottomMenuReasoningEffort scope="session:test" />);

    fireEvent.click(screen.getByLabelText('Reasoning effort: Normal'));
    const options = screen.getAllByRole('menuitemradio');

    options.forEach((option) => {
      expect(Array.from(option.children).some((child) => child.tagName === 'svg')).toBe(false);
      expect(option.lastElementChild).toHaveClass('size-3.5', 'shrink-0');
    });
    expect(options.filter((option) => option.querySelector('svg'))).toHaveLength(1);
  });

  it('picking deep updates the store, so the next chat request carries it', () => {
    render(<BottomMenuReasoningEffort scope="session:test" />);

    fireEvent.click(screen.getByLabelText('Reasoning effort: Normal'));
    fireEvent.click(screen.getByRole('menuitemradio', { name: /Deep/ }));

    expect(getReasoningEffort('session:test')).toBe('deep');
    expect(reasoningEffortForRequest(getReasoningEffort('session:test'))).toBe('deep');
    // The trigger now advertises the non-default level.
    expect(screen.getByLabelText('Reasoning effort: Deep')).toBeInTheDocument();
  });

  it('going back to normal drops the level from the request', () => {
    render(<BottomMenuReasoningEffort scope="session:test" />);

    fireEvent.click(screen.getByLabelText('Reasoning effort: Normal'));
    fireEvent.click(screen.getByRole('menuitemradio', { name: /Quick/ }));
    expect(reasoningEffortForRequest(getReasoningEffort('session:test'))).toBe('quick');

    fireEvent.click(screen.getByLabelText('Reasoning effort: Quick'));
    fireEvent.click(screen.getByRole('menuitemradio', { name: /Normal/ }));
    expect(reasoningEffortForRequest(getReasoningEffort('session:test'))).toBeUndefined();
  });
});
