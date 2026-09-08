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
   * `neutral` has no hue to wash, so it takes a real surface step plus a
   * hairline — the same exception `badge.tsx` documents for its own neutral.
   */
  it('gives neutral a surface step and a hairline instead', () => {
    render(
      <Note tone="neutral" testId="note">
        body
      </Note>
    );
    expect(box()).toHaveClass('border', 'border-border-subtle', 'bg-background-muted');
    expect(box().className).not.toContain('bg-wash-');
  });

  it('defaults to neutral', () => {
    render(<Note testId="note">body</Note>);
    expect(box()).toHaveClass('bg-background-muted');
  });

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
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });

  /**
   * The fade has to end on the note's own composite, so the ground rides in as
   * an inline custom property rather than as a per-tone arbitrary utility that
   * could fail to generate.
   */
  it.each([
    ['neutral', 'var(--background-muted)'],
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
