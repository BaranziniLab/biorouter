import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DICTATION_ACTIONS, SETUP_SUGGEST_LAST_SENTENCE } from '../../dictation/copy';
import { speechStatus } from '../../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from '../../dictionary/copy';
import { answer, listing, scan, settings } from '../../dictionary/dictionaryTestKit';
import { SETTINGS_SECTION_IDS } from '../settingsSections';

/**
 * The dictionary's rows in **Settings > Models > Speech models** (owner, 2026-11-19), rendered
 * with a fake daemon: what each row offers, what it sends, and when it refuses before the click.
 * They moved here from the Dictionary page's foot, so these are that page's settings tests,
 * kept with the controls.
 */

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  changeDictionarySettings: vi.fn(),
  activateDictionary: vi.fn(),
  getDictionarySuggestionScan: vi.fn(),
  startDictionarySuggestionScan: vi.fn(),
  stopDictionarySuggestionScan: vi.fn(),
}));
vi.mock('../../../api', () => api);
vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));

import DictionarySettingsRows from './DictionarySettingsRows';
import { resetDictionaryStoreForTests } from '../../dictionary/useDictionary';
import { resetSuggestionScanForTests } from '../../dictionary/useSuggestionScan';

const SUGGEST = DICTATION_ACTIONS.suggestFromChats;
const LEARN = DICTIONARY_LABELS.agentSettings.learnFromChats;
const noRaise = (reason = 'Use the host command.') => ({
  ...settings().authority,
  raise_settings: { allowed: false, reason },
});

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetSuggestionScanForTests();
  for (const fn of Object.values(api)) fn.mockReset();
  api.listDictionaryEntries.mockImplementation(async () => listing([]));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.changeDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.activateDictionary.mockResolvedValue(answer(200, settings({ suggest_from_chats: true })));
  api.getDictionarySuggestionScan.mockResolvedValue(answer(200, scan()));
});

function renderRows({
  activationChoiceShown = false,
  status = speechStatus(),
}: { activationChoiceShown?: boolean; status?: ReturnType<typeof speechStatus> } = {}) {
  return render(
    <DictionarySettingsRows status={status} activationChoiceShown={activationChoiceShown} />
  );
}

