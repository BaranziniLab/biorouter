import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Note, type NoteTone } from './note';

/**
 * Class-list assertions, not computed styles.
 *
 * jsdom never runs Tailwind, so `bg-wash-warning` computes to nothing here and
 * a `toHaveStyle` assertion would pass against any class at all. What CAN be
 * checked is that the primitive emits the token-driven pair for each tone and
 * emits no per-tone border — which is the whole shape rule (§2.5: status is a
 * translucent fill with tinted ink, never an outline) and the one that the nine
 * hand-rolled recipes this replaces each broke differently.
 */
const box = (testId = 'note') => screen.getByTestId(testId);

const STATUS_TONES: NoteTone[] = ['info', 'success', 'warning', 'danger'];

describe('Note', () => {
  it('has one shape in every tone', () => {
    for (const tone of ['neutral', ...STATUS_TONES] as NoteTone[]) {
      const { unmount } = render(
        <Note tone={tone} testId={`note-${tone}`}>
          body
        </Note>
      );
      expect(box(`note-${tone}`)).toHaveClass(
        'rounded-element',
        'px-3',
        'py-2.5',
        'text-supporting',
        'flex',
        'items-start',
        'gap-2'
      );
      unmount();
    }
  });

  it.each(STATUS_TONES)('paints %s as a wash with tinted ink and no coloured border', (tone) => {
    render(<Note tone={tone} testId="note">{`the ${tone} note`}</Note>);
    const classes = box().className;
    expect(classes).toContain(`bg-wash-${tone}`);
    expect(classes).toContain(`text-text-${tone}`);
    // A status note is a fill, not an outline. `border-{tone}` in any spelling
    // is the recipe this primitive exists to retire.
    expect(classes).not.toMatch(/\bborder-(?:border-)?(?:info|success|warning|danger)/);
    expect(classes).not.toMatch(/\bborder\b/);
  });

  /**
   * `neutral` has no hue to wash, so it takes the subtle solid fill (owner message 13, R-27):
   * no outline, the 1px border kept transparent so forced colours still draw the box.
   */
  it('gives neutral the subtle fill and no outline', () => {
    render(
      <Note tone="neutral" testId="note">
        body
      </Note>
    );
    expect(box()).toHaveClass('border', 'border-transparent', 'bg-background-fill');
    expect(box().className).not.toMatch(/border-border-|bg-background-muted|bg-wash-/);
  });

  it('defaults to neutral', () => {
    render(<Note testId="note">body</Note>);
    expect(box()).toHaveClass('bg-background-fill');
  });

  /** A fill never sits on a fill: the action a note holds steps back to the page ground. */
  it.each(['neutral', ...STATUS_TONES] as const)(
    'makes the %s note a fill surface for the controls it holds',
    (tone) => {
      render(
        <Note tone={tone} testId="note">
          body
        </Note>
      );
      expect(box()).toHaveClass('br-fill-surface');
    }
  );

  it('passes role and testId through, and merges a layout className', () => {
    render(
      <Note tone="danger" role="alert" testId="privacy-toggle-error" className="mt-3 min-w-0">
        it failed
      </Note>
    );
    const element = screen.getByTestId('privacy-toggle-error');
    expect(element).toHaveAttribute('role', 'alert');
    expect(element).toHaveClass('mt-3', 'min-w-0');
    // Merged, not replaced: the shape survives the call site's layout classes.
    expect(element).toHaveClass('rounded-element', 'bg-wash-danger');
  });

  it('renders the glyph at the row scale, top-aligned', () => {
    const Icon = ({ className }: { className?: string }) => (
      <svg data-testid="note-icon" className={className} />
    );
    render(
      <Note tone="warning" icon={Icon} testId="note">
        body
      </Note>
    );
    expect(screen.getByTestId('note-icon')).toHaveClass('h-4', 'w-4', 'mt-0.5', 'shrink-0');
  });

  it('renders an action beside the body', () => {
    render(
      <Note testId="note" action={<button type="button">Retry</button>}>
        body
      </Note>
    );
    // Centred on the note's first line by `.br-note-action` (main.css; measured in Chromium by
    // filledPrimitives.browser.test.tsx).
    expect(screen.getByRole('button', { name: 'Retry' }).parentElement).toHaveClass(
      'br-note-action'
    );
  });

  /**
   * The fade has to end on the note's own composite, so the ground rides in as
   * an inline custom property rather than as a per-tone arbitrary utility that
   * could fail to generate.
   */
  it.each([
    ['neutral', 'var(--background-fill)'],
    ['warning', 'var(--wash-solid-warning)'],
  ] as const)('hands the clamp %s’s own fade ground', (tone, expected) => {
    render(
      <Note tone={tone} testId="note">
        body
      </Note>
    );
    expect(box().getAttribute('style')).toContain(`--note-fade-ground: ${expected}`);
  });

  /**
   * ⚠ The clamp itself cannot be exercised here: jsdom resolves no custom
   * properties and reports `scrollHeight: 0`, so the primitive measures no
   * overflow and neither clamps nor offers "Show more" in any render test. That
   * is asserted at the source instead (`styles/noteClamp.test.ts`), and this
   * pair pins the observable half — an unmeasurable note never grows a control,
   * and `unclamped` never grows one either.
   */
  it('offers no control until something actually overflows', () => {
    render(<Note testId="note">a short line</Note>);
    expect(screen.queryByRole('button', { name: /Show more/ })).toBeNull();
    expect(box().querySelector('.biorouter-note-clamp')).toBeNull();
  });

  it('never clamps when `unclamped` is set', () => {
    render(
      <Note testId="note" unclamped>
        a mandated disclosure
      </Note>
    );
    expect(box().querySelector('.biorouter-note-clamp')).toBeNull();
    expect(screen.queryByRole('button', { name: /Show more/ })).toBeNull();
  });
});
