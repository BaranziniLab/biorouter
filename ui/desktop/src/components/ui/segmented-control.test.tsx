import { useState } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';

import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';

import { SegmentedControl, type SegmentedOption } from './segmented-control';

type Mode = 'light' | 'dark' | 'system';
const OPTIONS: SegmentedOption<Mode>[] = [
  { value: 'light', label: 'Light', testId: 'light-mode-button' },
  { value: 'dark', label: 'Dark', testId: 'dark-mode-button' },
  { value: 'system', label: 'System', testId: 'system-mode-button' },
];

function Controlled({ fill, onChange }: { fill?: boolean; onChange?: (value: Mode) => void }) {
  const [value, setValue] = useState<Mode>('light');
  return (
    <div>
      <button type="button">Before</button>
      <SegmentedControl
        aria-label="Theme"
        options={OPTIONS}
        value={value}
        fill={fill}
        onValueChange={(next) => {
          setValue(next);
          onChange?.(next);
        }}
      />
      <button type="button">After</button>
    </div>
  );
}

describe('SegmentedControl', () => {
  it('is a radiogroup of radios with aria-checked, keeping each testid', () => {
    render(<Controlled />);
    const group = screen.getByRole('radiogroup', { name: 'Theme' });
    expect(group).toHaveClass('br-segmented');
    const radios = screen.getAllByRole('radio');
    expect(radios).toHaveLength(3);
    expect(screen.getByTestId('light-mode-button')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByTestId('dark-mode-button')).toHaveAttribute('aria-checked', 'false');
  });

  it('is one Tab stop', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByRole('radio', { name: 'Light' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('moves and selects with the arrow keys, and jumps with Home and End', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled onChange={onChange} />);
    act(() => screen.getByRole('radio', { name: 'Light' }).focus());
    // Held, as a person's key is: Radix selects on the focus an arrow key moves.
    await user.keyboard('{ArrowRight>}');
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: 'Dark' })).toHaveAttribute('aria-checked', 'true')
    );
    await user.keyboard('{/ArrowRight}');
    await user.keyboard('{End}');
    expect(onChange).toHaveBeenLastCalledWith('system');
    expect(screen.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
    await user.keyboard('{Home}');
    expect(onChange).toHaveBeenLastCalledWith('light');
  });

  it('selects on click', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    await user.click(screen.getByRole('radio', { name: 'System' }));
    expect(screen.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
  });

  it('draws one thumb, unplaced until measured (jsdom measures 0), and never accent', () => {
    render(<Controlled />);
    const thumbs = document.querySelectorAll('[data-slot="segmented-thumb"]');
    expect(thumbs).toHaveLength(1);
    expect(thumbs[0]).toHaveAttribute('aria-hidden', 'true');
    // Fit mode in jsdom: no width to measure, so the selected segment paints its own ground.
    expect(thumbs[0]).toHaveAttribute('data-placed', 'false');
    expect(screen.getByRole('radiogroup')).toHaveAttribute('data-placed', 'false');
    expect(thumbs[0].className).not.toMatch(/accent/);
  });

  it('fill gives equal columns and a computed thumb', () => {
    render(<Controlled fill />);
    const group = screen.getByRole('radiogroup');
    expect(group).toHaveAttribute('data-fill', 'true');
    const thumb = document.querySelector<HTMLElement>('[data-slot="segmented-thumb"]')!;
    expect(thumb).toHaveAttribute('data-placed', 'true');
    expect(thumb.style.getPropertyValue('--br-segmented-count')).toBe('3');
    expect(thumb.style.getPropertyValue('--br-segmented-index')).toBe('0');
  });

  it('places the first thumb without motion', () => {
    render(<Controlled fill />);
    const thumb = document.querySelector('[data-slot="segmented-thumb"]')!;
    expect(thumb).toHaveAttribute('data-motion-still');
  });

  it('opts the thumb into the resize rule, so it never glides against a drag', () => {
    render(<Controlled />);
    expect(document.querySelector('[data-slot="segmented-thumb"]')).toHaveAttribute(
      'data-motion-layout'
    );
  });

  it('names an icon-only segment by its ariaLabel', () => {
    render(
      <SegmentedControl
        aria-label="View"
        value="a"
        onValueChange={() => {}}
        options={[
          { value: 'a', label: 'P', ariaLabel: 'Preview' },
          { value: 'b', label: 'R', ariaLabel: 'Raw' },
        ]}
      />
    );
    expect(screen.getByRole('radio', { name: 'Preview' })).toBeInTheDocument();
  });
});

/**
 * The chosen segment is RAISED: its ground is lighter than the track it sits in, in light and in
 * dark, in every family. The spec names the thumb's ground as `--background-default`; that is
 * lighter than `--background-muted` in light and DARKER in dark (the neutrals step up from the
 * page), so the chosen segment looked sunken there. jsdom paints nothing, so the order is read
 * from the stylesheet's own tokens, the way `check-contrast.mjs` reads them.
 */
describe('SegmentedControl thumb ground (dark reads raised, not sunken)', () => {
  type Scope = { decls: Record<string, string>; theme: Record<string, string> };
  const tokens = themeTokens as unknown as {
    buildScopes(css: string): Record<string, Scope>;
    resolveHex(name: string, scope: Scope): string | null;
    contrast(a: string, b: string): number;
  };
  const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
  const scopes = tokens.buildScopes(css);

  const lightness = (hex: string) => {
    const channel = (offset: number) => parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
  };

  const thumbToken = (dark: boolean): string => {
    const rule = dark
      ? css.match(/\.dark \.br-segmented-thumb,[^{]*\{([^}]*)\}/)
      : css.match(/\n\.br-segmented-thumb \{([^}]*)\}/);
    expect(rule, `a ${dark ? 'dark' : 'light'} thumb rule`).toBeTruthy();
    const ground = rule![1].match(/background:\s*var\((--[a-z-]+)\)/);
    expect(ground, 'the thumb paints a token').toBeTruthy();
    return ground![1];
  };

  it('is lighter than the track in all six scopes', () => {
    expect(Object.keys(scopes)).toHaveLength(6);
    for (const [name, scope] of Object.entries(scopes)) {
      const token = thumbToken(name.endsWith(':dark'));
      const thumb = tokens.resolveHex(token, scope);
      const track = tokens.resolveHex('--background-muted', scope);
      expect(thumb, `${name}: ${token} resolves`).toBeTruthy();
      expect(track, `${name}: --background-muted resolves`).toBeTruthy();
      expect(lightness(thumb!), `${name}: the thumb (${token}) is raised`).toBeGreaterThan(
        lightness(track!)
      );
    }
  });

  it('keeps the selected ink readable on that ground (--text-default over the thumb, 4.5:1)', () => {
    for (const [name, scope] of Object.entries(scopes)) {
      const thumb = tokens.resolveHex(thumbToken(name.endsWith(':dark')), scope)!;
      const ink = tokens.resolveHex('--text-default', scope)!;
      expect(tokens.contrast(ink, thumb), `${name}: selected ink`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('paints the pre-measurement fallback segment on the same ground as the thumb', () => {
    expect(css).toMatch(
      /\.dark \.br-segmented-thumb,\s*\.dark \.br-segmented\[data-placed='false'\] \.br-segmented-item\[data-state='checked'\]\s*\{\s*background: var\(--background-strong\);/
    );
  });
});
