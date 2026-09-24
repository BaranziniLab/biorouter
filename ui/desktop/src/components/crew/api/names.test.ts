import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { CREW_UNEXPECTED_RESPONSE, isStaleDaemon } from './errors';
import { resolve } from './names';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

describe('resolve', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('posts the selectors, and the connection only when one is named', async () => {
    mocks.crewHttp.mockResolvedValue({
      results: [{ status: 'unknown_name', kind: 'team', text: 'analysis-lab' }],
    });
    const signal = new AbortController().signal;

    await resolve([{ kind: 'team', text: 'analysis-lab' }], undefined, signal);

    expect(mocks.crewHttp).toHaveBeenLastCalledWith(
      '/resolve',
      'POST',
      { selectors: [{ kind: 'team', text: 'analysis-lab' }] },
      signal
    );

    mocks.crewHttp.mockResolvedValue({
      connection: { status: 'resolved', kind: 'connection', text: 'conn-1', id: 'conn-1' },
      results: [{ status: 'unknown_name', kind: 'person', text: '@bob' }],
    });
    await resolve([{ text: '@bob' }], 'conn-1');

    expect(mocks.crewHttp).toHaveBeenLastCalledWith(
      '/resolve',
      'POST',
      { connection: 'conn-1', selectors: [{ text: '@bob' }] },
      undefined
    );
  });

  it('returns each typed result in selector order', async () => {
    mocks.crewHttp.mockResolvedValue({
      connection: {
        status: 'resolved',
        kind: 'connection',
        text: 'UCSF HPC',
        id: 'conn-1',
        label: 'UCSF HPC',
      },
      results: [
        {
          status: 'resolved',
          kind: 'person',
          text: '@bob',
          id: 'principal-bob',
          label: 'Bob Lee (@bob)',
          username: 'bob',
        },
        { status: 'unknown_name', kind: 'team', text: 'nowhere' },
        {
          status: 'ambiguous_name',
          kind: 'channel',
          text: 'general',
          candidates: ['analysis-lab/general', 'core/general'],
        },
      ],
    });

    const result = await resolve(
      [{ kind: 'person', text: '@bob' }, { text: 'nowhere' }, { text: 'general' }],
      'UCSF HPC'
    );

    expect(result).toEqual({
      connection: {
        status: 'resolved',
        kind: 'connection',
        text: 'UCSF HPC',
        id: 'conn-1',
        label: 'UCSF HPC',
      },
      results: [
        {
          status: 'resolved',
          kind: 'person',
          text: '@bob',
          id: 'principal-bob',
          label: 'Bob Lee (@bob)',
          username: 'bob',
        },
        { status: 'unknown_name', kind: 'team', text: 'nowhere' },
        {
          status: 'ambiguous_name',
          kind: 'channel',
          text: 'general',
          candidates: ['analysis-lab/general', 'core/general'],
        },
      ],
    });
  });

  it('keeps no candidates on an unknown name, whatever the daemon sent', async () => {
    mocks.crewHttp.mockResolvedValue({
      results: [
        { status: 'unknown_name', kind: 'person', text: '@carol', candidates: ['Carol (@carol)'] },
      ],
    });

    const [unknown] = (await resolve([{ text: '@carol' }])).results;

    expect(unknown).toEqual({ status: 'unknown_name', kind: 'person', text: '@carol' });
  });

  it.each([
    ['a missing results list', {}],
    ['fewer results than selectors', { results: [] }],
    [
      'a resolved name without an ID',
      { results: [{ status: 'resolved', kind: 'person', text: '@bob' }] },
    ],
    ['an unknown kind', { results: [{ status: 'unknown_name', kind: 'robot', text: '@bob' }] }],
    ['an unknown status', { results: [{ status: 'guessed', kind: 'person', text: '@bob' }] }],
    [
      'candidates that are not labels',
      {
        results: [{ status: 'ambiguous_name', kind: 'person', text: '@bob', candidates: [{}] }],
      },
    ],
  ])('refuses %s', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);

    const failure = await resolve([{ text: '@bob' }]).catch((error) => error);

    expect(failure).toBeInstanceOf(CrewHttpError);
    expect(failure.code).toBe(CREW_UNEXPECTED_RESPONSE);
    expect(isStaleDaemon(failure)).toBe(false);
  });

  it('refuses an answer that names no connection when one was asked for', async () => {
    mocks.crewHttp.mockResolvedValue({
      results: [{ status: 'unknown_name', kind: 'person', text: '@bob' }],
    });
    const failure = await resolve([{ text: '@bob' }], 'conn-1').catch((error) => error);
    expect(failure.code).toBe(CREW_UNEXPECTED_RESPONSE);

    mocks.crewHttp.mockResolvedValue({
      connection: { status: 'resolved', kind: 'team', text: 'conn-1', id: 'team-1' },
      results: [{ status: 'unknown_name', kind: 'person', text: '@bob' }],
    });
    const wrongKind = await resolve([{ text: '@bob' }], 'conn-1').catch((error) => error);
    expect(wrongKind.code).toBe(CREW_UNEXPECTED_RESPONSE);
  });

  it('reports an ambiguous connection instead of choosing one', async () => {
    mocks.crewHttp.mockResolvedValue({
      connection: {
        status: 'ambiguous_name',
        kind: 'connection',
        text: 'lab',
        candidates: ['lab — alice@hpc', 'lab — alice@other'],
      },
      results: [],
    });

    await expect(resolve([], 'lab')).resolves.toEqual({
      connection: {
        status: 'ambiguous_name',
        kind: 'connection',
        text: 'lab',
        candidates: ['lab — alice@hpc', 'lab — alice@other'],
      },
      results: [],
    });
  });

  it('recognizes a daemon that predates the resolver', async () => {
    mocks.crewHttp.mockRejectedValueOnce(new CrewHttpError('Crew request failed (404)', 404));
    expect(isStaleDaemon(await resolve([{ text: '@bob' }]).catch((error) => error))).toBe(true);

    mocks.crewHttp.mockRejectedValueOnce(new CrewHttpError('Crew request failed (405)', 405));
    expect(isStaleDaemon(await resolve([{ text: '@bob' }]).catch((error) => error))).toBe(true);

    // A success that is not JSON at all (crewHttp reads it as null) is an older daemon's web page.
    mocks.crewHttp.mockResolvedValueOnce(null);
    expect(isStaleDaemon(await resolve([{ text: '@bob' }]).catch((error) => error))).toBe(true);
  });
});
