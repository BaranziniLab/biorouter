import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import type { FixedExtensionEntry } from '../../../ConfigContext';
import ExtensionList from './ExtensionList';

const entry = (name: string, enabled: boolean) =>
  ({
    type: 'stdio',
    name,
    description: `${name} description`,
    cmd: 'node',
    args: ['server.js'],
    enabled,
  }) as FixedExtensionEntry;

/**
 * The page's own loop in miniature: a toggle resolves, the config is re-read, and the list is
 * handed the new `enabled` flags (`ExtensionsSection.handleExtensionToggle`). `fail` makes the
 * toggle reject, as a config write that did not land does; `add` is an extension that appears
 * from somewhere else (an install).
 */
function Harness({
  initial,
  fail = false,
  extra,
  gate,
}: {
  initial: FixedExtensionEntry[];
  fail?: boolean;
  extra?: FixedExtensionEntry;
  /** A toggle that takes its time: the config write has not come back until this resolves. */
  gate?: Promise<void>;
}) {
  const [extensions, setExtensions] = useState(initial);
  const shown = extra ? [...extensions, extra] : extensions;
  return (
    <ExtensionList
      extensions={shown}
      onToggle={async (extension) => {
        await gate;
        if (fail) throw new Error('could not write config');
        setExtensions((current) =>
          current.map((e) => (e.name === extension.name ? { ...e, enabled: !e.enabled } : e))
        );
        return true;
      }}
    />
  );
}

const heading = (name: RegExp) => screen.getByRole('heading', { level: 2, name });

const initial = () => [entry('alpha', true), entry('bravo', false), entry('charlie', false)];

describe('ExtensionList: a toggle moves its row to the other group', () => {
  it('keeps keyboard focus on the switch that was pressed, after the row has moved', async () => {
    // The row is re-created under the other group, which removes the focused element from the
    // page: a keyboard user pressed Space and was dropped at the top of the document.
    const user = userEvent.setup({ delay: null });
    render(<Harness initial={initial()} />);

    const bravo = screen.getByRole('switch', { name: 'Bravo' });
    bravo.focus();
    await user.keyboard(' ');

    // No pointer is in the list, so the row moves at once.
    await waitFor(() => expect(heading(/^Enabled 2$/)).toBeVisible());
    expect(heading(/^Available 1$/)).toBeVisible();
    const after = screen.getByRole('switch', { name: 'Bravo' });
    expect(after).toHaveAttribute('aria-checked', 'true');
    expect(after).toHaveFocus();
  });

  it('leaves the rows where they are while the pointer is in the list, then moves the row when it leaves', async () => {
    // Spec principle 9: a row never moves under the pointer. Enabling a row used to pull it up
    // into the other group and slide every row below it under a pointer that was about to
    // click one of them.
    const user = userEvent.setup({ delay: null });
    render(<Harness initial={initial()} />);

    await user.click(screen.getByRole('switch', { name: 'Bravo' }));

    // The switch answers at once; the groups, and their counts, wait.
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: 'Bravo' })).toHaveAttribute('aria-checked', 'true')
    );
    expect(heading(/^Enabled 1$/)).toBeVisible();
    expect(heading(/^Available 2$/)).toBeVisible();

    // The pointer leaves: now the row relocates.
    await user.pointer({ target: document.body });
    await waitFor(() => expect(heading(/^Enabled 2$/)).toBeVisible());
    expect(heading(/^Available 1$/)).toBeVisible();
    const moved = screen.getByRole('switch', { name: 'Bravo' });
    expect(moved).toHaveAttribute('aria-checked', 'true');
    // The switch the person clicked keeps focus through the move.
    expect(moved).toHaveFocus();
  });

  it('does not take focus back when the person has already moved on by the time the row moves', async () => {
    // A config write takes a moment. Tab on in the meantime, and the move that follows must not
    // pull the keyboard back to the switch that started it.
    const user = userEvent.setup({ delay: null });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    render(<Harness initial={initial()} gate={gate} />);

    screen.getByRole('switch', { name: 'Bravo' }).focus();
    await user.keyboard(' ');
    const charlie = screen.getByRole('switch', { name: 'Charlie' });
    charlie.focus();

    release();
    await waitFor(() => expect(heading(/^Enabled 2$/)).toBeVisible());
    expect(screen.getByRole('switch', { name: 'Charlie' })).toHaveFocus();
  });

  it('moves nothing and takes nothing when the toggle fails, and does not remember it later', async () => {
    const user = userEvent.setup({ delay: null });
    const { rerender } = render(<Harness initial={initial()} fail />);

    const bravo = screen.getByRole('switch', { name: 'Bravo' });
    bravo.focus();
    await user.keyboard(' ');
    await waitFor(() => expect(bravo).not.toBeDisabled());
    expect(heading(/^Enabled 1$/)).toBeVisible();
    expect(bravo).toHaveFocus();

    // Focus leaves the page; a row then arrives from somewhere else. A stale memory of the
    // failed toggle would pull focus back onto Bravo's switch here.
    bravo.blur();
    expect(document.body).toHaveFocus();
    rerender(<Harness initial={initial()} fail extra={entry('delta', true)} />);
    await waitFor(() => expect(heading(/^Enabled 2$/)).toBeVisible());
    expect(document.body).toHaveFocus();
  });

  it('is not held by a pointer that is over a help bubble rather than over the list', async () => {
    // The bubble is portalled out of the list but is still its React child, so a pointer on it
    // looked like a pointer in the list and froze the groups until some later pointer leave.
    const user = userEvent.setup({ delay: null });
    const example = {
      type: 'builtin',
      name: 'example',
      display_name: 'Example',
      description: 'Example',
      enabled: true,
    } as FixedExtensionEntry;
    render(<Harness initial={[example, entry('bravo', false)]} />);

    await user.tab(); // the built-in's privacy badge: Tab opens its help at once
    await waitFor(() =>
      expect(document.querySelector('[data-slot="info-tip-content"]')).not.toBeNull()
    );
    await user.pointer({ target: document.querySelector('[data-slot="info-tip-content"]')! });

    const bravo = screen.getByRole('switch', { name: 'Bravo' });
    bravo.focus();
    await user.keyboard(' ');

    // No pointer is on the list, so the row moves at once: every row is enabled now.
    await waitFor(() => expect(heading(/^Enabled 2$/)).toBeVisible());
    expect(screen.queryByRole('heading', { level: 2, name: /^Available/ })).toBeNull();
  });
});
