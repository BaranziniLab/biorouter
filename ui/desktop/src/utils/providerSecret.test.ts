import { beforeEach, describe, expect, it, vi } from 'vitest';
import { loadProviderSecretPreview, removeProviderSecret } from './providerSecret';

const mocks = vi.hoisted(() => ({
  preview: vi.fn(),
  remove: vi.fn(),
  read: vi.fn(),
  headers: vi.fn(),
}));
vi.mock('../api', () => ({
  previewProviderSecret: mocks.preview,
  removeConfig: mocks.remove,
  readConfig: mocks.read,
}));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.headers }));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.headers.mockResolvedValue({ 'X-User-Action': 'synthetic-user-action' });
});

describe('saved provider key preview', () => {
  it('sends user-action proof and accepts only a masked preview', async () => {
    mocks.preview.mockResolvedValue({ data: 'synt••••••••-key' });
    await expect(loadProviderSecretPreview('google', 'GOOGLE_API_KEY')).resolves.toBe(
      'synt••••••••-key'
    );
    expect(mocks.preview).toHaveBeenCalledWith({
      body: { provider: 'google', key: 'GOOGLE_API_KEY' },
      headers: { 'X-User-Action': 'synthetic-user-action' },
      throwOnError: true,
    });
  });

  it.each([undefined, 'synthetic-full-key', 'short', 'synt••••••••too-long'])(
    'refuses an unmasked or malformed preview',
    async (data) => {
      mocks.preview.mockResolvedValue({ data });
      await expect(loadProviderSecretPreview('google', 'GOOGLE_API_KEY')).rejects.toThrow();
    }
  );

  it('accepts the fully masked form for short keys', async () => {
    mocks.preview.mockResolvedValue({ data: '••••••••' });
    await expect(loadProviderSecretPreview('google', 'GOOGLE_API_KEY')).resolves.toBe('••••••••');
  });
});

describe('removing a saved provider key', () => {
  it('deletes only the named secret and verifies it is absent', async () => {
    mocks.remove.mockResolvedValue({ data: 'removed' });
    mocks.read.mockResolvedValue({ data: null });
    await expect(removeProviderSecret('GOOGLE_API_KEY')).resolves.toBeUndefined();
    expect(mocks.remove).toHaveBeenCalledWith({
      body: { key: 'GOOGLE_API_KEY', is_secret: true },
      headers: { 'X-User-Action': 'synthetic-user-action' },
      throwOnError: true,
    });
    expect(mocks.read).toHaveBeenCalledWith({
      body: { key: 'GOOGLE_API_KEY', is_secret: true },
      throwOnError: true,
    });
  });

  it('does not claim the key is removed when the environment still supplies it', async () => {
    mocks.remove.mockResolvedValue({ data: 'removed' });
    mocks.read.mockResolvedValue({ data: { maskedValue: '••••••••' } });
    await expect(removeProviderSecret('GOOGLE_API_KEY')).rejects.toThrow(
      'Key is still present'
    );
  });
});
