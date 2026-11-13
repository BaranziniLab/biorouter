import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BUNDLED_BIOROUTER_VERSION } from '../../utils/biorouterVersion';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { resetCapturePrewarmForTests } from './capture';
import { DICTATION_MESSAGES } from './copy';
import type { DictationApi, DictationResult, DictationStatus } from './dictationApi';
import {
  DICTATION_VIEWER_KEY,
  normalizeRemoteHttpBackendUrl,
  acknowledgeRemoteHttp,
  dictationDaemonPlace,
  ensureDictationStatus,
  getDictationStatusSnapshot,
  INSTALL_POLL_MS,
  noteDictationInterest,
  readDictationViewer,
  resetDictationStoreForTests,
  resolveDictationPhase,
  STATUS_TIMEOUT_MS,
  subscribeDictationStatus,
  useDictationStatus,
  writeDictationViewer,
  type DictationStatusSnapshot,
} from './dictationStore';

function status(overrides: Partial<DictationStatus> = {}): DictationStatus {
  const allowed = { allowed: true, reason: null };
  return {
    engine: {
      compiled: true,
      state: 'stopped',
      backend: 'metal',
      device: 'Apple M4 Max',
      threads: 8,
      missing_cpu_features: [],
      rss_bytes: null,
    },
    models: [
      {
        id: 'whisper-large-v3-turbo-q5_0',
        display_name: 'Whisper large v3 turbo',
        installed: true,
        bytes: 574041195,
        location: 'user',
        superseded: false,
        suitability: 'recommended',
        recommended: true,
      },
    ],
    active_model: 'whisper-large-v3-turbo-q5_0',
    language: 'en',
    install: allowed,
    install_job: null,
    policy: {
      managed: false,
      enabled: allowed,
      capture: allowed,
      install: allowed,
      external_engine: allowed,
      dictionary: allowed,
      agent_writes: allowed,
      allowed_models: null,
    },
    limits: { max_seconds: 600, max_chunk_bytes: 524288, max_streams: 8 },
    queue: { streams: 0, segments: 0 },
    account: 'wgu',
    host: 'this-mac',
    models_dir: '/Users/wgu/.local/share/biorouter/dictation/models',
    ...overrides,
  };
}

function fakeApi(answers: Array<DictationResult<DictationStatus> | Error>) {
  const calls: number[] = [];
  const api = {
    status: vi.fn(async () => {
      calls.push(Date.now());
      const next = answers.length > 1 ? answers.shift()! : answers[0];
      if (next instanceof Error) throw next;
      return next;
    }),
  } as unknown as DictationApi;
  return { api, calls };
}

const ok = (data: DictationStatus): DictationResult<DictationStatus> => ({
  ok: true,
  status: 200,
  data,
});

beforeEach(() => {
  window.localStorage.clear();
  Object.assign(window, { electron: { dictationMicStatus: vi.fn(async () => 'granted') } });
});

afterEach(() => {
  resetDictationStoreForTests();
  vi.useRealTimers();
});

