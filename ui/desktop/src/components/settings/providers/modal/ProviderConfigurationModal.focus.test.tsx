import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../../api';
import ProviderConfigurationModal from './ProviderConfiguationModal';

/**
 * The Configure dialog opens from a row's button, not from a Dialog.Trigger, so Radix alone would
 * leave the focus on the page when it closes. The shared dialog (`ui/dialog.tsx`) now gives it
 * back to the opener for every dialog, which is why this one no longer carries a hook of its own.
 * This holds the behaviour the hook used to guarantee.
 */

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: vi.fn().mockResolvedValue(null), upsert: vi.fn(), remove: vi.fn() }),
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

function Opener() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Configure Anthropic
      </button>
      {open ? (
        <ProviderConfigurationModal provider={anthropic} onClose={() => setOpen(false)} />
      ) : null}
    </>
  );
}

describe('the Configure dialog and the keyboard', () => {
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
});
