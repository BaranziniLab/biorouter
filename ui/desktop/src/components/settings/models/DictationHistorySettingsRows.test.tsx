import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationHistorySettings } from '../../../api';
import { DICTATION_ACTIONS } from '../../dictation/copy';
import type { DictationStatus } from '../../dictation/dictationApi';
import { speechStatus } from '../../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from '../../dictionary/copy';
import { answer } from '../../dictionary/dictionaryTestKit';
import {
  SETTINGS_DEEP_LINKS,
  SETTINGS_GROUPS,
  SETTINGS_GROUP_IDS,
  SETTINGS_SECTIONS,
  resolveSettingsDeepLink,
} from '../settingsSections';

/**
 * The dictation history's rows in **Settings > Models > Speech models** (DD-96), rendered with a
 * fake daemon: what each row offers, what it sends, when it refuses before the click, and where
 * the deep link `dictation-history` lands.
 */

const api = vi.hoisted(() => ({
  getDictationHistorySettings: vi.fn(),
  changeDictationHistorySettings: vi.fn(),
}));
vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../api')>()),
  ...api,
}));
const PROOF = { 'X-User-Action': 'proof' };
vi.mock('../../../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../utils/userAction')>()),
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
const toasts = vi.hoisted(() => ({ toastError: vi.fn() }));
vi.mock('../../../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../toasts')>()),
  toastError: toasts.toastError,
}));

// Settings itself, for the deep link: every section but Models is a stub, and Models is this
// group alone, so the test reads where `SettingsView` scrolls and what it washes.
vi.mock('./ModelsSection', async () => {
  const { default: Rows } = await import('./DictationHistorySettingsRows');
  const { speechStatus: status } = await import('../../dictation/dictationTestKit');
  return { default: () => <Rows status={status()} /> };
});
vi.mock('../chat/ChatSettingsSection', () => ({ default: () => <div /> }));
vi.mock('../app/AppSettingsSection', () => ({ default: () => <div /> }));
vi.mock('../config/ConfigSettings', () => ({ default: () => <div /> }));
vi.mock('../warmSettings', () => ({ useWarmSettings: () => undefined }));
vi.mock('../../privacy/disclosureCopy', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../privacy/disclosureCopy')>()),
  readDisclosureAcknowledgement: async () => null,
}));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: vi.fn(async () => undefined), upsert: vi.fn(async () => undefined) }),
}));

import DictationHistorySettingsRows from './DictationHistorySettingsRows';
import SettingsView from '../SettingsView';

const labels = DICTIONARY_LABELS.historySettings;
const allowed = { allowed: true, reason: null };
const HOST_SETTINGS =
  'Do this on the server: `biorouter dictation history settings <option>` (as the service user).';

function historySettings(
  over: Partial<Omit<DictationHistorySettings, 'authority'>> & {
    authority?: Partial<DictationHistorySettings['authority']>;
  } = {}
): DictationHistorySettings {
  const { authority, ...rest } = over;
  return {
    keep_history: true,
    public_by_default: false,
    ...rest,
    authority: {
      list_private: allowed,
      write: allowed,
      tier_change: allowed,
      raise_settings: allowed,
      ...authority,
    },
  };
}

const noRaise = (reason = HOST_SETTINGS) => ({ raise_settings: { allowed: false, reason } });

/** The status with `policy.history`, which the renderer's status type does not name yet. */
function withHistoryPolicy(history: { allowed: boolean; reason: string | null }): DictationStatus {
  const status = speechStatus();
  return {
    ...status,
    policy: { ...status.policy, managed: true, history } as DictationStatus['policy'],
  };
}

function renderRows(status: DictationStatus = speechStatus()) {
  return render(<DictationHistorySettingsRows status={status} />);
}

const keepSwitch = () => screen.findByRole('switch', { name: labels.keep });
const publicSwitch = () => screen.findByRole('switch', { name: labels.publicByDefault });

/** Every request the rows made, with the headers each carried. */
function everyRequest() {
  return [
    ...api.getDictationHistorySettings.mock.calls,
    ...api.changeDictationHistorySettings.mock.calls,
  ].map(([options]) => options as { headers?: Record<string, string>; body?: unknown });
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  toasts.toastError.mockReset();
  api.getDictationHistorySettings.mockResolvedValue(answer(200, historySettings()));
  api.changeDictationHistorySettings.mockImplementation(
    async ({ body }: { body: Partial<DictationHistorySettings> }) =>
      answer(200, historySettings(body))
  );
});

