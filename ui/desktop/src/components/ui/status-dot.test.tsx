import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatusDot, type StatusDotTone } from './status-dot';

const TONES: StatusDotTone[] = ['success', 'warning', 'danger', 'neutral', 'idle'];

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

/** The body of the first rule whose selector is exactly `selector`. */
function ruleBody(selector: string): string {
  const at = MAIN_CSS.indexOf(`${selector} {`);
  expect(at, `no rule for ${selector} in main.css`).toBeGreaterThanOrEqual(0);
  return MAIN_CSS.slice(at, MAIN_CSS.indexOf('}', at));
}

describe('StatusDot', () => {
  it('is hidden from assistive technology when it sits beside a word', () => {
    const { container } = render(
      <p>
        <StatusDot tone="success" /> Connected
      </p>
    );
    const dot = container.querySelector('[data-slot="status-dot"]');
    expect(dot).toHaveAttribute('aria-hidden', 'true');
    expect(dot).not.toHaveAttribute('role');
    expect(dot).not.toHaveAttribute('aria-label');
    // The word is the only thing a screen reader hears.
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('becomes a named image when it stands alone', () => {
    render(<StatusDot tone="danger" label="Offline" />);
    const dot = screen.getByRole('img', { name: 'Offline' });
    expect(dot).not.toHaveAttribute('aria-hidden');
  });

  it('treats a blank label as no label, rather than an unnamed image', () => {
    const { container } = render(<StatusDot tone="warning" label="   " />);
    expect(screen.queryByRole('img')).toBeNull();
    expect(container.querySelector('[data-slot="status-dot"]')).toHaveAttribute(
      'aria-hidden',
      'true'
    );
  });

  it.each(TONES)('carries the %s tone as a hook the stylesheet paints', (tone) => {
    const { container } = render(<StatusDot tone={tone} />);
    const dot = container.querySelector('[data-slot="status-dot"]');
    expect(dot).toHaveAttribute('data-tone', tone);
    expect(dot).toHaveClass('biorouter-status-dot');
  });

  it('marks a live dot, and only a live one', () => {
    const { container, rerender } = render(<StatusDot tone="success" live />);
    const dot = () => container.querySelector('[data-slot="status-dot"]');
    expect(dot()).toHaveAttribute('data-live', 'true');
    rerender(<StatusDot tone="success" />);
    expect(dot()).not.toHaveAttribute('data-live');
  });

  /**
   * jsdom does not load main.css, so the paint is asserted at the source: each
   * tone maps to the token the spec names, never a literal, and the live halo
   * declares a still rest under reduced motion rather than relying on the
   * global reset alone (which would park an infinite loop on its first frame).
   */
  it('paints every tone from a token and gives the live halo a reduced-motion rest', () => {
    const expected: Record<StatusDotTone, string> = {
      success: 'var(--background-success)',
      warning: 'var(--background-warning)',
      danger: 'var(--background-danger)',
      neutral: 'var(--text-subtle)',
      idle: 'var(--background-strong)',
    };
    for (const tone of TONES) {
      expect(ruleBody(`.biorouter-status-dot[data-tone='${tone}']`)).toContain(
        `--status-dot-fill: ${expected[tone]}`
      );
    }
    expect(ruleBody('.biorouter-status-dot')).toMatch(/width: 8px;[\s\S]*height: 8px;/);
    const reduced = MAIN_CSS.slice(MAIN_CSS.indexOf('/* StatusDot reduced-motion rest */'));
    expect(reduced).toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.biorouter-status-dot\[data-live='true'\]::after \{\s*animation: none;/
    );
  });
});
