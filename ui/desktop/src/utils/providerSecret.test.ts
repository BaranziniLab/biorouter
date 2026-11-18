import { beforeEach, describe, expect, it, vi } from 'vitest';
import { loadProviderSecret } from './providerSecret';

const mocks = vi.hoisted(() => ({
  reveal: vi.fn(),
  headers: vi.fn(),
}));

vi.mock('../api', () => ({ revealProviderSecret: mocks.reveal }));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.headers }));

describe('loadProviderSecret', () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it('sends the user-action proof only with the requested provider and key', async () => {
    mocks.headers.mockResolvedValue({ 'X-User-Action': 'synthetic-user-action' });
    mocks.reveal.mockResolvedValue({ data: 'synthetic-provider-key' });

    await expect(loadProviderSecret('google', 'GOOGLE_API_KEY')).resolves.toBe(
      'synthetic-provider-key'
    );
    expect(mocks.reveal).toHaveBeenCalledWith({
      body: { provider: 'google', key: 'GOOGLE_API_KEY' },
      headers: { 'X-User-Action': 'synthetic-user-action' },
      throwOnError: true,
    });
  });

  it('refuses a response without a string value', async () => {
    mocks.headers.mockResolvedValue({});
    mocks.reveal.mockResolvedValue({ data: undefined });
    await expect(loadProviderSecret('google', 'GOOGLE_API_KEY')).rejects.toThrow();
  });
});
