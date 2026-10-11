import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BioRouterHintsModal } from './BioRouterHintsModal';
import { hintsDialogCopy } from './copy';

const electron = {
  readFile: vi.fn(),
  writeFile: vi.fn(),
};

beforeEach(() => {
  electron.readFile.mockReset();
  electron.writeFile.mockReset();
  Object.assign(window, { electron });
});

describe('Project hints dialog', () => {
  it('loads the file into one labelled field and saves it back', async () => {
    electron.readFile.mockResolvedValue({ file: 'Use R 4.4.', found: true, error: null });
    electron.writeFile.mockResolvedValue(undefined);
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);

    const dialog = await screen.findByRole('dialog', { name: hintsDialogCopy.title });
    const field = await screen.findByRole('textbox', { name: hintsDialogCopy.field });
    await waitFor(() => expect(field).toHaveValue('Use R 4.4.'));
    expect(field).toHaveAccessibleDescription(hintsDialogCopy.helper);
    // The subtitle names the file in one family; the path is behind Copy path (V4-23).
    expect(dialog).toHaveTextContent(hintsDialogCopy.found);
    expect(dialog).not.toHaveTextContent('/work/proj');

    fireEvent.change(field, { target: { value: 'Use R 4.5.' } });
    fireEvent.click(screen.getByRole('button', { name: hintsDialogCopy.save }));
    await waitFor(() =>
      expect(electron.writeFile).toHaveBeenCalledWith('/work/proj/.biorouterhints', 'Use R 4.5.')
    );
    expect(await screen.findByRole('status')).toHaveTextContent(hintsDialogCopy.saved);
  });

  it('says "New .biorouterhints file" for a project that has none, and sets it in one family', async () => {
    electron.readFile.mockResolvedValue({ file: '', found: false, error: null });
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);
    const dialog = await screen.findByRole('dialog', { name: hintsDialogCopy.title });
    expect(dialog).toHaveTextContent(hintsDialogCopy.newFile);
    // Nothing in the dialog's header is mono: a path in a sans sentence was two families in one
    // line, and the file's name is plain words (principle 3 and 10).
    expect(dialog.querySelector('[class~="font-mono"]')).toBeNull();
  });

  it('copies the full path from one Copy path action, and says so', async () => {
    electron.readFile.mockResolvedValue({ file: '', found: true, error: null });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);
    await screen.findByRole('dialog', { name: hintsDialogCopy.title });

    fireEvent.click(screen.getByRole('button', { name: hintsDialogCopy.copyPath }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('/work/proj/.biorouterhints'));
    expect(await screen.findByRole('status')).toHaveTextContent(hintsDialogCopy.pathCopied);
  });

  it('says so when the path cannot be copied', async () => {
    // The clipboard path focuses the window before its one retry; jsdom has no such method.
    vi.spyOn(window, 'focus').mockImplementation(() => undefined);
    electron.readFile.mockResolvedValue({ file: '', found: true, error: null });
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
      configurable: true,
    });
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);
    await screen.findByRole('dialog', { name: hintsDialogCopy.title });

    fireEvent.click(screen.getByRole('button', { name: hintsDialogCopy.copyPath }));
    expect(await screen.findByRole('alert')).toHaveTextContent(hintsDialogCopy.copyFailed);
  });

  it('says so when the file cannot be read, instead of offering an empty editor', async () => {
    electron.readFile.mockResolvedValue({ file: '', found: true, error: 'EACCES' });
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);
    expect(await screen.findByRole('alert')).toHaveTextContent(hintsDialogCopy.readError('EACCES'));
    expect(screen.queryByRole('textbox')).toBeNull();
  });
});
