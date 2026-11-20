import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SavedProviderSecret } from './SavedProviderSecret';

const mocks = vi.hoisted(() => ({ preview: vi.fn(), remove: vi.fn() }));
vi.mock('../../../../../../utils/providerSecret', () => ({
  SECRET_MASK: '••••••••',
  loadProviderSecretPreview: mocks.preview,
  removeProviderSecret: mocks.remove,
}));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.preview.mockResolvedValue('synt••••••••-key');
  mocks.remove.mockResolvedValue(undefined);
});

describe('saved provider key', () => {
  it('shows one fixed read-only mask and explains local replacement without retrieval', () => {
    render(
      <SavedProviderSecret
        id="key"
        provider="google"
        secretKey="GOOGLE_API_KEY"
        label="API key"
        value=""
        onChange={vi.fn()}
        onRemoved={vi.fn()}
      />
    );
    const input = screen.getByDisplayValue('••••••••');
    expect(input).toHaveAttribute('readonly');
    expect(input).toHaveAccessibleDescription(
      'Edit to replace this saved key. The saved value is never shown.'
    );
    expect(screen.getByRole('button', { name: 'Edit API key' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: /show api key/i })).toBeNull();
    expect(mocks.preview).not.toHaveBeenCalled();
  });

  it('does not remove a key until confirmed and allows cancellation', async () => {
    const removed = vi.fn();
    render(
      <SavedProviderSecret
        provider="google"
        secretKey="GOOGLE_API_KEY"
        label="API key"
        value=""
        onChange={vi.fn()}
        onRemoved={removed}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    expect(mocks.remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(mocks.remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    await waitFor(() => expect(removed).toHaveBeenCalledOnce());
    expect(mocks.remove).toHaveBeenCalledWith('GOOGLE_API_KEY');
  });

  it('keeps the fixed saved-key mask when deletion fails', async () => {
    mocks.remove.mockRejectedValue(new Error('synthetic-sensitive-error'));
    const removed = vi.fn();
    render(
      <SavedProviderSecret
        provider="google"
        secretKey="GOOGLE_API_KEY"
        label="API key"
        value=""
        onChange={vi.fn()}
        onRemoved={removed}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not remove the key');
    expect(screen.queryByText(/synthetic-sensitive-error/)).toBeNull();
    expect(screen.getByDisplayValue('••••••••')).toHaveAttribute('readonly');
    expect(mocks.preview).not.toHaveBeenCalled();
    expect(removed).not.toHaveBeenCalled();
  });

  it('explains when the environment still supplies the key', async () => {
    mocks.remove.mockRejectedValue(new Error('Key is still present'));
    render(
      <SavedProviderSecret
        provider="google"
        secretKey="GOOGLE_API_KEY"
        label="API key"
        value=""
        onChange={vi.fn()}
        onRemoved={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove API key' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('comes from the environment');
  });
});
