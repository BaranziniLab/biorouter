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
   * `tone="neutral"` paints the strong fill (`.br-badge-neutral`), an OPAQUE fill. A
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
    expect(button.className).toContain('br-badge-neutral');
    expect(button.className).toContain('tint-selected');
    expect(button.querySelector('span')).toBeNull();
  });

  /**
   * The neutral fill is an authored class that reads `--br-badge-ground`, so a washed row can step
   * the badge back to the page ground (main.css, the rows block; V3-R2-5). A Tailwind fill utility
   * here would out-rank that class and no ground could move the badge, so the tone must not carry
   * one; `groundsAndRows.browser.test.tsx` paints both grounds in Chromium.
   */
  it('paints the neutral tone through the ground-reading class, and only the neutral tone', () => {
    render(
      <>
        <Badge>Built-in</Badge>
        <Badge tone="danger">Flag</Badge>
        <Badge className="bg-background-muted">Public</Badge>
      </>
    );
    const neutral = screen.getByText('Built-in');
    expect(neutral.className).toContain('br-badge-neutral');
    expect(neutral.className).not.toMatch(/(^|\s)bg-/);
    expect(screen.getByText('Flag').className).not.toContain('br-badge-neutral');
    // A caller's own fill is kept beside it and wins by layer (the privacy pills' muted rest).
    expect(screen.getByText('Public').className).toMatch(/br-badge-neutral.*bg-background-muted/);
  });
});
