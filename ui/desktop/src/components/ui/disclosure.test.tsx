import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Disclosure } from './disclosure';

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

function Form({ defaultOpen }: { defaultOpen?: boolean }) {
  return (
    <form aria-label="Connection">
      <label>
        Server
        <input name="host" defaultValue="hpc.ucsf.edu" />
      </label>
      <Disclosure summary="Port 22 · your SSH settings" defaultOpen={defaultOpen}>
        <label>
          Port
          <input name="port" defaultValue="22" />
        </label>
      </Disclosure>
    </form>
  );
}

describe('Disclosure', () => {
  it('is closed by default, labelled Advanced, and renders nothing inside', () => {
    render(<Form />);
    const trigger = screen.getByRole('button', { name: 'Advanced' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    // Unmounted, not merely hidden: not in the accessibility tree, the tab order
    // or the DOM, so a closed Advanced section can never submit a stale field.
    expect(screen.queryByLabelText('Port')).toBeNull();
    expect(document.querySelector('input[name="port"]')).toBeNull();
  });

  it('states the defaults while closed, as the trigger description', () => {
    render(<Form />);
    const trigger = screen.getByRole('button', { name: 'Advanced' });
    expect(screen.getByText('Port 22 · your SSH settings')).toBeInTheDocument();
    expect(trigger).toHaveAccessibleDescription('Port 22 · your SSH settings');
  });

  it('mounts the body on open and unmounts it again on close', async () => {
    const user = userEvent.setup();
    render(<Form />);
    const trigger = screen.getByRole('button', { name: 'Advanced' });

    await user.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByLabelText('Port')).toHaveValue('22');
    // Once open, the fields speak for themselves.
    expect(screen.queryByText('Port 22 · your SSH settings')).toBeNull();
    expect(trigger).not.toHaveAttribute('aria-describedby');

    await user.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByLabelText('Port')).toBeNull();
    expect(screen.getByText('Port 22 · your SSH settings')).toBeInTheDocument();
  });

  it('starts open with defaultOpen', () => {
    render(<Form defaultOpen />);
    expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
    expect(screen.getByLabelText('Port')).toBeInTheDocument();
    expect(screen.queryByText('Port 22 · your SSH settings')).toBeNull();
  });

  it('can be controlled, so a caller can open it to reveal an invalid field', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    function Controlled() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Reveal
          </button>
          <Disclosure
            label="Trouble signing in?"
            open={open}
            onOpenChange={(next) => {
              onOpenChange(next);
              setOpen(next);
            }}
          >
            <p>Check the server name.</p>
          </Disclosure>
        </>
      );
    }
    render(<Controlled />);
    expect(screen.queryByText('Check the server name.')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Reveal' }));
    expect(screen.getByText('Check the server name.')).toBeInTheDocument();
    expect(onOpenChange).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Trouble signing in?' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(screen.queryByText('Check the server name.')).toBeNull();
  });

  it('is a ghost small button in muted ink, never a submit', () => {
    render(<Form />);
    const trigger = screen.getByRole('button', { name: 'Advanced' });
    expect(trigger).toHaveAttribute('type', 'button');
    expect(trigger).toHaveClass('h-control-sm', 'text-text-muted', 'bg-transparent');
    expect(trigger.querySelector('svg')).toHaveClass('biorouter-disclosure-chevron');
  });

  /**
   * jsdom does not load main.css, so the motion contract is read at the source:
   * the body opens over `--dur-med` and closes over `--dur-fast` (exits are
   * shorter than entrances), the chevron turns 90°, and reduced motion is a
   * declared rest rather than an accident of the global reset.
   */
  it('authors its motion from the duration ladder, with a reduced-motion rest', () => {
    expect(MAIN_CSS).toMatch(
      /\.biorouter-disclosure-panel\[data-state='open'\] \{\s*animation: biorouter-disclosure-open var\(--dur-med\) var\(--ease-out\);/
    );
    expect(MAIN_CSS).toMatch(
      /\.biorouter-disclosure-panel\[data-state='closed'\] \{\s*animation: biorouter-disclosure-close var\(--dur-fast\) var\(--ease-out\);/
    );
    expect(MAIN_CSS).toMatch(/@keyframes biorouter-disclosure-open \{/);
    expect(MAIN_CSS).toMatch(/--radix-collapsible-content-height/);
    expect(MAIN_CSS).toMatch(
      /\[data-state='open'\] > \.biorouter-disclosure-chevron \{\s*transform: rotate\(90deg\);\s*transition-duration: var\(--dur-fast-max\);/
    );
    const reduced = MAIN_CSS.slice(MAIN_CSS.indexOf('/* Disclosure reduced-motion rest */'));
    expect(reduced).toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.biorouter-disclosure-panel\[data-state\] \{\s*animation: none;/
    );
  });
});