describe('the dictionary rows in Settings', () => {
  it('are one group the dictionary deep link lands on, headed by Edit your dictionary', async () => {
    renderRows();
    const group = screen.getByTestId('settings-dictionary');
    expect(group).toHaveAttribute('id', SETTINGS_SECTION_IDS.dictionary);
    expect(group.firstElementChild).toBe(screen.getByTestId('speech-dictionary'));
    expect(
      within(screen.getByTestId('speech-dictionary')).getByRole('button', {
        name: DICTIONARY_LABELS.editYourDictionary,
      })
    ).toBeInTheDocument();
    // In order: the agent's terms, the capability, the chat extractor.
    const agent = await screen.findByTestId('dictionary-agent-writes-row');
    const capability = screen.getByTestId('dictionary-suggest-from-chats');
    const learn = screen.getByTestId('speech-learn-from-chats');
    expect(screen.getByTestId('speech-dictionary').nextElementSibling).toBe(agent);
    expect(agent.nextElementSibling).toBe(capability);
    expect(capability.nextElementSibling).toBe(learn);
  });

  it('gives each row its help from the message catalog', async () => {
    renderRows();
    await screen.findByTestId('dictionary-agent-writes-row');
    for (const [label, help] of [
      [DICTIONARY_LABELS.agentSettings.title, DICTIONARY_MESSAGES['MSG-AGENT-WRITES-HELP']],
      [LEARN, DICTIONARY_MESSAGES['MSG-LEARN-FROM-CHATS-HELP']],
      [
        SUGGEST,
        `The agent can propose terms it sees in your chats. ${SETUP_SUGGEST_LAST_SENTENCE.on_request}`,
      ],
    ] as const) {
      expect(
        screen.getByRole('button', { name: `About ${label}` }),
        label
      ).toHaveAccessibleDescription(help);
    }
  });

  it('is absent, and reads nothing, where managed policy turns the dictionary off', () => {
    renderRows({
      status: speechStatus({
        policy: {
          ...speechStatus().policy,
          managed: true,
          dictionary: { allowed: false, reason: null },
        },
      }),
    });
    expect(screen.queryByTestId('settings-dictionary')).toBeNull();
    expect(api.getDictionarySettings).not.toHaveBeenCalled();
  });

  it('keeps its options away while the daemon has not answered', async () => {
    api.getDictionarySettings.mockImplementation(() => new Promise(() => {}));
    renderRows();
    expect(screen.getByTestId('speech-dictionary')).toBeInTheDocument();
    await waitFor(() => expect(api.getDictionarySettings).toHaveBeenCalled());
    expect(screen.queryByTestId('dictionary-agent-writes')).toBeNull();
    expect(screen.queryByRole('switch')).toBeNull();
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('keeps every option closed when the settings answer carries no authority', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(200, { ...settings({ learn_from_chats: true }), authority: null })
    );
    renderRows();
    const mode = await screen.findByTestId('dictionary-agent-writes');
    expect(mode).toBeDisabled();
    expect(screen.getByRole('switch', { name: SUGGEST })).toBeDisabled();
    expect(screen.getByRole('switch', { name: LEARN })).toBeDisabled();
    expect(screen.queryByTestId('dictionary-scan')).toBeNull();
    fireEvent.click(screen.getByRole('switch', { name: LEARN }));
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('leaves the capability to the first install’s own choice while that shows', async () => {
    renderRows({ activationChoiceShown: true });
    await screen.findByTestId('dictionary-agent-writes-row');
    expect(screen.queryByRole('switch', { name: SUGGEST })).toBeNull();
    expect(screen.getByRole('switch', { name: LEARN })).toBeInTheDocument();
  });
});

describe('Agent’s terms', () => {
  it('shows the person all three agent write modes, each with its line, and saves one', async () => {
    renderRows();
    const choice = await screen.findByTestId('dictionary-agent-writes');
    expect(choice).toBeEnabled();
    expect(choice).toHaveTextContent('Add when I ask, suggest otherwise');
    await userEvent.click(choice);
    for (const [name, hint] of [
      ['Wait for my review', DICTIONARY_LABELS.agentSettings.suggestHint],
      ['Add when I ask, suggest otherwise', DICTIONARY_LABELS.agentSettings.onRequestHint],
      ['Add without asking', DICTIONARY_LABELS.agentSettings.addHint],
    ]) {
      expect(screen.getByRole('menuitemradio', { name: new RegExp(`^${name}`) })).toHaveTextContent(
        hint
      );
    }
    await userEvent.click(screen.getByRole('menuitemradio', { name: /^Wait for my review/ }));
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { agent_writes: 'suggest' },
          headers: { 'X-User-Action': 'proof' },
        })
      )
    );
  });

  it('blocks raises but allows a lower mode when the host denies raising settings', async () => {
    api.getDictionarySettings.mockResolvedValue(answer(200, settings({ authority: noRaise() })));
    renderRows();
    const mode = await screen.findByTestId('dictionary-agent-writes');
    expect(mode).toBeEnabled();
    await userEvent.click(mode);
    const add = await screen.findByRole('menuitemradio', { name: /^Add without asking/ });
    expect(add).toHaveAttribute('data-disabled');
    fireEvent.click(add);
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('menuitemradio', { name: /^Wait for my review/ }));
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({ body: { agent_writes: 'suggest' } })
      )
    );
  });
});

describe('Suggest dictionary words from my chats', () => {
  it('turns the capability on through the activation route', async () => {
    renderRows();
    const toggle = await screen.findByRole('switch', { name: SUGGEST });
    expect(toggle).not.toBeChecked();
    await userEvent.click(toggle);
    await waitFor(() =>
      expect(api.activateDictionary).toHaveBeenCalledWith(
        expect.objectContaining({
          body: { suggest_from_chats: true },
          headers: { 'X-User-Action': 'proof' },
        })
      )
    );
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
  });

  it('cannot be turned on without raise authority, and says why before the click', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: noRaise(
            'Do this on the server: `biorouter dictionary settings <option>` (as the service user).'
          ),
        })
      )
    );
    renderRows();
    const toggle = await screen.findByRole('switch', { name: SUGGEST });
    expect(toggle).toBeDisabled();
    expect(screen.getByTestId('dictionary-suggest-from-chats')).toHaveTextContent(
      'Do this on the server: `biorouter dictionary settings --suggest-from-chats on` (as the service user).'
    );
    fireEvent.click(toggle);
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('lets a known no-raise caller turn an enabled capability off', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(200, settings({ suggest_from_chats: true, authority: noRaise() }))
    );
    renderRows();
    const toggle = await screen.findByRole('switch', { name: SUGGEST });
    expect(toggle).toBeChecked();
    expect(toggle).toBeEnabled();
    await userEvent.click(toggle);
    await waitFor(() =>
      expect(api.activateDictionary).toHaveBeenCalledWith(
        expect.objectContaining({ body: { suggest_from_chats: false } })
      )
    );
  });
});

