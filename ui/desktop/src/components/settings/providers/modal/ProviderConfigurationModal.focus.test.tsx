import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import ProviderConfigurationModal from './ProviderConfiguationModal';

/**
 * The Configure dialog opens from a row's button, not from a Dialog.Trigger, so Radix alone would
 * leave the focus on the page when it closes. The shared dialog (`ui/dialog.tsx`) now gives it
 * back to the opener for every dialog, which is why this one no longer carries a hook of its own.
 * This holds the behaviour the hook used to guarantee.
 */

const mocks = vi.hoisted(() => ({ read: vi.fn() }));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: vi.fn(), remove: vi.fn() }),
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ getCurrentModelAndProvider: vi.fn() }),
}));

const anthropic = {
  name: 'anthropic',
  is_configured: false,
  provider_type: 'Builtin',
  metadata: {
    name: 'anthropic',
    display_name: 'Anthropic',
    description: '',
    default_model: '',
    known_models: [],
    model_doc_link: '',
    config_keys: [
      { name: 'ANTHROPIC_API_KEY', required: true, secret: true, oauth_flow: false, default: null },
    ],
  },
} as unknown as ProviderDetails;

function Opener({ provider = anthropic }: { provider?: ProviderDetails }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Configure Anthropic
      </button>
      {open ? (
        <ProviderConfigurationModal provider={provider} onClose={() => setOpen(false)} />
      ) : null}
    </>
  );
}

const savedAnthropic = { ...anthropic, is_configured: true } as ProviderDetails;

describe('the Configure dialog and the keyboard', () => {
  beforeEach(() => {
    mocks.read.mockReset();
    mocks.read.mockResolvedValue(null);
  });

  it('gives the focus back to the button that opened it when Cancel closes it', async () => {
    const user = userEvent.setup();
    render(<Opener />);
    const opener = screen.getByRole('button', { name: 'Configure Anthropic' });
    await user.click(opener);
    await screen.findByRole('dialog', { name: 'Configure Anthropic' });

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(opener).toHaveFocus();
  });

  it('does the same when Escape closes it', async () => {
    const user = userEvent.setup();
    render(<Opener />);
    const opener = screen.getByRole('button', { name: 'Configure Anthropic' });
    await user.click(opener);
    await screen.findByRole('dialog', { name: 'Configure Anthropic' });

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(opener).toHaveFocus();
  });

  it('opens on the key field, not on Remove, once the saved values are in', async () => {
    const user = userEvent.setup();
    render(<Opener provider={savedAnthropic} />);
    await user.click(screen.getByRole('button', { name: 'Configure Anthropic' }));
    const dialog = await screen.findByRole('dialog', { name: 'Configure Anthropic' });

    // A configured provider has a Remove button, which is the first tabbable control while the
    // fields still read "Loading…". Radix used to pick it.
    expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument();
    await waitFor(() => expect(dialog.querySelector('input[type="password"]')).not.toBeNull());
    const key = dialog.querySelector('input[type="password"]') as HTMLInputElement;
    await waitFor(() => expect(key).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Remove' })).not.toHaveFocus();
  });

  it('leaves focus where the person put it when they moved before the fields loaded', async () => {
    // Hold the saved-value read open, so the fields are provably absent while the person moves.
    let release: (value: null) => void = () => {};
    mocks.read.mockReturnValue(new Promise<null>((resolve) => (release = resolve)));
    const user = userEvent.setup();
    render(<Opener provider={savedAnthropic} />);
    await user.click(screen.getByRole('button', { name: 'Configure Anthropic' }));
    const dialog = await screen.findByRole('dialog', { name: 'Configure Anthropic' });
    expect(dialog.querySelector('input[type="password"]')).toBeNull();

    const cancel = screen.getByRole('button', { name: 'Cancel' });
    cancel.focus();
    release(null);
    await waitFor(() => expect(dialog.querySelector('input[type="password"]')).not.toBeNull());
    expect(cancel).toHaveFocus();
  });

  it('still returns focus to the opener after the key field took it', async () => {
    const user = userEvent.setup();
    render(<Opener provider={savedAnthropic} />);
    const opener = screen.getByRole('button', { name: 'Configure Anthropic' });
    await user.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Configure Anthropic' });
    await waitFor(() =>
      expect(dialog.querySelector('input[type="password"]') as HTMLElement).toHaveFocus()
    );

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(opener).toHaveFocus();
  });
});
