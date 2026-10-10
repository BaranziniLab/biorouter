import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Spinner } from './spinner';

describe('Spinner', () => {
  it('is decorative by default, at 16px', () => {
    const { container } = render(<Spinner />);
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveClass('br-spinner');
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    expect(svg).toHaveAttribute('data-size', '16');
    expect(svg.getAttribute('class')).not.toMatch(/animate-spin/);
  });

  it('becomes a named status when it stands alone', () => {
    render(<Spinner size={14} label="Loading skills" />);
    const status = screen.getByRole('status', { name: 'Loading skills' });
    expect(status.querySelector('svg')).toHaveAttribute('data-size', '14');
  });

  it('turns once per two --dur-slow periods, linear, and stands still under reduced motion', () => {
    const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
    expect(css).toMatch(
      /\.br-spinner \{[^}]*animation: br-spin calc\(var\(--dur-slow\) \* 2\) linear infinite;/
    );
    expect(css).toMatch(/@keyframes br-spin/);
    expect(css).toMatch(/prefers-reduced-motion: reduce\) \{\s*\.br-spinner \{\s*animation: none;/);
  });
});
