import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resetDictationStoreForTests } from '../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../dictation/dictationTestKit';
import { answer, entry, listing, scan, settings } from './dictionaryTestKit';

/**
 * While the privacy master switch is off every dictionary term, a private one included,
 * shapes dictation into every chat (dictionary.md, "Privacy tiers and the bias set"). The
 * Dictionary page then shows the note every composer shows, so the person who keeps the
 * terms sees it where they keep them.
 */

const privacy = vi.hoisted(() => ({ enabled: false }));
vi.mock('../ConfigContext', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../ConfigContext')>()),
  usePrivacyTiersEnabled: () => privacy.enabled,
  usePrivacyTiersRecord: () => undefined,
}));

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  getDictionarySuggestionScan: vi.fn(),
}));
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  ...api,
}));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

import DictionaryView from './DictionaryView';
import { resetDictionaryStoreForTests } from './useDictionary';
import { resetSuggestionScanForTests } from './useSuggestionScan';

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetSuggestionScanForTests();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  api.listDictionaryEntries.mockImplementation(async () =>
    listing([entry({ written: 'EGLN1', tier: 'private' })])
  );
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.getDictionarySuggestionScan.mockResolvedValue(answer(200, scan()));
});

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={['/dictionary']}>
      <DictionaryView />
    </MemoryRouter>
  );

describe('the Dictionary page with the privacy tiers off', () => {
  it('shows the note the composers show', async () => {
    privacy.enabled = false;
    renderPage();
    await screen.findByText('EGLN1');
    expect(screen.getByTestId('privacy-tiers-off-note')).toBeInTheDocument();
  });

  it('shows no note while the tiers are on', async () => {
    privacy.enabled = true;
    renderPage();
    await screen.findByText('EGLN1');
    expect(screen.queryByTestId('privacy-tiers-off-note')).not.toBeInTheDocument();
  });
});
