import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ImportSessionModal } from './ImportSessionModal';
import { IMPORT_DROP, IMPORT_INVALID_JSON, IMPORT_NOT_JSON, IMPORT_TITLE } from './copy';

function file(name: string, text: string, type = 'application/json'): File {
  const blob = new File([text], name, { type });
  // jsdom's File has no `text()` in every version; give it one.
  Object.defineProperty(blob, 'text', { value: async () => text });
  return blob;
}

function renderModal(onImport = vi.fn(async () => {})) {
  const onClose = vi.fn();
  render(<ImportSessionModal isOpen onClose={onClose} onImport={onImport} />);
  return { onImport, onClose };
}

describe('ImportSessionModal', () => {
  it('is titled, and its drop target is a button the keyboard can reach', () => {
    renderModal();
    expect(screen.getByRole('dialog', { name: IMPORT_TITLE })).toBeInTheDocument();
    const target = screen.getByRole('button', { name: new RegExp(IMPORT_DROP) });
    expect(target.tagName).toBe('BUTTON');
  });

  it('imports a dropped JSON file and closes', async () => {
    const { onImport, onClose } = renderModal();
    fireEvent.drop(screen.getByTestId('import-drop-target'), {
      dataTransfer: { files: [file('chat.json', '{"id":"x"}')] },
    });
    await waitFor(() => expect(onImport).toHaveBeenCalledWith('{"id":"x"}'));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('refuses a file that is not JSON, in words', async () => {
    const { onImport } = renderModal();
    fireEvent.drop(screen.getByTestId('import-drop-target'), {
      dataTransfer: { files: [file('notes.txt', 'hello', 'text/plain')] },
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(IMPORT_NOT_JSON);
    expect(onImport).not.toHaveBeenCalled();
  });

  it('says so when the JSON does not parse', async () => {
    const { onImport } = renderModal();
    fireEvent.drop(screen.getByTestId('import-drop-target'), {
      dataTransfer: { files: [file('chat.json', '{not json')] },
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(IMPORT_INVALID_JSON);
    expect(onImport).not.toHaveBeenCalled();
  });
});
