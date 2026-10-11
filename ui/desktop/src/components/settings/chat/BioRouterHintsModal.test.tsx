import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BioRouterHintsModal } from './BioRouterHintsModal';

afterEach(() => vi.restoreAllMocks());

describe('saving project hints', () => {
  it.each([false, true])('reports the actual file write result: %s', async (written) => {
    const writeFile = vi.fn().mockResolvedValue(written);
    vi.spyOn(console, 'error').mockImplementation(() => {});
    window.electron = {
      readFile: vi.fn().mockResolvedValue({ file: 'original', found: false, error: '' }),
      writeFile,
    } as unknown as typeof window.electron;
    render(<BioRouterHintsModal directory="/tmp/project" setIsBioRouterHintsModalOpen={vi.fn()} />);
    fireEvent.change(await screen.findByDisplayValue('original'), {
      target: { value: 'updated' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(writeFile).toHaveBeenCalledWith('/tmp/project/.biorouterhints', 'updated')
    );
    if (written) {
      expect(await screen.findByText('Saved')).toBeInTheDocument();
      expect(screen.getByText('Found')).toBeInTheDocument();
    } else {
      expect(await screen.findByText(/Failed to save .biorouterhints file/)).toBeInTheDocument();
      expect(screen.queryByText('Saved')).not.toBeInTheDocument();
      expect(screen.queryByText('Found')).not.toBeInTheDocument();
    }
  });
});
