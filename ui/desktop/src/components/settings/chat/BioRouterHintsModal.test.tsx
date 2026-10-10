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
    expect(dialog).toHaveTextContent('/work/proj/.biorouterhints');

    fireEvent.change(field, { target: { value: 'Use R 4.5.' } });
    fireEvent.click(screen.getByRole('button', { name: hintsDialogCopy.save }));
    await waitFor(() =>
      expect(electron.writeFile).toHaveBeenCalledWith('/work/proj/.biorouterhints', 'Use R 4.5.')
    );
    expect(await screen.findByRole('status')).toHaveTextContent(hintsDialogCopy.saved);
  });

  it('says so when the file cannot be read, instead of offering an empty editor', async () => {
    electron.readFile.mockResolvedValue({ file: '', found: true, error: 'EACCES' });
    render(<BioRouterHintsModal directory="/work/proj" setIsBioRouterHintsModalOpen={vi.fn()} />);
    expect(await screen.findByRole('alert')).toHaveTextContent(hintsDialogCopy.readError('EACCES'));
    expect(screen.queryByRole('textbox')).toBeNull();
  });
});