describe('when the store loads', () => {
  it('sends nothing until something shows interest in dictation, not even to a subscriber', async () => {
    // The real API, so a request from subscribing or from importing would reach fetch.
    resetDictationStoreForTests();
    const fetch = vi.fn(async () => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetch);
    try {
      // What a mounting ChatInput does: subscribe through the hook.
      const { result, unmount } = renderHook(() => useDictationStatus());
      subscribeDictationStatus(() => undefined);
      for (let i = 0; i < 5; i++) await Promise.resolve();
      expect(result.current.load).toBe('unloaded');
      expect(fetch).not.toHaveBeenCalled();
      unmount();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('gives up on a status read after 5 s, so a hung daemon holds neither a press nor later reads', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const signals: Array<AbortSignal | undefined> = [];
    const api = {
      status: vi.fn((signal?: AbortSignal) => {
        signals.push(signal);
        return new Promise<never>(() => undefined);
      }),
    } as unknown as DictationApi;
    resetDictationStoreForTests(api);
    const read = ensureDictationStatus();
    await vi.advanceTimersByTimeAsync(STATUS_TIMEOUT_MS - 1);
    expect(getDictationStatusSnapshot().load).toBe('loading');
    await vi.advanceTimersByTimeAsync(2);
    await expect(read).resolves.toBeNull();
    expect(getDictationStatusSnapshot().load).toBe('failed');
    expect(signals[0]?.aborted).toBe(true);
    // The phase reads idle, so a press opens its stream and the create decides.
    expect(resolveDictationPhase(getDictationStatusSnapshot(), { secure: true }).phase).toBe(
      'idle'
    );
    // The next sign of interest asks again rather than joining the hung read.
    noteDictationInterest();
    expect(api.status).toHaveBeenCalledTimes(2);
  });

  it('loads once on the first interest, and tells subscribers', async () => {
    const { api } = fakeApi([ok(status())]);
    resetDictationStoreForTests(api);
    const listener = vi.fn();
    subscribeDictationStatus(listener);
    noteDictationInterest();
    noteDictationInterest();
    expect(getDictationStatusSnapshot().load).toBe('loading');
    await ensureDictationStatus();
    noteDictationInterest();
    expect(api.status).toHaveBeenCalledTimes(1);
    expect(getDictationStatusSnapshot()).toMatchObject({ load: 'loaded', micStatus: 'granted' });
    expect(listener).toHaveBeenCalled();
  });

  it('interest also warms the media device list, without opening a microphone', async () => {
    const { api } = fakeApi([ok(status())]);
    resetDictationStoreForTests(api);
    resetCapturePrewarmForTests();
    const enumerateDevices = vi.fn(async () => []);
    const getUserMedia = vi.fn();
    Object.defineProperty(window, 'isSecureContext', { value: true, configurable: true });
    Object.defineProperty(navigator, 'mediaDevices', {
      value: { enumerateDevices, getUserMedia },
      configurable: true,
    });
    noteDictationInterest();
    await ensureDictationStatus();
    expect(enumerateDevices).toHaveBeenCalledTimes(1);
    expect(getUserMedia).not.toHaveBeenCalled();
    Object.defineProperty(navigator, 'mediaDevices', { value: undefined, configurable: true });
  });

  it('reads 404 or 405 as a backend from before dictation', async () => {
    const { api } = fakeApi([
      { ok: false, status: 404, error: { code: 'http404', message: '' }, retryAfterS: null },
    ]);
    resetDictationStoreForTests(api);
    await ensureDictationStatus();
    expect(getDictationStatusSnapshot().load).toBe('absent');
    expect(
      resolveDictationPhase(getDictationStatusSnapshot(), {
        secure: true,
        host: 'lab',
        appVersion: '1.95.0',
      })
    ).toMatchObject({
      phase: 'unavailable',
      reasonId: 'MSG-REMOTE-OLD',
    });
  });

  it('tries again after a failed read', async () => {
    const { api } = fakeApi([new Error('offline'), ok(status())]);
    resetDictationStoreForTests(api);
    await ensureDictationStatus();
    expect(getDictationStatusSnapshot().load).toBe('failed');
    await ensureDictationStatus();
    expect(getDictationStatusSnapshot().load).toBe('loaded');
  });

  it('refreshes when the window regains focus, once loaded', async () => {
    const { api } = fakeApi([ok(status())]);
    resetDictationStoreForTests(api);
    window.dispatchEvent(new Event('focus'));
    expect(api.status).not.toHaveBeenCalled();
    await ensureDictationStatus();
    window.dispatchEvent(new Event('focus'));
    await Promise.resolve();
    expect(api.status).toHaveBeenCalledTimes(2);
  });

  it('polls every 1.5 s only while a model installs', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const job = (phase: 'downloading' | 'installed') => ({
      model: 'whisper-small.en-q8_0',
      bytes_done: 100,
      bytes_total: 400,
      bytes_per_second: 10,
      phase,
      error: null,
    });
    const { api } = fakeApi([
      ok(status({ install_job: job('downloading') })),
      ok(status({ install_job: job('downloading') })),
      ok(status({ install_job: job('installed') })),
    ]);
    resetDictationStoreForTests(api);
    await ensureDictationStatus();
    expect(api.status).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS);
    expect(api.status).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS);
    expect(api.status).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS * 4);
    expect(api.status).toHaveBeenCalledTimes(3);
  });
});