describe('the dictation history rows in Settings', () => {
  it('are one group the deep link lands on, headed by Dictation history, on by default', async () => {
    renderRows();
    const keep = await keepSwitch();
    const group = screen.getByTestId('settings-dictation-history');
    expect(group).toHaveAttribute('id', SETTINGS_GROUP_IDS.dictationHistory);
    expect(group.firstElementChild).toBe(screen.getByTestId('dictation-history-keep'));
    expect(keep).toHaveAttribute('aria-checked', 'true');
    expect(keep).toBeEnabled();
    expect(
      screen.getByRole('button', { name: `About ${labels.keep}` })
    ).toHaveAccessibleDescription(DICTIONARY_MESSAGES['MSG-HISTORY-HELP']);
    // Public by default is off, with its own help, under the history switch.
    const pub = await publicSwitch();
    expect(pub).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByTestId('dictation-history-keep').nextElementSibling).toBe(
      screen.getByTestId('dictation-history-public')
    );
    expect(
      screen.getByRole('button', { name: `About ${labels.publicByDefault}` })
    ).toHaveAccessibleDescription(DICTIONARY_MESSAGES['MSG-HISTORY-PUBLIC-HELP']);
  });

  it('turns the history off at once, without raise authority, and deletes nothing', async () => {
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ authority: noRaise() }))
    );
    const user = userEvent.setup();
    renderRows();
    const keep = await keepSwitch();
    expect(keep).toBeEnabled();
    await user.click(keep);
    await waitFor(() =>
      expect(api.changeDictationHistorySettings).toHaveBeenCalledWith({
        body: { keep_history: false },
        headers: PROOF,
      })
    );
    await waitFor(() => expect(keep).toHaveAttribute('aria-checked', 'false'));
    // Public by default shows only while the history is on.
    expect(screen.queryByTestId('dictation-history-public')).toBeNull();
    expect(api.changeDictationHistorySettings).toHaveBeenCalledTimes(1);
  });

  it('cannot be turned on without raise authority, and says why before the click', async () => {
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ keep_history: false, authority: noRaise() }))
    );
    const user = userEvent.setup();
    renderRows();
    const keep = await keepSwitch();
    expect(keep).toHaveAttribute('aria-checked', 'false');
    expect(keep).toBeDisabled();
    expect(screen.getByTestId('dictation-history-keep')).toHaveTextContent(
      'Do this on the server: `biorouter dictation history settings --keep on` (as the service user).'
    );
    await user.click(keep);
    await user.click(screen.getByTestId('dictation-history-keep'));
    expect(api.changeDictationHistorySettings).not.toHaveBeenCalled();
  });

  it('turns the history on with raise authority', async () => {
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ keep_history: false }))
    );
    const user = userEvent.setup();
    renderRows();
    const keep = await keepSwitch();
    expect(keep).toBeEnabled();
    expect(screen.queryByTestId('dictation-history-public')).toBeNull();
    await user.click(keep);
    await waitFor(() =>
      expect(api.changeDictationHistorySettings).toHaveBeenCalledWith({
        body: { keep_history: true },
        headers: PROOF,
      })
    );
    expect(await publicSwitch()).toHaveAttribute('aria-checked', 'false');
  });

  it('asks MSG-HISTORY-PUBLIC-CONFIRM before saving new dictations as public', async () => {
    const user = userEvent.setup();
    renderRows();
    const pub = await publicSwitch();
    await user.click(pub);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(labels.publicConfirmTitle);
    expect(dialog).toHaveTextContent(DICTIONARY_MESSAGES['MSG-HISTORY-PUBLIC-CONFIRM']);
    // Asking writes nothing, and Cancel leaves it off.
    expect(api.changeDictationHistorySettings).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole('button', { name: DICTATION_ACTIONS.cancel }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(api.changeDictationHistorySettings).not.toHaveBeenCalled();
    expect(pub).toHaveAttribute('aria-checked', 'false');

    await user.click(pub);
    const again = await screen.findByRole('dialog');
    await user.click(within(again).getByRole('button', { name: labels.publicConfirm }));
    await waitFor(() =>
      expect(api.changeDictationHistorySettings).toHaveBeenCalledWith({
        body: { public_by_default: true },
        headers: PROOF,
      })
    );
    await waitFor(() => expect(pub).toHaveAttribute('aria-checked', 'true'));
  });

  it('turns public by default off without asking, even without raise authority', async () => {
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ public_by_default: true, authority: noRaise() }))
    );
    const user = userEvent.setup();
    renderRows();
    const pub = await publicSwitch();
    expect(pub).toHaveAttribute('aria-checked', 'true');
    expect(pub).toBeEnabled();
    await user.click(pub);
    expect(screen.queryByRole('dialog')).toBeNull();
    await waitFor(() =>
      expect(api.changeDictationHistorySettings).toHaveBeenCalledWith({
        body: { public_by_default: false },
        headers: PROOF,
      })
    );
  });

  it('cannot save as public without raise authority, and asks nothing', async () => {
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ authority: noRaise() }))
    );
    const user = userEvent.setup();
    renderRows();
    const pub = await publicSwitch();
    expect(pub).toBeDisabled();
    expect(screen.getByTestId('dictation-history-public')).toHaveTextContent(
      'Do this on the server: `biorouter dictation history settings --public-by-default on` (as the service user).'
    );
    await user.click(pub);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(api.changeDictationHistorySettings).not.toHaveBeenCalled();
  });

  it('says why a refused write was refused, under the setting’s name, and reads again', async () => {
    const message = 'Biorouter could not confirm this came from you, so nothing was changed.';
    api.changeDictationHistorySettings.mockResolvedValue(answer(403, { code: 'agent', message }));
    const user = userEvent.setup();
    renderRows();
    await user.click(await keepSwitch());
    await waitFor(() =>
      expect(toasts.toastError).toHaveBeenCalledWith({ title: labels.keep, msg: message })
    );
    await waitFor(() => expect(api.getDictationHistorySettings).toHaveBeenCalledTimes(2));
    expect(await keepSwitch()).toHaveAttribute('aria-checked', 'true');
  });

  it('reads the settings again when the window takes focus', async () => {
    renderRows();
    await keepSwitch();
    expect(api.getDictationHistorySettings).toHaveBeenCalledTimes(1);
    api.getDictationHistorySettings.mockResolvedValue(
      answer(200, historySettings({ keep_history: false }))
    );
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(api.getDictationHistorySettings).toHaveBeenCalledTimes(2));
    await waitFor(async () => expect(await keepSwitch()).toHaveAttribute('aria-checked', 'false'));
  });

  it('sends the user action proof on every request it makes', async () => {
    const user = userEvent.setup();
    renderRows();
    await user.click(await keepSwitch());
    await waitFor(() => expect(api.changeDictationHistorySettings).toHaveBeenCalled());
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(api.getDictationHistorySettings).toHaveBeenCalledTimes(2));
    const requests = everyRequest();
    expect(requests.length).toBeGreaterThanOrEqual(3);
    for (const request of requests) expect(request.headers).toEqual(PROOF);
  });

  it('is absent, and reads nothing, where managed policy turns the history off', async () => {
    renderRows(withHistoryPolicy({ allowed: false, reason: 'Turned off by your organization.' }));
    await act(async () => undefined);
    expect(screen.queryByTestId('settings-dictation-history')).toBeNull();
    expect(screen.queryByRole('switch', { name: labels.keep })).toBeNull();
    expect(screen.queryByRole('switch', { name: labels.publicByDefault })).toBeNull();
    expect(api.getDictationHistorySettings).not.toHaveBeenCalled();
  });

  it('shows where the policy allows the history, or a daemon names no history policy', async () => {
    const first = renderRows(withHistoryPolicy(allowed));
    expect(await keepSwitch()).toBeInTheDocument();
    first.unmount();
    renderRows(speechStatus());
    expect(await keepSwitch()).toBeInTheDocument();
  });

  it('is absent against a daemon with no history routes', async () => {
    api.getDictationHistorySettings.mockResolvedValue(answer(404, { message: 'Not Found' }));
    renderRows();
    await waitFor(() => expect(api.getDictationHistorySettings).toHaveBeenCalled());
    await act(async () => undefined);
    expect(screen.queryByTestId('settings-dictation-history')).toBeNull();
  });
});

