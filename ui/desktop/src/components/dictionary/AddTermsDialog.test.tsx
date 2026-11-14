import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetDictationStoreForTests } from '../dictation/dictationStore';
import { fakeStatusApi, speechStatus } from '../dictation/dictationTestKit';
import { DICTIONARY_LABELS, DICTIONARY_MESSAGES } from './copy';
import { answer, listing, settings } from './dictionaryTestKit';
import { MAX_IMPORT_BYTES } from './parseTermList';

const api = vi.hoisted(() => ({
  listDictionaryEntries: vi.fn(),
  getDictionarySettings: vi.fn(),
  importDictionary: vi.fn(),
}));
vi.mock('../../api', () => api);
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
}));
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
const toasts = vi.hoisted(() => ({ toastError: vi.fn() }));
vi.mock('../../toasts', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  toastError: toasts.toastError,
}));

import AddTermsDialog, { PLAN_DEBOUNCE_MS } from './AddTermsDialog';
import DictionaryView from './DictionaryView';
import { resetDictionaryStoreForTests } from './useDictionary';

/**
 * **Paste a list…** and **Import a file…** against the daemon's limit (1 MiB, `MAX_IMPORT_BYTES`):
 * a file over it is refused by its size and never read, and text over it is refused before it
 * is sniffed, parsed or sent, so a list chosen by mistake costs one length check, not a freeze.
 */

/** A file that says it holds `size` bytes, with its read watched. */
function fileOf(name: string, text: string, size = text.length) {
  const file = new File([text], name, { type: 'text/plain' });
  const read = vi.fn(async () => text);
  Object.defineProperty(file, 'size', { value: size });
  Object.defineProperty(file, 'text', { value: read });
  return { file, read };
}

const tooLarge = DICTIONARY_MESSAGES['MSG-DICTIONARY-TOO-LARGE'];

beforeEach(() => {
  resetDictionaryStoreForTests();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  for (const fn of Object.values(api)) fn.mockReset();
  toasts.toastError.mockReset();
  api.listDictionaryEntries.mockResolvedValue(listing([]));
  api.getDictionarySettings.mockResolvedValue(answer(200, settings()));
  api.importDictionary.mockResolvedValue(
    answer(200, { new: 2, merged: 0, duplicate: 0, invalid: [] })
  );
});

afterEach(() => {
  resetDictationStoreForTests();
});

describe('the add terms dialog', () => {
  it('refuses a file over 1 MiB by its size, never reads it, and keeps the text it holds', async () => {
    render(
      <AddTermsDialog
        initial={{ content: 'UCSF', fileName: null }}
        authority={null}
        onClose={vi.fn()}
      />
    );
    const { file, read } = fileOf('everything.txt', 'x', MAX_IMPORT_BYTES + 1);
    await act(async () => {
      fireEvent.change(screen.getByTestId('dictionary-add-terms-file'), {
        target: { files: [file] },
      });
    });
    expect(await screen.findByTestId('dictionary-add-terms-file-refused')).toHaveTextContent(
      tooLarge
    );
    expect(read).not.toHaveBeenCalled();
    expect(screen.getByTestId('dictionary-add-terms-content')).toHaveValue('UCSF');
  });

  it('takes a file at the limit, and a later edit clears an earlier refusal', async () => {
    render(<AddTermsDialog authority={null} onClose={vi.fn()} />);
    const input = screen.getByTestId('dictionary-add-terms-file');
    const big = fileOf('big.txt', 'x', MAX_IMPORT_BYTES + 1);
    await act(async () => {
      fireEvent.change(input, { target: { files: [big.file] } });
    });
    expect(screen.getByTestId('dictionary-add-terms-file-refused')).toBeInTheDocument();
    const small = fileOf('terms.txt', 'UCSF\nEPAS1');
    await act(async () => {
      fireEvent.change(input, { target: { files: [small.file] } });
    });
    expect(small.read).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('dictionary-add-terms-content')).toHaveValue('UCSF\nEPAS1');
    expect(screen.queryByTestId('dictionary-add-terms-file-refused')).toBeNull();
  });

  it('refuses text over 1 MiB before it is encoded, parsed or sent', async () => {
    const encode = vi.spyOn(TextEncoder.prototype, 'encode');
    try {
      render(
        <AddTermsDialog
          initial={{ content: 'x'.repeat(MAX_IMPORT_BYTES + 1), fileName: null }}
          authority={null}
          onClose={vi.fn()}
        />
      );
      expect(screen.getByTestId('dictionary-add-terms-plan')).toHaveTextContent(tooLarge);
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, PLAN_DEBOUNCE_MS + 50));
      });
      expect(api.importDictionary).not.toHaveBeenCalled();
      expect(encode).not.toHaveBeenCalled();
      expect(screen.getByTestId('dictionary-add-terms-apply')).toBeDisabled();
    } finally {
      encode.mockRestore();
    }
  });

  it('asks the daemon for a dry run of a list under the limit', async () => {
    render(
      <AddTermsDialog
        initial={{ content: 'UCSF\nEPAS1', fileName: null }}
        authority={null}
        onClose={vi.fn()}
      />
    );
    await waitFor(() => expect(api.importDictionary).toHaveBeenCalledTimes(1));
    expect(api.importDictionary.mock.calls[0][0].body).toMatchObject({
      format: 'lines',
      content: 'UCSF\nEPAS1',
      dry_run: true,
    });
    expect(
      await screen.findByText('2 new · 0 already in your dictionary · 0 skipped')
    ).toBeInTheDocument();
  });
});

describe('Import a file… on the page', () => {
  it('refuses a file over 1 MiB by its size and opens no dialog', async () => {
    render(
      <MemoryRouter initialEntries={['/dictionary']}>
        <DictionaryView />
      </MemoryRouter>
    );
    await screen.findByText(DICTIONARY_MESSAGES['MSG-EMPTY']);
    const { file, read } = fileOf('everything.csv', 'x', MAX_IMPORT_BYTES + 1);
    await act(async () => {
      fireEvent.change(screen.getByTestId('dictionary-import-input'), {
        target: { files: [file] },
      });
    });
    expect(toasts.toastError).toHaveBeenCalledWith({
      title: DICTIONARY_LABELS.importFile,
      msg: tooLarge,
    });
    expect(read).not.toHaveBeenCalled();
    expect(screen.queryByTestId('dictionary-add-terms-dialog')).toBeNull();
  });
});
