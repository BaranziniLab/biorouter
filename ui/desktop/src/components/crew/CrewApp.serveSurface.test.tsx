import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import type { Mock } from 'vitest';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import CrewApp from './CrewApp';
import { crewHttp, crewRequest, observeCrew } from './crewApi';
import { crewNeedsDesktopCopy } from './CrewNeedsDesktop';
import { installResizeObserverStub } from './test/crewTestUtils';

vi.mock('./crewApi', async () => {
  const actual = await vi.importActual<typeof import('./crewApi')>('./crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../ConfigContext')>('../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('./CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * CROSSCUT-5: a browser opened with `biorouter serve` offered all of Crew, and the daemon there
 * refuses every Crew request (it holds no key to check a person's proof against, SD-7). The page
 * asked for the saved connections, failed, and every control after that failed the same way. The
 * route now says up front that Crew needs the desktop app, and asks the daemon nothing.
 */

const mocked = {
  crewHttp: crewHttp as unknown as Mock,
  crewRequest: crewRequest as unknown as Mock,
  observeCrew: observeCrew as unknown as Mock,
};

function renderRoute() {
  return render(
    <MemoryRouter initialEntries={['/crew']}>
      <CrewApp />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.crewHttp.mockImplementation(async (path: string) =>
    path === '/connections' ? { connections: [] } : {}
  );
  mocked.crewRequest.mockResolvedValue({});
  mocked.observeCrew.mockResolvedValue('terminal');
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('the Crew route on each surface (CROSSCUT-5)', () => {
  it('in a serve browser, says Crew needs the desktop app, offers nothing and asks nothing', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderRoute();
    expect(screen.getByRole('heading', { name: crewNeedsDesktopCopy.title })).toBeInTheDocument();
    expect(screen.getByText(crewNeedsDesktopCopy.body)).toBeInTheDocument();
    expect(screen.queryAllByRole('button')).toEqual([]);
    expect(screen.queryAllByRole('textbox')).toEqual([]);
    expect(mocked.crewHttp).not.toHaveBeenCalled();
    expect(mocked.crewRequest).not.toHaveBeenCalled();
    expect(mocked.observeCrew).not.toHaveBeenCalled();
  });

  it('on the desktop, mounts Crew as before', async () => {
    renderRoute();
    await waitFor(() =>
      expect(mocked.crewHttp.mock.calls.some(([path]) => path === '/connections')).toBe(true)
    );
    expect(screen.queryByText(crewNeedsDesktopCopy.title)).toBeNull();
  });
});
