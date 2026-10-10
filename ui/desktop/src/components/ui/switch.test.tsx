import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { Switch } from './switch';

const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');

/** The declarations of the first rule whose selector list is exactly `selector`. */
function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = css.match(new RegExp(`(?:^|\\n)${escaped} \\{([^}]*)\\}`));
  expect(match, `rule ${selector}`).not.toBeNull();
  return match![1];
}

describe('Switch geometry and colour (authored in main.css)', () => {
  it('is a 32×20 round track: off on --control-track-off, on on the accent', () => {
    const track = rule('.br-switch');
    expect(track).toMatch(/inline-size: 32px;/);
    expect(track).toMatch(/block-size: 20px;/);
    expect(track).toMatch(/border-radius: var\(--radius-full\);/);
    expect(track).toMatch(/background-color: var\(--control-track-off\);/);
    expect(rule(".br-switch[data-state='checked']")).toMatch(
      /background-color: var\(--background-accent\);/
    );
  });

  it('has a 24px hit target', () => {
    expect(rule('.br-switch::after')).toMatch(/inset: -2px 0;/);
  });

  it('has a 16px knob inset 2px in both states that never changes size', () => {
    const knob = rule('.br-switch-thumb');
    expect(knob).toMatch(/inline-size: 16px;/);
    expect(knob).toMatch(/block-size: 16px;/);
    expect(knob).toMatch(/transform: translateX\(2px\);/);
    const on = rule(
      ".br-switch-thumb[data-state='checked'],\n.dark .br-switch-thumb[data-state='checked']"
    );
    expect(on).toMatch(/transform: translateX\(14px\);/);
    expect(on).not.toMatch(/size|width|height/);
  });

  it('rings the off knob with --border-control and fills it with --text-muted in dark', () => {
    expect(rule('.br-switch-thumb')).toMatch(/0 0 0 1px var\(--border-control\)/);
    expect(rule('.dark .br-switch-thumb')).toMatch(/background-color: var\(--text-muted\);/);
  });

  it('moves the knob and the track over --dur-fast with --ease-out, and lands at rest when still', () => {
    expect(rule('.br-switch')).toMatch(
      /transition: background-color var\(--dur-fast\) var\(--ease-out\);/
    );
    expect(rule('.br-switch-thumb')).toMatch(/transform var\(--dur-fast\) var\(--ease-out\)/);
    expect(
      rule('.br-switch[data-motion-still],\n.br-switch[data-motion-still] > .br-switch-thumb')
    ).toMatch(/transition-duration: 0ms;/);
  });
});

describe('Switch behaviour', () => {
  it('is a Radix button[role=switch] named by the visible label', () => {
    render(
      <div>
        <span id="spell-label">Spellcheck</span>
        <Switch aria-labelledby="spell-label" checked={false} onCheckedChange={() => {}} />
      </div>
    );
    const control = screen.getByRole('switch', { name: 'Spellcheck' });
    expect(control.tagName).toBe('BUTTON');
    expect(control).toHaveClass('br-switch');
    expect(control.querySelector('.br-switch-thumb')).not.toBeNull();
  });

  it('ignores the retired variant prop', () => {
    render(
      <>
        <Switch aria-label="A" variant="mono" checked={false} onCheckedChange={() => {}} />
        <Switch aria-label="B" checked={false} onCheckedChange={() => {}} />
      </>
    );
    expect(screen.getByRole('switch', { name: 'A' }).className).toBe(
      screen.getByRole('switch', { name: 'B' }).className
    );
  });

  it('lands a value that arrives without a toggle at rest, and animates a toggle', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <Switch aria-label="Sync" checked={false} onCheckedChange={() => {}} />
    );
    const control = screen.getByRole('switch', { name: 'Sync' });
    expect(control).not.toHaveAttribute('data-motion-still');

    // The saved value loads after mount: no slide.
    rerender(<Switch aria-label="Sync" checked onCheckedChange={() => {}} />);
    expect(control).toHaveAttribute('data-motion-still');

    function Toggle() {
      const [on, setOn] = useState(false);
      return <Switch aria-label="Toggle me" checked={on} onCheckedChange={setOn} />;
    }
    render(<Toggle />);
    const mine = screen.getByRole('switch', { name: 'Toggle me' });
    await user.click(mine);
    expect(mine).toHaveAttribute('aria-checked', 'true');
    expect(mine).not.toHaveAttribute('data-motion-still');
  });
});
