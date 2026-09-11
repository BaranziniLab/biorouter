import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { mockReadConfig } = vi.hoisted(() => ({ mockReadConfig: vi.fn() }));
vi.mock('../api', () => ({ readConfig: mockReadConfig }));

import {
  CALLER_PROVIDER_HEADER,
  resetHostProviderForTests,
  userActionHeaders,
} from './userAction';
import { BROWSER_SURFACE_MARKER } from './surface';

/**
 * SD-9: what each surface says on the requests the daemon's reach gate reads.
 *
 * The browser half is the one that was missing. On a `biorouter serve` daemon a
 * chat started on the host's private model is private from its first reply, and
 * the daemon reaches a private chat only for a caller whose stated capability
 * covers it — so a tab that stated nothing lost its own chat after one answer.
 */
const onBrowser = () => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
};

beforeEach(() => {
  vi.clearAllMocks();
  resetHostProviderForTests();
  delete document.documentElement.dataset.biorouterSurface;
  Object.assign(window, {
    electron: { getUserActionKey: vi.fn(async () => 'desktop-user-action-key') },
  });
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('userActionHeaders', () => {
  it('proves the person on the desktop, and states no capability', async () => {
    expect(await userActionHeaders()).toEqual({ 'X-User-Action': 'desktop-user-action-key' });
    expect(mockReadConfig).not.toHaveBeenCalled();
  });

  it("states the host's configured model in a browser, and claims no proof", async () => {
    onBrowser();
    mockReadConfig.mockResolvedValue({ data: 'versa_azure' });

    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });
    expect(mockReadConfig).toHaveBeenCalledWith({
      body: { key: 'BIOROUTER_PROVIDER', is_secret: false },
    });
    // Read once per page: the host's model cannot change under a running tab.
    await userActionHeaders();
    expect(mockReadConfig).toHaveBeenCalledTimes(1);
  });

  it('says nothing when the host model cannot be read, and asks again next time', async () => {
    onBrowser();
    mockReadConfig.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    expect(await userActionHeaders()).toEqual({});

    mockReadConfig.mockResolvedValueOnce({ data: 'versa_azure' });
    expect(await userActionHeaders()).toEqual({ [CALLER_PROVIDER_HEADER]: 'versa_azure' });
  });

  it('uses the header name the daemon reads', () => {
    const gate = readFileSync(
      join(__dirname, '../../../../crates/biorouter-server/src/routes/session_reach.rs'),
      'utf8'
    );
    expect(gate).toContain(`pub const CALLER_PROVIDER_HEADER: &str = "${CALLER_PROVIDER_HEADER}";`);
  });
});
