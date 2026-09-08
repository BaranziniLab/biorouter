import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelsBottomBar from './ModelsBottomBar';
import { __resetDisclosureStoreForTests } from '../../../privacy/disclosureCopy';

/**
 * The composer chip with nothing bound — reachable since a user can enter the
 * app before configuring a provider ("Explore Biorouter first →").
 */

const mocks = vi.hoisted(() => ({
  read: vi.fn(async () => ''),
  getProviders: vi.fn(async () => [] as unknown[]),
  getPrivacyDisclosure: vi.fn(),
  ackPrivacyDisclosure: vi.fn(),
  setView: vi.fn(),
  currentProvider: null as string | null,
  modelConfigStatus: 'ready' as 'loading' | 'ready' | undefined,
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
    currentModel: null,
    currentProvider: mocks.currentProvider,
    modelConfigStatus: mocks.modelConfigStatus,
    getCurrentModelAndProviderForDisplay: async () => ({ model: '', provider: '' }),
    getCurrentModelDisplayName: async () => 'Select Model',
    getCurrentProviderDisplayName: async () => '',
  }),
}));
vi.mock('../../../BaseChat', () => ({ useCurrentModelInfo: () => null }));
vi.mock('../subcomponents/SwitchModelModal', () => ({ SwitchModelModal: () => null }));
vi.mock('../subcomponents/LeadWorkerSettings', () => ({ LeadWorkerSettings: () => null }));

const dropdownRef = { current: null } as unknown as React.RefObject<HTMLDivElement>;

const renderBar = () =>
  render(
    <ModelsBottomBar
      sessionId={null}
      dropdownRef={dropdownRef}
      setView={mocks.setView}
      alerts={[]}
    />
  );

describe('the model chip with no provider configured', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __resetDisclosureStoreForTests();
    mocks.currentProvider = null;
    mocks.modelConfigStatus = 'ready';
    mocks.getPrivacyDisclosure.mockResolvedValue({
      data: { title_template: '{provider}', long: 'L', short: 'S', acknowledged: true },
    });
  });

  it('reads "Choose a model" and opens the provider catalog', () => {
    renderBar();
    const chip = screen.getByTestId('model-chip-choose-model');
    expect(chip).toHaveTextContent('Choose a model');
    fireEvent.click(chip);
    expect(mocks.setView).toHaveBeenCalledWith('ConfigureProviders');
  });

  /**
   * ⚠ **The dropdown is replaced, not disabled.** Its two items are "Change
   * Model" and "Lead/Worker Settings", which both read as adjustments to a model
   * that does not exist.
   */
  it('offers no model dropdown at all in that state', () => {
    renderBar();
    expect(screen.queryByText('Current model')).toBeNull();
  });

  /**
   * ⚠ The state a naive `!currentProvider` check gets wrong: the config has not
   * been read yet, so `currentProvider` is null on every install, configured or
   * not. Telling a Versa user to choose a model for the first frames after
   * launch is worse than saying nothing.
   */
  it('says nothing while the config is still loading', () => {
    mocks.modelConfigStatus = 'loading';
    renderBar();
    expect(screen.queryByTestId('model-chip-choose-model')).toBeNull();
  });

  it('leaves a configured chip alone', () => {
    mocks.currentProvider = 'versa_azure';
    renderBar();
    expect(screen.queryByTestId('model-chip-choose-model')).toBeNull();
  });
});