describe('resolveDictationPhase: the phase table', () => {
  const loaded = (data: DictationStatus): DictationStatusSnapshot => ({
    load: 'loaded',
    status: data,
    micStatus: 'granted',
  });
  const env = { secure: true, surface: 'desktop' as const };

  it('an insecure page is unavailable before anything else, loaded or not', () => {
    expect(
      resolveDictationPhase(
        { load: 'unloaded', status: null, micStatus: 'unknown' },
        { secure: false, host: '10.0.0.5:8765' }
      )
    ).toMatchObject({ phase: 'unavailable', reasonId: 'MSG-INSECURE' });
  });

  it('before the status has loaded, the microphone looks idle so a first click still records', () => {
    expect(
      resolveDictationPhase({ load: 'unloaded', status: null, micStatus: 'unknown' }, env).phase
    ).toBe('idle');
    expect(
      resolveDictationPhase({ load: 'failed', status: null, micStatus: 'unknown' }, env).phase
    ).toBe('idle');
  });

  it('policy that forbids dictation or live capture leaves the button unavailable with its reason', () => {
    const base = status();
    const enabledOff = { allowed: false, reason: 'Dictation is off here.' };
    const captureOff = { allowed: false, reason: 'Live capture is off here.' };
    // The daemon's own sentence for the refused door, as Settings shows it.
    expect(
      resolveDictationPhase(
        loaded(status({ policy: { ...base.policy, enabled: enabledOff, capture: captureOff } })),
        env
      )
    ).toEqual({
      phase: 'unavailable',
      reasonId: 'MSG-POLICY',
      reason: 'Dictation is off here.',
    });
    expect(
      resolveDictationPhase(
        loaded(status({ policy: { ...base.policy, capture: captureOff } })),
        env
      )
    ).toEqual({
      phase: 'unavailable',
      reasonId: 'MSG-POLICY-CAPTURE',
      reason: 'Live capture is off here.',
    });
    // The catalog's sentence only when the daemon gave none.
    expect(
      resolveDictationPhase(
        loaded(status({ policy: { ...base.policy, enabled: { allowed: false, reason: null } } })),
        env
      ).reason
    ).toBe(DICTATION_MESSAGES['MSG-POLICY']);
    expect(
      resolveDictationPhase(
        loaded(status({ policy: { ...base.policy, capture: { allowed: false, reason: null } } })),
        env
      ).reason
    ).toBe(DICTATION_MESSAGES['MSG-POLICY-CAPTURE']);
  });

  it('no engine, and an unsupported processor, are unavailable with their sentences', () => {
    const base = status();
    expect(
      resolveDictationPhase(loaded(status({ engine: { ...base.engine, compiled: false } })), env)
    ).toMatchObject({ phase: 'unavailable', reasonId: 'MSG-NO-ENGINE' });
    const cpu = status({
      engine: { ...base.engine, state: 'unsupported_cpu', missing_cpu_features: ['AVX2', 'FMA'] },
    });
    const desktop = resolveDictationPhase(loaded(cpu), env);
    expect(desktop.reason).toContain('(AVX2, FMA)');
    expect(desktop.reason).toContain('Settings > App > Use external server');
    expect(resolveDictationPhase(loaded(cpu), { secure: true, surface: 'browser' })).toMatchObject({
      reasonId: 'MSG-CPU-SERVER',
    });
    // The Intel build under Rosetta before macOS 15 names the build to install instead.
    const rosetta = resolveDictationPhase(loaded(cpu), { ...env, rosettaBeforeMacOS15: true });
    expect(rosetta).toMatchObject({ phase: 'unavailable', reasonId: 'MSG-CPU-ROSETTA' });
    expect(rosetta.reason).toContain('Apple Silicon build');
  });

  it('without a model: set up, installing, or the reason installing is refused', () => {
    const none = { models: [], active_model: null };
    expect(resolveDictationPhase(loaded(status(none)), env).phase).toBe('not-installed');
    expect(
      resolveDictationPhase(
        loaded(
          status({
            ...none,
            install_job: {
              model: 'whisper-small.en-q8_0',
              bytes_done: 50,
              bytes_total: 200,
              bytes_per_second: 1,
              phase: 'downloading',
              error: null,
            },
          })
        ),
        env
      )
    ).toEqual({ phase: 'installing', progress: 25 });
    expect(
      resolveDictationPhase(
        loaded(
          status({
            ...none,
            install: { allowed: false, reason: 'Dictation is not set up on this server.' },
          })
        ),
        env
      )
    ).toEqual({ phase: 'unavailable', reason: 'Dictation is not set up on this server.' });
  });

  it('fills every placeholder from the page when the caller passes no environment', () => {
    Object.assign(window, {
      appConfig: {
        get: (key: string) =>
          key === 'BIOROUTER_API_HOST' ? 'https://lab.example:8765' : undefined,
      },
    });
    const insecure = resolveDictationPhase(
      { load: 'unloaded', status: null, micStatus: 'unknown' },
      { secure: false }
    );
    expect(insecure.reason).toContain(`http://${location.host}.`);
    const old = resolveDictationPhase(
      { load: 'absent', status: null, micStatus: 'unknown' },
      { secure: true }
    );
    expect(old.reason).toBe(
      `The Biorouter service on lab.example:8765 is a version without local dictation. Update it to ${BUNDLED_BIOROUTER_VERSION} or later, or turn off Use external server in Settings > App.`
    );
    for (const decision of [insecure, old]) expect(decision.reason).not.toMatch(/<[^<>]+>/);
  });

  it('a usable model while another installs is idle', () => {
    expect(
      resolveDictationPhase(
        loaded(
          status({
            install_job: {
              model: 'whisper-small.en-q8_0',
              bytes_done: 1,
              bytes_total: 2,
              bytes_per_second: 1,
              phase: 'downloading',
              error: null,
            },
          })
        ),
        env
      ).phase
    ).toBe('idle');
  });
});

