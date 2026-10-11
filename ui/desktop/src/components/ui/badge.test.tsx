import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Badge } from './badge';

describe('Badge', () => {
  it('is a span by default', () => {
    render(<Badge>KB</Badge>);
    expect(screen.getByText('KB').tagName).toBe('SPAN');
  });

  /**
   * ⚠ The reason `asChild` exists, and it is an accessibility fix rather than a
   * convenience.
   *
   * The global focus rule (design system D-15) paints
   * `background-color: var(--background-focus)` on the focused `<button>`, and
   * `tone="neutral"` is `bg-background-fill-strong` — an OPAQUE fill. A
   * `<button><Badge/></button>` therefore covers its own focus surface
   * completely, and a keyboard user gets no focus indication at all on any
   * toggle chip. With `asChild` the fill and the focus surface are the same box.
   *
   * jsdom cannot see the paint, so what is asserted is the structure that makes
   * it possible: ONE element, which is the caller's button, wearing the badge's
   * classes.
   */
  it('renders the caller’s own element, with the chip classes on it', () => {
    render(
      <Badge variant="chip" asChild className="tint-selected">
        <button type="button" aria-pressed>
          ingest
        </button>
      </Badge>
    );

    const button = screen.getByRole('button', { name: 'ingest' });
    expect(button.tagName).toBe('BUTTON');
    // The chip height, the neutral tone and the call site's own class all land
    // on the button itself — there is no wrapper between them.
    expect(button.className).toContain('h-6');
    expect(button.className).toContain('bg-background-fill-strong');
    expect(button.className).toContain('tint-selected');
    expect(button.querySelector('span')).toBeNull();
  });
});
