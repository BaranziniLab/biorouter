import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Progress } from './progress';

/**
 * The progress primitive that replaced five hand-rolled bars.
 *
 * The ARIA half of this suite is real: roles and attributes are DOM, and jsdom
 * reports them faithfully. The GEOMETRY half is a source assertion against
 * `main.css`, for the usual reason — jsdom has no layout engine and never runs
 * Tailwind, so it cannot tell an 8px track from a 4px one, and it would report
 * the same `0px` for both.
 */
const CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

describe('Progress', () => {
  it('is a progressbar to a screen reader — always, not per call site', () => {
    render(<Progress label="Downloading update" value={40} />);
    const bar = screen.getByRole('progressbar', { name: 'Downloading update' });
    expect(bar).toHaveAttribute('aria-valuemin', '0');
    expect(bar).toHaveAttribute('aria-valuemax', '100');
    expect(bar).toHaveAttribute('aria-valuenow', '40');
  });

  it('clamps out-of-range values rather than overflowing the track', () => {
    const { rerender } = render(<Progress label="x" value={150} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '100');
    rerender(<Progress label="x" value={-20} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
    rerender(<Progress label="x" value={Number.NaN} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
  });

  it('scales a non-percentage max', () => {
    render(<Progress label="Files" value={3} max={12} />);
    const bar = screen.getByRole('progressbar');
    expect(bar).toHaveAttribute('aria-valuemax', '12');
    expect(bar).toHaveAttribute('aria-valuenow', '3');
    expect((bar.firstElementChild as HTMLElement).style.width).toBe('25%');
  });

  /**
   * Per ARIA, an indeterminate progressbar has NO `aria-valuenow`. Reporting 0
   * would announce a stalled download that is in fact running — the failure the
   * whole variant exists to avoid.
   */
  it('drops aria-valuenow when indeterminate, and sweeps instead of filling', () => {
    render(<Progress label="Downloading model" indeterminate />);
    const bar = screen.getByRole('progressbar', { name: 'Downloading model' });
    expect(bar).not.toHaveAttribute('aria-valuenow');
    const fill = bar.firstElementChild as HTMLElement;
    expect(fill).toHaveClass('br-progress__fill--indeterminate');
    // The sweep owns the width; an inline one would fight the animation.
    expect(fill.style.width).toBe('');
  });

  /** A download at 0% is still a download; the floor keeps it from reading dead. */
  it('honours a minimum visible width without lying about the value', () => {
    render(<Progress label="Downloading update" value={0} minVisiblePercent={4} />);
    const bar = screen.getByRole('progressbar');
    expect(bar).toHaveAttribute('aria-valuenow', '0');
    expect((bar.firstElementChild as HTMLElement).style.width).toBe('4%');
  });

  it('takes the fill as a prop rather than forcing a fork', () => {
    const { rerender } = render(<Progress label="x" value={50} />);
    expect(screen.getByRole('progressbar').firstElementChild).toHaveClass('bg-background-accent');
    rerender(<Progress label="x" value={50} tone="success" />);
    expect(screen.getByRole('progressbar').firstElementChild).toHaveClass('bg-background-success');
  });

  /**
   * The usage gauge turns its FILL danger past the limit while remaining the same
   * heat gauge; `track` is what stops the ground following the fill into neutral.
   */
  it('lets the track ground stay put while the fill changes tone', () => {
    render(<Progress label="Tokens" value={100} tone="danger" track="heat" />);
    const bar = screen.getByRole('progressbar');
    expect(bar).toHaveClass('bg-heat-0');
    expect(bar.firstElementChild).toHaveClass('bg-background-danger');
  });

  it('defaults the track ground to the tones own pairing', () => {
    const { rerender } = render(<Progress label="x" value={10} tone="heat" />);
    expect(screen.getByRole('progressbar')).toHaveClass('bg-heat-0');
    rerender(<Progress label="x" value={10} tone="accent" />);
    expect(screen.getByRole('progressbar')).toHaveClass('bg-background-muted');
  });

  describe('the geometry, asserted in main.css because jsdom cannot measure it', () => {
    const track = CSS.match(/\.br-progress\s*\{([^}]*)\}/)?.[1];
    const fill = CSS.match(/\.br-progress__fill\s*\{([^}]*)\}/)?.[1];

    it('is an 8px pill track', () => {
      expect(track).toBeTruthy();
      expect(track).toMatch(/height:\s*8px/);
      expect(track).toMatch(/border-radius:\s*var\(--radius-full\)/);
      expect(track).toMatch(/overflow:\s*hidden/);
    });

    /**
     * `--dur-med-min` is the 250ms tier, and its own comment in the token block
     * names "progress fill". The bars this replaced ran 300ms or nothing at all.
     */
    it('transitions width on the 250ms tier', () => {
      expect(fill).toBeTruthy();
      expect(fill).toMatch(/transition:\s*width var\(--dur-med-min\)/);
    });

    /** Spec 2.6: `size="thin"` is a 4px track; spec 2.4: an ambient loop's period is `--dur-slow` × n. */
    it('offers a 4px thin track and sweeps on a multiple of --dur-slow', () => {
      expect(CSS).toMatch(/\.br-progress\[data-size='thin'\]\s*\{\s*height:\s*4px;/);
      const sweep = CSS.match(/\.br-progress__fill--indeterminate\s*\{([^}]*)\}/)?.[1];
      expect(sweep).toMatch(/animation:\s*br-progress-sweep calc\(var\(--dur-slow\) \* 3\)/);
      const { rerender } = render(<Progress label="x" value={10} size="thin" />);
      expect(screen.getByRole('progressbar')).toHaveAttribute('data-size', 'thin');
      rerender(<Progress label="x" value={10} />);
      expect(screen.getByRole('progressbar')).not.toHaveAttribute('data-size');
    });

    it('has an indeterminate sweep, authored as keyframes', () => {
      expect(CSS).toMatch(/\.br-progress__fill--indeterminate\s*\{/);
      expect(CSS).toMatch(/@keyframes br-progress-sweep/);
      const sweep = CSS.match(/@keyframes br-progress-sweep\s*\{([\s\S]*?)\n\}/)?.[1];
      expect(sweep).toContain('translateX(-100%)');
      expect(sweep).toContain('translateX(250%)');
    });

    /**
     * The global reduced-motion reset nulls the duration and clamps the iteration
     * count, which would park the sliver off the right edge — an empty track that
     * reads as 0%. It has to hold as a dimmed full track instead.
     */
    it('parks the sweep as a dimmed full track under reduced motion', () => {
      const rule = CSS.match(
        /@media \(prefers-reduced-motion: reduce\) \{\s*\.br-progress__fill--indeterminate \{([^}]*)\}/
      )?.[1];
      expect(rule).toBeTruthy();
      expect(rule).toMatch(/animation:\s*none/);
      expect(rule).toMatch(/width:\s*100%/);
    });
  });

  /**
   * The point of the primitive: the five bars it replaced are gone, not merely
   * joined by a sixth. Each of these was a hand-rolled `<div>` track with a
   * `<div>` fill, and four of the five had no `role="progressbar"` at all.
   */
  describe('the five hand-rolled bars it replaced', () => {
    const CONVERTED = [
      '../UpdateAvailableModal.tsx',
      '../settings/app/UpdateSection.tsx',
      '../settings/usage/UsagePanel.tsx',
      '../ContextWindowIndicator.tsx',
      '../onboarding/OllamaInlineCard.tsx',
    ];

    it.each(CONVERTED)('%s renders the primitive and no hand-rolled track', (relative) => {
      const source = readFileSync(join(__dirname, relative), 'utf8');
      expect(source).toContain('<Progress');
      // A hand-rolled bar is a rounded track wrapping a width-driven fill. The
      // giveaway that survived every rewrite is the inline width percentage.
      expect(source).not.toMatch(/rounded-full[^"'`]*"\s*>[\s\S]{0,200}?style=\{\{\s*width:/);
    });
  });
});