describe('where speech is transcribed (DD-92)', () => {
  const config = (values: Record<string, unknown>) =>
    Object.assign(window, { appConfig: { get: (key: string) => values[key] } });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('is this computer only for the daemon the desktop app started itself', () => {
    config({ BIOROUTER_LOCAL_BACKEND: true, BIOROUTER_API_HOST: 'http://127.0.0.1:51234' });
    expect(dictationDaemonPlace(status())).toEqual({ kind: 'this-computer' });
  });

  it('is the server on an external backend, named as the daemon names itself, else by its address', () => {
    config({ BIOROUTER_LOCAL_BACKEND: false, BIOROUTER_API_HOST: 'https://lab.example:8765' });
    expect(dictationDaemonPlace(status({ host: 'lab-gpu-1' }))).toEqual({
      kind: 'external-backend',
      host: 'lab-gpu-1',
    });
    expect(dictationDaemonPlace(null)).toEqual({
      kind: 'external-backend',
      host: 'lab.example:8765',
    });
    // A window that cannot tell is not told "this computer".
    config({});
    expect(dictationDaemonPlace(status({ host: 'somewhere' })).kind).toBe('external-backend');
  });

  it('is the server a browser reaches on serve', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    config({ BIOROUTER_LOCAL_BACKEND: true });
    expect(dictationDaemonPlace(status({ host: 'hub-node-3' }))).toEqual({
      kind: 'serve',
      host: 'hub-node-3',
    });
  });
});

describe('per viewer settings', () => {
  it('reads defaults when nothing or garbage is stored, and keeps what is written', () => {
    expect(readDictationViewer()).toEqual({
      noiseReduction: false,
      showButton: true,
      ackHosts: [],
      policyOffHint: false,
    });
    window.localStorage.setItem(DICTATION_VIEWER_KEY, '{not json');
    expect(readDictationViewer().showButton).toBe(true);
    writeDictationViewer({ deviceId: 'abc', noiseReduction: true, ackHosts: ['lab:8765'] });
    expect(readDictationViewer()).toMatchObject({
      deviceId: 'abc',
      noiseReduction: true,
      ackHosts: ['lab:8765'],
    });
  });

  it('survives storage that throws', () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    expect(readDictationViewer().showButton).toBe(true);
    expect(() => writeDictationViewer({ showButton: false })).not.toThrow();
    getItem.mockRestore();
    setItem.mockRestore();
  });

  it('remembers that policy turned dictation off, for the next launch', async () => {
    const base = status();
    const { api } = fakeApi([
      ok(status({ policy: { ...base.policy, enabled: { allowed: false, reason: 'x' } } })),
    ]);
    resetDictationStoreForTests(api);
    await ensureDictationStatus();
    expect(readDictationViewer().policyOffHint).toBe(true);
  });
});

describe('external HTTP acknowledgement', () => {
  it('keeps one normalized URL per viewer in the existing store', () => {
    // The URL selection is covered by the backend configuration test; this pins persistence.
    acknowledgeRemoteHttp(`remote-http:sha256:${'a'.repeat(64)}`);
    acknowledgeRemoteHttp(`remote-http:sha256:${'a'.repeat(64)}`);
    expect(readDictationViewer().ackHosts).toEqual([`remote-http:sha256:${'a'.repeat(64)}`]);
    expect(normalizeRemoteHttpBackendUrl('http://LAB.example:8765/')).toEqual({
      url: 'http://lab.example:8765/',
      host: 'lab.example:8765',
    });
    expect(normalizeRemoteHttpBackendUrl('https://lab.example')).toBeNull();
    expect(normalizeRemoteHttpBackendUrl('http://127.0.0.1:8765')).toBeNull();
  });
});
