import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';
import { useChatCrewAccess } from './chatCrewAccess';
import { connection, grantRow } from './testing';
import { announceGrantsChanged, forgetUnconfirmedRevocations } from './useCrewGrants';

/**
 * A browser opened with `biorouter serve` has no Crew (SD-8): the daemon refuses every Crew route
 * there without a person's proof (`crew_human_authority_unavailable`), so a chat in that browser
 * asks for nothing, as the Crew view, the attention dot and the change feed already do (W1-02).
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  mocks.crewHttp.mockImplementation(async (path: string) => {
    if (path === '/connections') return { connections: [connection] };
    if (path === '/connections/conn-1/grants')
      return { grants: [grantRow({ session_id: 'chat-1' })] };
    return {};
  });
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('useChatCrewAccess on a browser surface', () => {
  it('a browser surface asks for no Crew connections', async () => {
    // `renderer.tsx` stamps this marker on `<html>` when it serves the browser build.
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await quietFor(50);
    // A grant changing elsewhere asks the hook to read again; it still asks for nothing.
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'chat-1', change: 'granted' })
    );
    window.dispatchEvent(new Event('focus'));
    await quietFor(50);
    expect(mocks.crewHttp).not.toHaveBeenCalled();
    // Nothing looked up: the chat shows nothing and blocks nothing.
    expect(result.current.state).toBe('unknown');
  });

  it('the desktop still reads the connections for the same chat', async () => {
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await vi.waitFor(() => expect(result.current.state).toBe('active'));
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections',
      'GET',
      undefined,
      expect.anything()
    );
  });
});
