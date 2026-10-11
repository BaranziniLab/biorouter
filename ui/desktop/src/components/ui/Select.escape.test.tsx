import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { Dialog, DialogContent, DialogDescription, DialogTitle } from './dialog';
import { Select } from './Select';

/**
 * Escape closes the innermost open surface only (APG; vision round 2, V4-R2-2).
 *
 * Switch models put a Select in a dialog. With the provider menu open, one Escape removed the
 * menu AND the dialog, and the choice in progress was lost, while a Radix menu in another dialog
 * (Tool permissions) closed only itself. Radix is not wrong: a react-select menu is not a Radix
 * layer, so the dialog is the top layer, and its Escape listener runs in the CAPTURE phase on the
 * document, before react-select's own handler ever sees the key. Stopping propagation inside the
 * Select would therefore change nothing. The Select claims the key first instead (a window capture
 * listener, which runs before the document's), and the dialog leaves an Escape that is already
 * claimed alone.
 */
const PROVIDERS = [
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'ollama', label: 'Ollama' },
];

function SwitchModels({ onClose }: { onClose: () => void }) {
  const [open, setOpen] = useState(true);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) onClose();
      }}
    >
      <DialogContent>
        <DialogTitle>Switch models</DialogTitle>
        <DialogDescription>Pick a provider and a model.</DialogDescription>
        <Select aria-label="Provider" options={PROVIDERS} />
      </DialogContent>
    </Dialog>
  );
}

const combobox = () => screen.getByRole('combobox', { name: 'Provider' });

describe('Escape with a Select menu open inside a dialog', () => {
  it('closes the menu and keeps the dialog; a second Escape closes the dialog', async () => {
    const user = userEvent.setup();
    let closed = 0;
    render(<SwitchModels onClose={() => closed++} />);

    await user.click(combobox());
    await user.keyboard('{ArrowDown}');
    expect(await screen.findAllByRole('option')).toHaveLength(3);
    expect(combobox()).toHaveAttribute('aria-expanded', 'true');

    await user.keyboard('{Escape}');
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(combobox()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('dialog', { name: 'Switch models' })).toBeInTheDocument();
    expect(closed).toBe(0);
    expect(combobox()).toHaveFocus();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(closed).toBe(1);
  });

  it('keeps a typed filter’s choice in progress: Escape closes the menu, not the dialog', async () => {
    const user = userEvent.setup();
    let closed = 0;
    render(<SwitchModels onClose={() => closed++} />);

    await user.click(combobox());
    await user.keyboard('Open');
    expect(await screen.findByRole('option', { name: 'OpenAI' })).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('option')).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Switch models' })).toBeInTheDocument();
    expect(closed).toBe(0);
  });

  it('leaves a closed Select’s Escape to the dialog', async () => {
    const user = userEvent.setup();
    let closed = 0;
    render(<SwitchModels onClose={() => closed++} />);

    combobox().focus();
    expect(combobox()).toHaveAttribute('aria-expanded', 'false');
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(closed).toBe(1);
  });

  it('claims only its own Escape: another expanded combobox is not the Select’s', async () => {
    const user = userEvent.setup();
    let closed = 0;
    function Palette() {
      const [open, setOpen] = useState(true);
      return (
        <Dialog
          open={open}
          onOpenChange={(next) => {
            setOpen(next);
            if (!next) closed++;
          }}
        >
          <DialogContent>
            <DialogTitle>Commands</DialogTitle>
            <DialogDescription>A list that is always shown.</DialogDescription>
            {/* cmdk's input says expanded for as long as it is mounted; Escape must still close
                a command palette's dialog. */}
            <input role="combobox" aria-label="Search commands" aria-expanded="true" />
          </DialogContent>
        </Dialog>
      );
    }
    render(<Palette />);
    screen.getByRole('combobox', { name: 'Search commands' }).focus();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(closed).toBe(1);
  });
});