describe('Suggest words from my messages', () => {
  it('turns on through the settings write, never the activation route', async () => {
    renderRows();
    const toggle = await screen.findByRole('switch', { name: LEARN });
    expect(toggle).not.toBeChecked();
    expect(toggle).toBeEnabled();
    await userEvent.click(toggle);
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({ body: { learn_from_chats: true } })
      )
    );
    expect(api.activateDictionary).not.toHaveBeenCalled();
  });

  it('cannot be turned on without raise authority, and says why before the click', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(
        200,
        settings({
          authority: noRaise(
            'Do this on the server: `biorouter dictionary settings <option>` (as the service user).'
          ),
        })
      )
    );
    renderRows();
    const toggle = await screen.findByRole('switch', { name: LEARN });
    expect(toggle).toBeDisabled();
    expect(screen.getByTestId('speech-learn-from-chats')).toHaveTextContent(
      'Do this on the server: `biorouter dictionary settings --learn-from-chats on` (as the service user).'
    );
    fireEvent.click(toggle);
    expect(api.changeDictionarySettings).not.toHaveBeenCalled();
  });

  it('lets a known no-raise caller turn the chat extractor off', async () => {
    api.getDictionarySettings.mockResolvedValue(
      answer(200, settings({ learn_from_chats: true, authority: noRaise() }))
    );
    renderRows();
    const toggle = await screen.findByRole('switch', { name: LEARN });
    expect(toggle).toBeChecked();
    expect(toggle).toBeEnabled();
    await userEvent.click(toggle);
    await waitFor(() =>
      expect(api.changeDictionarySettings).toHaveBeenCalledWith(
        expect.objectContaining({ body: { learn_from_chats: false } })
      )
    );
  });
});

