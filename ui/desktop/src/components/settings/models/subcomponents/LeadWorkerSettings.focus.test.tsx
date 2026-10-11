import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LeadWorkerSettings } from './LeadWorkerSettings';

/**
 * The dialog's models load after it mounts, so at mount its only control is the × (its body is a
 * skeleton). Radix used to put focus there and leave it there: a keyboard user opened Lead/worker
 * mode on the close control. The dialog now rests focus on its box and the first switch takes it
 * once the form is in, unless the person has already moved (`../dialogFocus.ts`).
 */

vi.mock('../predefinedModelsUtils', () => ({
  shouldShowPredefinedModels: () => false,
  getPredefinedModelsFromEnv: () => [],
}));

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  getProviders: vi.fn(),
}));

vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    upsert: vi.fn(),
    remove: vi.fn(),
    getProviders: mocks.getProviders,
  }),
}));

vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: null }),
}));

const providers = [
  {
    is_configured: true,
    name: 'openai',
    metadata: { display_name: 'OpenAI', known_models: [{ name: 'gpt-4o' }] },
  },
];

describe('LeadWorkerSettings initial focus', () => {
  beforeEach(() => {
    mocks.read.mockReset();
    mocks.read.mockResolvedValue(null);
    mocks.getProviders.mockReset();
    mocks.getProviders.mockResolvedValue(providers);
  });

  it('opens on the Lead/worker switch, not on the close control, once the form is in', async () => {
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    const dialog = await screen.findByRole('dialog');

    const toggle = await screen.findByRole('switch');
    await waitFor(() => expect(toggle).toHaveFocus());
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(screen.getByRole('button', { name: 'Close' })).not.toHaveFocus();
  });

  it('leaves focus where the person put it when they moved before the form loaded', async () => {
    // Hold the providers read open, so the skeleton is provably still showing.
    let release: (value: typeof providers) => void = () => {};
    mocks.getProviders.mockReturnValue(
      new Promise<typeof providers>((resolve) => (release = resolve))
    );
    render(<LeadWorkerSettings isOpen onClose={vi.fn()} />);
    const dialog = await screen.findByRole('dialog');
    expect(screen.queryByRole('switch')).toBeNull();

    const close = screen.getByRole('button', { name: 'Close' });
    close.focus();
    release(providers);
    await screen.findByRole('switch');
    expect(dialog.contains(close)).toBe(true);
    expect(close).toHaveFocus();
  });
});
