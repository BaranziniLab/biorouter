import { beforeEach, describe, expect, it, vi } from 'vitest';

const { mockReadConfig } = vi.hoisted(() => ({ mockReadConfig: vi.fn() }));
vi.mock('../api', () => ({ readConfig: mockReadConfig }));

import {
  CALLER_PROVIDER_HEADER,
  forgetHostConfiguredProvider,
  resetHostProviderForTests,
  userActionHeaders,
} from './userAction';
import { announceAppModelSelection } from './sessionBindingSync';
import { BROWSER_SURFACE_MARKER } from './surface';

/**
 * rq2-091 (S-13 finding 3): a `biorouter serve` page that switched the host's default from
 * Baseten back to Versa kept stating Baseten as `X-Caller-Provider`, so the daemon read the page
 * as public and refused every private chat until a reload. A host-provider read that left before
 * the switch came back after `forgetHostConfiguredProvider()` and cached the old default.
 */
type ReadAnswer = { data: string };

/** A `readConfig` call held open until the test answers it. */
function heldRead(): (answer: ReadAnswer) => void {
  let answer: (value: ReadAnswer) => void = () => {};
  mockReadConfig.mockImplementationOnce(
    () =>
      new Promise<ReadAnswer>((resolve) => {
        answer = resolve;
      })
  );
  return (value) => answer(value);
}

beforeEach(() => {
  vi.clearAllMocks();
  mockReadConfig.mockReset();
  resetHostProviderForTests();
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
});

describe('the host provider a browser page states', () => {
  it('a_read_that_started_before_forget_does_not_recache_the_old_provider', async () => {
    const answerStaleRead = heldRead();
    const inFlight = userActionHeaders();

    // The page's own default switch lands while that read is out.
    forgetHostConfiguredProvider();
    mockReadConfig.mockResolvedValue({ data: 'versa_azure' });
    answerStaleRead({ data: 'baseten' });

    // Neither the request that was waiting nor the next one states the old default.
    expect(await inFlight).not.toEqual({ [CALLER_PROVIDER_HEADER]: 'baseten' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });
  });

  it('a_read_in_flight_when_another_tab_announces_a_switch_is_not_cached_either', async () => {
    mockReadConfig.mockResolvedValueOnce({ data: 'versa_azure' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });

    // A switch in this tab dropped the cache; a read goes out; another tab then switches again.
    forgetHostConfiguredProvider();
    const answerStaleRead = heldRead();
    const inFlight = userActionHeaders();
    announceAppModelSelection();
    mockReadConfig.mockResolvedValue({ data: 'anthropic' });
    answerStaleRead({ data: 'baseten' });

    expect(await inFlight).not.toEqual({ [CALLER_PROVIDER_HEADER]: 'baseten' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'anthropic' });
  });

  it('still caches a read that nothing interrupted', async () => {
    mockReadConfig.mockResolvedValue({ data: 'versa_azure' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });
    expect(mockReadConfig).toHaveBeenCalledTimes(1);
  });
});
