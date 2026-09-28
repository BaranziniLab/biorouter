import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PrivacyPanel from './PrivacyPanel';
import { __resetDisclosureStoreForTests } from '../../privacy/disclosureCopy';

const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => undefined as unknown),
  getProviders: vi.fn(),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  model: { currentModel: 'gpt-5.5-2026-04-24', currentProvider: 'versa_azure' } as {
    currentModel: string | null;
    currentProvider: string | null;
  } | null,
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: vi.fn(), getProviders: mocks.getProviders }),
}));
vi.mock('../../ModelAndProviderContext', () => ({
  useOptionalModelAndProvider: () => mocks.model ?? undefined,
  useModelAndProvider: () => mocks.model,
}));
// Fixtures, not the product's sentences: the one definition lives in Rust.
vi.mock('../../../api', () => ({
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

const served = {
  title_template: '{provider} is not hosted by your institution.',
  long: 'DIALOG-COPY-MARKER about this model and this chat.',
  short: 'SHORT-MARKER',
  settings_title: 'SETTINGS-TITLE-MARKER',
  settings: 'SETTINGS-COPY-MARKER about such models.',
  acknowledged: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  __resetDisclosureStoreForTests();
  mocks.model = { currentModel: 'gpt-5.5-2026-04-24', currentProvider: 'versa_azure' };
  mocks.getPrivacyDisclosure.mockResolvedValue({ data: served });
  mocks.getProviders.mockResolvedValue([
    { name: 'versa_azure', resolved_tier: 'private', metadata: { tier: 'private' } },
    { name: 'claude_code', resolved_tier: 'public', metadata: { tier: 'public' } },
  ]);
});

// W2-PRV-14: the panel led with the dialog's copy ("Anything a chat on this
// model can reach", "switch this chat"), and a person on a private model read
// "this model" as their own.
describe('PrivacyPanel’s statement about non-private models', () => {
  it('leads with the served class-level copy, not the dialog’s', async () => {
    render(<PrivacyPanel />);
    const statement = await screen.findByTestId('non-private-model-statement');
    expect(statement).toHaveTextContent('SETTINGS-TITLE-MARKER');
    expect(statement).toHaveTextContent('SETTINGS-COPY-MARKER');
    expect(statement).not.toHaveTextContent('DIALOG-COPY-MARKER');
    // Still above the switch (DR-17 requirement 3).
    const row = screen.getByRole('switch', { name: /Privacy tiers/ }).closest('div');
    expect(statement.compareDocumentPosition(row!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('says which side of the line the current model is on', async () => {
    render(<PrivacyPanel />);
    expect(await screen.findByTestId('privacy-current-model-tier')).toHaveTextContent(
      'Your current model, gpt-5.5-2026-04-24, is private.'
    );
  });

  it('says public for a public model, and nothing while the tier is unresolved', async () => {
    mocks.model = { currentModel: 'claude-fable-5-1', currentProvider: 'claude_code' };
    const { unmount } = render(<PrivacyPanel />);
    expect(await screen.findByTestId('privacy-current-model-tier')).toHaveTextContent(
      'Your current model, claude-fable-5-1, is public.'
    );
    unmount();

    mocks.getProviders.mockRejectedValue(new Error('catalog down'));
    render(<PrivacyPanel />);
    await screen.findByTestId('non-private-model-statement');
    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    expect(screen.queryByTestId('privacy-current-model-tier')).toBeNull();
  });

  it('falls back to the dialog’s copy from a daemon that serves no settings copy', async () => {
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: { ...served, settings: undefined, settings_title: undefined },
    });
    render(<PrivacyPanel />);
    const statement = await screen.findByTestId('non-private-model-statement');
    expect(statement).toHaveTextContent('DIALOG-COPY-MARKER');
    expect(statement).toHaveTextContent('A non-private model is not hosted by your institution.');
  });
});
