import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HIGHLIGHT_CLASS, HIGHLIGHT_FALLBACK_MS, highlightRow } from './highlightRow';
import { DUR } from '../../styles/motion';

describe('highlightRow', () => {
  let row: HTMLElement;

  beforeEach(() => {
    vi.useFakeTimers();
    row = document.createElement('div');
    document.body.append(row);
  });

  afterEach(() => {
    vi.useRealTimers();
    row.remove();
  });

  it('takes the shared wash class, not an inline ring', () => {
    highlightRow(row);

    expect(row).toHaveClass(HIGHLIGHT_CLASS);
    expect(HIGHLIGHT_CLASS).toBe('br-highlight');
    // The old mechanism painted a box-shadow with the retired teal alias.
    expect(row.style.boxShadow).toBe('');
  });

  it('lets the class go when its own animation ends', () => {
    highlightRow(row);
    row.dispatchEvent(new Event('animationend'));

    expect(row).not.toHaveClass(HIGHLIGHT_CLASS);
  });

  it('ignores the animation of a child, which bubbles', () => {
    highlightRow(row);
    const child = document.createElement('span');
    row.append(child);
    child.dispatchEvent(new Event('animationend', { bubbles: true }));

    expect(row).toHaveClass(HIGHLIGHT_CLASS);
  });

  it('never leaves the class behind where no animation runs at all', () => {
    highlightRow(row);
    vi.advanceTimersByTime(HIGHLIGHT_FALLBACK_MS - 1);
    expect(row).toHaveClass(HIGHLIGHT_CLASS);

    vi.advanceTimersByTime(2);
    expect(row).not.toHaveClass(HIGHLIGHT_CLASS);
  });

  it('waits out the wash’s whole period, three slow periods, before giving up', () => {
    expect(HIGHLIGHT_FALLBACK_MS).toBeGreaterThan(3 * DUR.slow);
  });

  it('cancels at once, and a second highlight of the same row restarts it', () => {
    const cancel = highlightRow(row);
    cancel();
    expect(row).not.toHaveClass(HIGHLIGHT_CLASS);

    highlightRow(row);
    vi.advanceTimersByTime(HIGHLIGHT_FALLBACK_MS - 100);
    highlightRow(row);
    vi.advanceTimersByTime(HIGHLIGHT_FALLBACK_MS - 100);
    // The first timer would have fired by now; the restart keeps it for its own period.
    expect(row).toHaveClass(HIGHLIGHT_CLASS);
  });
});