describe('Recent chats', () => {
  const on = (overrides: Parameters<typeof settings>[0] = {}) =>
    api.getDictionarySettings.mockResolvedValue(
      answer(200, settings({ learn_from_chats: true, ...overrides }))
    );

  it('is offered only while Suggest words from my messages is on', async () => {
    renderRows();
    await screen.findByRole('switch', { name: LEARN });
    expect(screen.queryByTestId('dictionary-scan')).toBeNull();
    expect(api.getDictionarySuggestionScan).not.toHaveBeenCalled();
  });

  it('is offered right under the switch once suggestions are on, as a quiet Find words', async () => {
    on();
    renderRows();
    const start = await screen.findByTestId('dictionary-scan-start');
    expect(start).toHaveTextContent(/^Find words$/);
    expect(start).toHaveAccessibleName('Find words in my recent chats');
    expect(start).toBeEnabled();
    expect(screen.getByTestId('speech-learn-from-chats').nextElementSibling).toBe(
      screen.getByTestId('dictionary-scan')
    );
    expect(
      screen.getByRole('button', { name: `About ${DICTIONARY_LABELS.scan.row}` })
    ).toHaveAccessibleDescription(DICTIONARY_MESSAGES['MSG-SCAN-HELP']);
    // Read once on mount, so a scan another window started shows here.
    await waitFor(() => expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(1));
    expect(api.getDictionarySuggestionScan).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'proof' } })
    );
  });

  it('is not offered to a caller that may not write', async () => {
    on({
      authority: {
        ...settings().authority,
        write: { allowed: false, reason: 'This host does not allow dictionary edits.' },
      },
    });
    renderRows();
    await screen.findByRole('switch', { name: LEARN });
    expect(screen.queryByTestId('dictionary-scan-start')).toBeNull();
  });

  it('shows its progress with Stop, then how it ended, and reads the listing again', async () => {
    on();
    api.startDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'running', chats_read: 0, chats_total: 3, words_found: 1 }))
    );
    renderRows();
    const start = await screen.findByTestId('dictionary-scan-start');
    await waitFor(() => expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(1));
    api.getDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'finished', chats_read: 3, chats_total: 3, words_found: 2 }))
    );
    const listed = api.listDictionaryEntries.mock.calls.length;
    fireEvent.click(start);
    const status = screen.getByTestId('dictionary-scan-status');
    await waitFor(() =>
      expect(status).toHaveTextContent('Reading chat 1 of 3, 1 word found so far')
    );
    expect(status).toHaveAttribute('aria-live', 'polite');
    expect(status).toHaveClass('text-text-muted');
    expect(screen.queryByTestId('dictionary-scan-start')).toBeNull();
    expect(screen.getByTestId('dictionary-scan-stop')).toHaveTextContent('Stop');
    expect(api.startDictionarySuggestionScan).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'proof' } })
    );
    await waitFor(
      () =>
        expect(screen.getByTestId('dictionary-scan-status')).toHaveTextContent(
          'Read 3 chats. 2 new words to review.'
        ),
      { timeout: 3000 }
    );
    expect(api.listDictionaryEntries.mock.calls.length).toBeGreaterThan(listed);
    expect(screen.getByTestId('dictionary-scan-start')).toBeEnabled();
  });

  it('stops when asked and says where it stopped', async () => {
    on();
    const running = scan({ state: 'running', chats_read: 1, chats_total: 40, words_found: 1 });
    api.startDictionarySuggestionScan.mockResolvedValue(answer(200, running));
    api.stopDictionarySuggestionScan.mockResolvedValue(answer(200, running));
    renderRows();
    const start = await screen.findByTestId('dictionary-scan-start');
    await waitFor(() => expect(api.getDictionarySuggestionScan).toHaveBeenCalledTimes(1));
    api.getDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'stopped', chats_read: 12, chats_total: 40, words_found: 2 }))
    );
    fireEvent.click(start);
    const stop = await screen.findByTestId('dictionary-scan-stop');
    expect(stop).toHaveAccessibleName('Stop');
    fireEvent.click(stop);
    await waitFor(() =>
      expect(api.stopDictionarySuggestionScan).toHaveBeenCalledWith(
        expect.objectContaining({ headers: { 'X-User-Action': 'proof' } })
      )
    );
    await waitFor(
      () =>
        expect(screen.getByTestId('dictionary-scan-status')).toHaveTextContent(
          'Stopped after 12 of 40 chats. 2 new words to review.'
        ),
      { timeout: 3000 }
    );
    expect(api.stopDictionarySuggestionScan).toHaveBeenCalledTimes(1);
  });

  it('shows the daemon’s sentence for a refused start, in muted ink', async () => {
    on();
    const message =
      'Turn on Suggest words from my messages first, then find words in your recent chats.';
    api.startDictionarySuggestionScan.mockResolvedValue(
      answer(409, { code: 'suggestions_off', message })
    );
    renderRows();
    fireEvent.click(await screen.findByTestId('dictionary-scan-start'));
    await waitFor(() =>
      expect(screen.getByTestId('dictionary-scan-status')).toHaveTextContent(message)
    );
    expect(screen.getByTestId('dictionary-scan-status')).toHaveClass('text-text-muted');
    expect(screen.getByTestId('dictionary-scan-start')).toBeEnabled();
  });

  it('shows a scan another window started, and a failure in muted ink', async () => {
    on();
    api.getDictionarySuggestionScan.mockResolvedValueOnce(
      answer(200, scan({ state: 'running', chats_read: 3, chats_total: 10, words_found: 0 }))
    );
    api.getDictionarySuggestionScan.mockResolvedValue(
      answer(200, scan({ state: 'failed', chats_read: 5, chats_total: 10, words_found: 0 }))
    );
    renderRows();
    await waitFor(() =>
      expect(screen.getByTestId('dictionary-scan-status')).toHaveTextContent(
        'Reading chat 4 of 10, 0 words found so far'
      )
    );
    await waitFor(
      () =>
        expect(screen.getByTestId('dictionary-scan-status')).toHaveTextContent(
          'Biorouter could not finish reading your chats.'
        ),
      { timeout: 3000 }
    );
    expect(screen.getByTestId('dictionary-scan-status')).toHaveClass('text-text-muted');
  });
});
