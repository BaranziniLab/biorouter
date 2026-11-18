import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { SecretInput } from './secret-input';

function SavedSecret({ load }: { load: () => Promise<string> }) {
  const [value, setValue] = useState('');
  return (
    <SecretInput
      aria-label="API key"
      revealLabel="API key"
      value={value}
      onChange={(event) => setValue(event.target.value)}
      placeholder="••••••••"
      loadSavedValue={load}
    />
  );
}

describe('a saved secret input', () => {
  it('keeps a newly typed value when a previous reveal finishes late', async () => {
    let complete!: (value: string) => void;
    const load = vi.fn(() => new Promise<string>((resolve) => (complete = resolve)));
    render(<SavedSecret load={load} />);
    const input = screen.getByLabelText('API key');

    fireEvent.click(screen.getByRole('button', { name: 'Show API key' }));
    fireEvent.change(input, { target: { value: 'synthetic-new-key' } });
    await act(async () => complete('synthetic-old-key'));
    expect(input).toHaveValue('synthetic-new-key');
    expect(input).toHaveAttribute('type', 'password');

    fireEvent.click(screen.getByRole('button', { name: 'Show API key' }));
    expect(input).toHaveAttribute('type', 'text');
    expect(input).toHaveValue('synthetic-new-key');
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('reports a failed reveal without exposing the returned error', async () => {
    const load = vi.fn().mockRejectedValue(new Error('synthetic-sensitive-error'));
    render(<SavedSecret load={load} />);
    fireEvent.click(screen.getByRole('button', { name: 'Show API key' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not show the saved key');
    expect(screen.queryByText(/synthetic-sensitive-error/)).toBeNull();
    expect(screen.getByLabelText('API key')).toHaveAttribute('type', 'password');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Show API key' })).toBeEnabled());
  });
});
