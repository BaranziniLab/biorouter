/**
 * The preview's two ends.
 *
 * `tail` exists because logs and arguments want opposite halves of the same
 * text: an argument's first lines say what was asked, a running tool's last
 * lines say where it got to. A head preview of a live log shows the first
 * second of a minute-long run, forever, and that is what shipped before this.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ToolContentPreview } from './ToolContentPreview';

const body = (text: string) => (
  <ToolContentPreview text={text} tail={false}>
    {(visible) => <pre data-testid="out">{visible}</pre>}
  </ToolContentPreview>
);
const tail = (text: string) => (
  <ToolContentPreview text={text} tail>
    {(visible) => <pre data-testid="out">{visible}</pre>}
  </ToolContentPreview>
);
const shown = () => screen.getByTestId('out').textContent;

const TEN = Array.from({ length: 10 }, (_, i) => `line-${i}`).join('\n');

describe('ToolContentPreview', () => {
  it('previews the first lines by default', () => {
    render(body(TEN));
    expect(shown()).toContain('line-0');
    expect(shown()).not.toContain('line-9');
  });

  it('previews the LAST lines with tail', () => {
    render(tail(TEN));
    expect(shown()).toContain('line-9');
    // The negative control: without it, a `tailPreview` that returned the whole
    // text would satisfy the assertion above.
    expect(shown()).not.toContain('line-0');
  });

  it('keeps the newest line whole when the character cap bites', () => {
    // One very long line followed by a short newest one: the cut must land at
    // the FRONT, so the newest line survives intact.
    const text = `${'x'.repeat(2000)}\nnewest-line`;
    render(tail(text));
    expect(shown()).toContain('newest-line');
    expect(shown()!.length).toBeLessThanOrEqual(600);
  });

  it('offers Show more only when something is hidden', () => {
    const { unmount } = render(tail(TEN));
    expect(screen.getByRole('button', { name: 'Show more' })).toBeInTheDocument();
    unmount();
    render(tail('one line'));
    expect(screen.queryByRole('button', { name: 'Show more' })).not.toBeInTheDocument();
    expect(shown()).toBe('one line');
  });
});
