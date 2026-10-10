import { beforeEach, describe, expect, it, vi } from 'vitest';
import { addExtensionFromDeepLink } from './deeplink';
import { toastService } from '../../../toasts';

vi.mock('../../../toasts', () => ({
  toastService: {
    handleError: vi.fn((_title: string, message: string, options?: { shouldThrow?: boolean }) => {
      if (options?.shouldThrow) throw new Error(message);
    }),
    success: vi.fn(),
  },
}));

/**
 * The refusals a link can earn. A refusal stays on screen (it is an error, so the copy rules keep
 * it visible), and its title is sentence case like every other title in the app.
 */
describe('addExtensionFromDeepLink refusals', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const add = vi.fn(async () => undefined);
  const setView = vi.fn();

  const titleOfRefusal = async (url: string) => {
    await expect(addExtensionFromDeepLink(url, add, setView)).rejects.toThrow();
    expect(toastService.handleError).toHaveBeenCalledTimes(1);
    expect(add).not.toHaveBeenCalled();
    return vi.mocked(toastService.handleError).mock.calls[0][0];
  };

  it('names a link that is not a biorouter:// link', async () => {
    expect(await titleOfRefusal('https://example.com/extension?name=x&cmd=npx')).toBe(
      'Invalid protocol'
    );
  });

  it('names a link without a name', async () => {
    expect(await titleOfRefusal('biorouter://extension?cmd=npx')).toBe('Missing field');
  });

  it('names a command that is not on the allowed list', async () => {
    expect(await titleOfRefusal('biorouter://extension?name=x&cmd=rm&arg=-rf')).toBe(
      'Invalid command'
    );
  });

  it('names an npx call that would run a string as code', async () => {
    expect(await titleOfRefusal('biorouter://extension?name=x&cmd=npx&arg=-c&arg=whoami')).toBe(
      'Security risk'
    );
  });
});
