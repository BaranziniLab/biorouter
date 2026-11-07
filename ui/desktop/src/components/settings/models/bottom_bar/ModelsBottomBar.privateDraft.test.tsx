import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { PendingChatModelContext } from '../pendingChatModel';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';
import { markPrivateDraft, resetPrivateDraftsForTests } from '../../../dictation/privateDraft';

/**
 * DD-71, MSG-PRIVATE-DRAFT-SWITCH: an unsent chat whose draft holds text dictated with
 * private terms eligible asks before its pick moves it to a model whose instance tier is
 * not private, and the person's say so proceeds. The dialog is stubbed: what matters is
 * what the chip does with the pick the dialog hands it.
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => '' as unknown),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  switchModelProps: [] as Record<string, unknown>[],
}));

vi.mock('../../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getPrivacyDisclosure: mocks.getPrivacyDisclosure,
  ackPrivacyDisclosure: mocks.ackPrivacyDisclosure,
}));
vi.mock('../../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));
vi.mock('../../../ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, getProviders: mocks.getProviders }),
  usePrivacyTiersEnabled: () => true,
}));
vi.mock('../../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    getCurrentModelAndProviderForDisplay: async () => ({ model: 'gpt-5.5', provider: 'Versa' }),
    getCurrentModelDisplayName: async () => 'gpt-5.5',
    getCurrentProviderDisplayName: async () => 'Versa',
  }),
}));
vi.mock('../subcomponents/SwitchModelModal', () => ({
  SwitchModelModal: (props: Record<string, unknown>) => {
    mocks.switchModelProps.push(props);
    return <div data-testid="switch-model-modal" />;
  },
}));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const row = (name: string, tier: 'public' | 'private') => ({
  name,
  is_configured: true,
  provider_type: 'Builtin',
  metadata: { name, display_name: name, tier, runs_locally: false },
  affiliation: null,
  resolved_tier: tier,
});

const choose = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  mocks.switchModelProps = [];
  resetPrivateDraftsForTests();
  __resetDisclosureStoreForTests();
  mocks.getProviders.mockResolvedValue([row('versa_azure', 'private'), row('openai', 'public')]);
  mocks.getPrivacyDisclosure.mockResolvedValue({
    data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
  });
});

afterEach(() => resetPrivateDraftsForTests());

async function pickFor(model: { name: string; provider: string }) {
  render(
    <PendingChatModelContext.Provider value={{ choose, tabId: 'tab-1' }}>
      <ModelsBottomBar sessionId={null} setView={vi.fn()} dictationDraftKey="tab:new" />
    </PendingChatModelContext.Provider>
  );
  await screen.findByRole('button', { name: /Current model:/ });
  fireEvent.pointerDown(screen.getByLabelText(/Current model/), { button: 0, ctrlKey: false });
  fireEvent.click(await screen.findByRole('menuitem', { name: /Change model/ }));
  await screen.findByTestId('switch-model-modal');
  const props = mocks.switchModelProps[mocks.switchModelProps.length - 1];
  const onChoose = props.onChooseForUnsentChat as (model: unknown) => void;
  await act(async () => onChoose(model));
}

describe('switching an unsent chat whose draft was dictated with private terms', () => {
  it('asks before a public model, and switches on Switch anyway', async () => {
    markPrivateDraft('tab:new');
    await pickFor({ name: 'gpt-5.6-sol', provider: 'openai' });
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(
      'If you switch this chat to gpt-5.6-sol, a public model, the draft goes there with your next message. Switch anyway?'
    );
    expect(choose).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Switch anyway' }));
    expect(choose).toHaveBeenCalledWith({ name: 'gpt-5.6-sol', provider: 'openai' });
  });

  it('keeps the model when the person cancels', async () => {
    markPrivateDraft('tab:new');
    await pickFor({ name: 'gpt-5.6-sol', provider: 'openai' });
    fireEvent.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' })
    );
    expect(choose).not.toHaveBeenCalled();
  });

  it('does not ask for a private model', async () => {
    markPrivateDraft('tab:new');
    await pickFor({ name: 'gpt-5.2', provider: 'versa_azure' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(choose).toHaveBeenCalledTimes(1);
  });

  it('does not ask when the draft holds no privately dictated text', async () => {
    await pickFor({ name: 'gpt-5.6-sol', provider: 'openai' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(choose).toHaveBeenCalledTimes(1);
  });
});