describe('the deep link dictation-history', () => {
  const scrollIntoView = vi.fn();

  beforeEach(() => {
    scrollIntoView.mockReset();
    Element.prototype.scrollIntoView = scrollIntoView;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('resolves to the group on the Models tab, a key of its own beside the sections', () => {
    expect(resolveSettingsDeepLink('dictation-history')).toEqual({
      tab: 'models',
      sectionId: 'settings-dictation-history',
    });
    expect(SETTINGS_GROUP_IDS.dictationHistory).toBe('settings-dictation-history');
    // A group, not a section and not a legacy link: neither pinned set moves.
    expect(Object.prototype.hasOwnProperty.call(SETTINGS_DEEP_LINKS, 'dictation-history')).toBe(
      false
    );
    const sectionIds = Object.values(SETTINGS_SECTIONS).map((section) => section.id);
    for (const group of Object.values(SETTINGS_GROUPS)) {
      expect(sectionIds).not.toContain(group.id);
      expect(group.id).toMatch(/^settings-[a-z-]+$/);
    }
  });

  it('scrolls the group into view and washes its first row', async () => {
    vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'setTimeout', 'clearTimeout'] });
    render(
      <SettingsView
        onClose={() => {}}
        setView={() => {}}
        viewOptions={{ section: 'dictation-history' }}
      />
    );
    expect(screen.getByTestId('settings-models-tab')).toHaveAttribute('data-state', 'active');
    // The rows wait for the daemon's answer, and the link keeps looking until they show.
    await act(async () => {
      vi.advanceTimersByTime(50);
    });
    expect(scrollIntoView).not.toHaveBeenCalled();
    let group: HTMLElement | null = null;
    await act(async () => {
      group = await vi.waitFor(() => screen.getByTestId('settings-dictation-history'));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
    });
    expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ block: 'start' }));
    expect(scrollIntoView.mock.contexts[0]).toBe(group);
    expect(screen.getByTestId('dictation-history-keep')).toHaveClass('br-highlight');
  });
});
