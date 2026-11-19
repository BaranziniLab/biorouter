import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as providerSecret from './providerSecret';

const { removeProviderSecret } = providerSecret;

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

describe('saved provider key display', () => {
  it('uses one fixed mask without saved-key bytes or length', () => {
    expect(providerSecret.SECRET_MASK).toBe('••••••••');
    expect(mocks.preview).not.toHaveBeenCalled();
  });

  it('has no loader export for saved or environment credential readback', () => {
    expect(providerSecret).not.toHaveProperty('loadProviderSecretPreview');
    expect(mocks.preview).not.toHaveBeenCalled();
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
