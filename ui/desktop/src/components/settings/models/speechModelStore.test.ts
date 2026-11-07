import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  dictationModelsApi,
  resetDictationApiForTests,
  type DictationApi,
  type DictationModelsApi,
  type DictationResult,
  type DictationStatus,
} from '../../dictation/dictationApi';
import {
  INSTALL_POLL_MS,
  refreshDictationStatus,
  getDictationStatusSnapshot,
  resetDictationStoreForTests,
  useDictationStatus,
} from '../../dictation/dictationStore';
import {
  DICTATION_LANGUAGES,
  englishOnly,
  getSpeechModelsSnapshot,
  installAllowance,
  installBytes,
  installPercent,
  installQuestion,
  installSpeechModel,
  languageName,
  languageOptions,
  modelAfterRemoving,
  removeQuestion,
  removeSpeechModel,
  rescanNotice,
  rescanSpeechModels,
  resetSpeechModelsForTests,
  sectionInstallRefusal,
  selectSpeechModel,
  setDictationLanguage,
  speechModelRows,
  speedLine,
  testDictationSpeed,
} from './speechModelStore';

/** What the real model routes were called with: the generated client, recorded. */
const calls = vi.hoisted(() => ({ list: [] as Array<{ name: string; options: unknown }> }));

vi.mock('../../../api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../../api')>();
  const record = (name: string) => async (options: unknown) => {
    calls.list.push({ name, options });
    return { data: {}, response: new Response('{}', { status: 200 }) };
  };
  return {
    ...real,
    installDictationModel: record('install'),
    cancelDictationInstall: record('cancel'),
    deleteDictationModel: record('delete'),
    selectDictationModel: record('select'),
    rescanDictationModels: record('rescan'),
    benchDictationModel: record('bench'),
  };
});

const TURBO = 'whisper-large-v3-turbo-q5_0';
const SMALL = 'whisper-small.en-q8_0';
const allowed = { allowed: true, reason: null };

function model(
  id: string,
  overrides: Partial<DictationStatus['models'][number]> = {}
): DictationStatus['models'][number] {
  return {
    id,
    display_name: id === TURBO ? 'Whisper large-v3-turbo (q5_0)' : 'Whisper small.en (q8_0)',
    installed: false,
    bytes: id === TURBO ? 574_041_195 : 264_477_561,
    location: null,
    superseded: false,
    suitability: id === TURBO ? 'recommended' : 'suitable',
    recommended: id === TURBO,
    ...overrides,
  };
}

function status(overrides: Partial<DictationStatus> = {}): DictationStatus {
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
    models: [model(TURBO), model(SMALL)],
    active_model: null,
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
    limits: { max_seconds: 600, max_chunk_bytes: 65536, max_streams: 4 },
    queue: { streams: 0, segments: 0 },
    account: 'wgu',
    host: 'mac',
    models_dir: '/Users/wgu/.local/share/biorouter/dictation/models',
    ...overrides,
  };
}

const ok = <T>(data: T, code = 200): DictationResult<T> => ({ ok: true, status: code, data });
/** A refusal, assignable to the result of any route. */
const refused = (code: number, error: string, message = '') => ({
  ok: false as const,
  status: code,
  error: { code: error, message },
  retryAfterS: null,
});

/** A status API that answers whatever `answers` holds next, and the last answer after that. */
function statusApi(answers: DictationStatus[]) {
  const calls = { status: 0 };
  const api = {
    status: vi.fn(async () => {
      calls.status += 1;
      const next = answers.length > 1 ? answers.shift()! : answers[0];
      return ok(next);
    }),
    createStream: vi.fn(),
    putAudio: vi.fn(),
    finish: vi.fn(),
    poll: vi.fn(),
    drop: vi.fn(),
  } as unknown as DictationApi;
  return { api, calls };
}

function modelsApi(overrides: Partial<DictationModelsApi> = {}): DictationModelsApi {
  return {
    install: vi.fn(async (id: string) => ok({ model: id, phase: 'downloading' as const }, 202)),
    cancel: vi.fn(async () => ok({ cancelled: true })),
    remove: vi.fn(async (id: string) => ok({ removed: [id] })),
    select: vi.fn(async () => ok({ active_model: TURBO, language: 'en' })),
    rescan: vi.fn(async () => ok({ added: [], unrecognized: [] })),
    bench: vi.fn(async () => ok({ model: SMALL, rtf: 0.04, elapsed_ms: 220 })),
    ...overrides,
  };
}

const job = (
  bytes_done: number,
  phase: 'downloading' | 'verifying' | 'installed' | 'failed' = 'downloading',
  error: string | null = null
): NonNullable<DictationStatus['install_job']> => ({
  model: SMALL,
  bytes_done,
  bytes_total: 265_362_659,
  bytes_per_second: 10_000_000,
  phase,
  error,
});

beforeEach(() => {
  resetDictationStoreForTests();
  resetSpeechModelsForTests();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('installing (DD-66: the llamaServerStore pattern)', () => {
  it('polls the status every 1.5 s while the install runs, and stops when it ends', async () => {
    vi.useFakeTimers();
    const installed = status({
      models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
      active_model: SMALL,
      install_job: job(265_362_659, 'installed'),
    });
    const { api, calls } = statusApi([
      status({ install_job: job(1_000_000) }),
      status({ install_job: job(90_000_000) }),
      status({ install_job: job(265_362_659, 'verifying') }),
      installed,
    ]);
    resetDictationStoreForTests(api);
    const models = modelsApi();
    resetSpeechModelsForTests(models);

    await installSpeechModel(SMALL);
    expect(models.install).toHaveBeenCalledWith(SMALL);
    expect(calls.status).toBe(1);
    for (let poll = 2; poll <= 4; poll++) {
      await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS);
      expect(calls.status).toBe(poll);
    }
    expect(getDictationStatusSnapshot().status?.active_model).toBe(SMALL);
    // The job has ended: nothing is read again, however long the page stays open.
    await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS * 20);
    expect(calls.status).toBe(4);
  });

  it('reads nothing on a timer when no install runs', async () => {
    vi.useFakeTimers();
    const { api, calls } = statusApi([status()]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(modelsApi());
    await selectSpeechModel(TURBO);
    const after = calls.status;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls.status).toBe(after);
  });

  it('keeps an install’s progress across navigation: nothing is tied to a mounted page', async () => {
    vi.useFakeTimers();
    const { api } = statusApi([
      status({ install_job: job(10_000_000) }),
      status({ install_job: job(132_000_000) }),
    ]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(modelsApi());

    const page = renderHook(() => useDictationStatus());
    await act(async () => {
      await installSpeechModel(SMALL);
    });
    expect(installPercent(page.result.current.status!.install_job!)).toBe(3);
    // Leaving Settings unmounts the section; the download and its polls go on.
    page.unmount();
    await vi.advanceTimersByTimeAsync(INSTALL_POLL_MS);
    const back = renderHook(() => useDictationStatus());
    const row = speechModelRows(back.result.current.status!).find((r) => r.model.id === SMALL)!;
    expect(row.installing?.bytes_done).toBe(132_000_000);
    expect(installPercent(row.installing!)).toBe(49);
    expect(row.canInstall).toBe(false);
    back.unmount();
  });

  it('reads once more when the read after a 202 began before the install did', async () => {
    const { api, calls } = statusApi([status(), status({ install_job: job(1) })]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(modelsApi());
    await installSpeechModel(SMALL);
    expect(calls.status).toBe(2);
    expect(getDictationStatusSnapshot().status?.install_job?.phase).toBe('downloading');
  });

  it('keeps a refusal on its row in the daemon’s words, and clears it on the next try', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    const models = modelsApi({
      install: vi.fn(async () =>
        refused(
          403,
          'proofRequired',
          "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one."
        )
      ),
    });
    resetSpeechModelsForTests(models);
    await installSpeechModel(SMALL);
    expect(getSpeechModelsSnapshot().errors[SMALL]).toBe(
      "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one."
    );
    expect(getSpeechModelsSnapshot().pending[SMALL]).toBeUndefined();

    (models.install as ReturnType<typeof vi.fn>).mockResolvedValueOnce(refused(409, 'installBusy'));
    await installSpeechModel(SMALL);
    // An empty message gets MSG-SETTINGS-REFUSED, which names the code.
    expect(getSpeechModelsSnapshot().errors[SMALL]).toBe(
      'The Biorouter service refused this (installBusy, HTTP 409).'
    );
  });

  it('reports a daemon that did not answer, and never leaves a request marked in flight', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(
      modelsApi({ remove: vi.fn(async () => Promise.reject(new Error('socket hang up'))) })
    );
    await removeSpeechModel(SMALL);
    expect(getSpeechModelsSnapshot().errors[SMALL]).toBe(
      'The Biorouter service did not answer. Try again.'
    );
    expect(getSpeechModelsSnapshot().pending).toEqual({});
  });

  it('sends one request per model at a time', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    let release: () => void = () => undefined;
    const models = modelsApi({
      install: vi.fn(
        () =>
          new Promise<DictationResult<{ model: string; phase: 'installed' }>>((done) => {
            release = () => done(ok({ model: SMALL, phase: 'installed' }));
          })
      ),
    });
    resetSpeechModelsForTests(models);
    const first = installSpeechModel(SMALL);
    await Promise.resolve();
    expect(getSpeechModelsSnapshot().pending[SMALL]).toBe('install');
    await installSpeechModel(SMALL);
    expect(models.install).toHaveBeenCalledTimes(1);
    release();
    await first;
  });
});

describe('what each row offers', () => {
  it('offers Install, Use, Update and Remove only where the daemon would do them', () => {
    const rows = speechModelRows(
      status({
        models: [
          model(TURBO, { installed: true, location: 'user' }),
          model(SMALL, { installed: true, location: 'system', superseded: true }),
        ],
        active_model: TURBO,
      })
    );
    const [turbo, small] = rows;
    expect(turbo).toMatchObject({
      active: true,
      canUse: false,
      canRemove: true,
      canInstall: false,
    });
    // A copy in the shared folder: usable, updatable into this account's folder, not removable.
    expect(small).toMatchObject({ active: false, canUse: true, canRemove: false, canUpdate: true });
    expect(small.elsewhere).toBe('system');
  });

  it('holds every Install while one install runs', () => {
    const rows = speechModelRows(status({ install_job: job(5) }));
    expect(rows.map((row) => row.canInstall)).toEqual([false, false]);
    expect(rows[1].installing?.bytes_done).toBe(5);
    expect(rows[0].installing).toBeNull();
  });

  it('shows a failed install on its row until the model is installed', () => {
    const failed = job(
      1,
      'failed',
      "The downloaded file did not match Biorouter's pin, so nothing was installed."
    );
    expect(speechModelRows(status({ install_job: failed }))[1].failure).toBe(failed.error);
    expect(
      speechModelRows(
        status({
          install_job: failed,
          models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
        })
      )[1].failure
    ).toBeNull();
  });

  it('does not bring a failed install back after the model, installed since, is removed', async () => {
    const cancelled = { ...job(16_278_622, 'failed', 'The install was cancelled.'), model: TURBO };
    const installed = status({
      models: [model(TURBO, { installed: true, location: 'user' }), model(SMALL)],
      active_model: TURBO,
      install_job: cancelled,
    });
    const removed = status({ install_job: cancelled });
    const { api } = statusApi([installed, removed]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(modelsApi());
    await refreshDictationStatus();
    // Rescan installed turbo after that job was cancelled: the row hides the failure.
    expect(speechModelRows(installed)[0].failure).toBeNull();
    await removeSpeechModel(TURBO);
    // The daemon still reports the job it ended before the install; it stays history.
    expect(speechModelRows(removed)[0].failure).toBeNull();
    // A new install that fails is reported again.
    const failedAgain = { ...cancelled, bytes_done: 2_000_000 };
    expect(speechModelRows(status({ install_job: failedAgain }))[0].failure).toBe(
      'The install was cancelled.'
    );
  });

  it('answers a refused install with status.install.reason, shown once for every model', () => {
    const agent =
      "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one.";
    const refusedStatus = status({ install: { allowed: false, reason: agent } });
    expect(sectionInstallRefusal(refusedStatus)).toBe(agent);
    for (const row of speechModelRows(refusedStatus)) {
      expect(row.allowance).toEqual({ allowed: false, reason: agent });
      expect(row.canInstall).toBe(false);
    }
  });

  it('leaves a policy sentence to the policy note, and refuses only the models the list leaves out', () => {
    const sentence =
      "Your organization's managed policy allows only these speech models here: whisper-small.en-q8_0.";
    const narrowed = status({
      install: { allowed: false, reason: sentence },
      policy: { ...status().policy, managed: true, allowed_models: [SMALL] },
    });
    expect(sectionInstallRefusal(narrowed)).toBeNull();
    const [turbo, small] = speechModelRows(narrowed);
    expect(installAllowance(narrowed, turbo.model)).toEqual({ allowed: false, reason: sentence });
    expect(turbo.notAllowed).toBe(true);
    expect(small.allowance.allowed).toBe(true);
    expect(small.canInstall).toBe(true);
  });

  it('counts a download’s megabytes against the size the row showed', () => {
    // Turbo's job also fetches the 885,098 B voice detector, first: 574,926,293 B in all.
    const turbo = {
      ...job(287_020_597),
      model: TURBO,
      bytes_total: 574_926_293,
    };
    expect(installBytes(turbo, 574_041_195)).toEqual({ done: 286_135_499, total: 574_041_195 });
    // While the detector downloads, none of the model's bytes have arrived.
    expect(installBytes({ ...turbo, bytes_done: 400_000 }, 574_041_195)).toEqual({
      done: 0,
      total: 574_041_195,
    });
    // The whole job's percent stays the composer's.
    expect(installPercent(turbo)).toBe(49);
    // A job of the model alone is counted as it is.
    expect(installBytes(job(90_000_000), 265_362_659)).toEqual({
      done: 90_000_000,
      total: 265_362_659,
    });
  });

  it('names the model that takes the place of the one in use, by the daemon’s rule', () => {
    const both = status({
      models: [
        model(TURBO, { installed: true, location: 'user' }),
        model(SMALL, { installed: true, location: 'user' }),
      ],
      active_model: TURBO,
    });
    expect(modelAfterRemoving(both, TURBO)?.id).toBe(SMALL);
    // The default (the row marked recommended) first, whatever the catalog order.
    expect(modelAfterRemoving({ ...both, active_model: SMALL }, SMALL)?.id).toBe(TURBO);
    // A model managed policy leaves out never takes the place.
    expect(
      modelAfterRemoving(
        { ...both, policy: { ...both.policy, managed: true, allowed_models: [TURBO] } },
        TURBO
      )
    ).toBeNull();

    const folder = both.models_dir;
    expect(removeQuestion(both, both.models[0])).toBe(
      `Remove Whisper large-v3-turbo (q5_0) from ${folder}? It is the model in use, so dictation will switch to Whisper small.en (q8_0).`
    );
    expect(removeQuestion(both, both.models[1])).toBe(
      `Remove Whisper small.en (q8_0) from ${folder}?`
    );
    const alone = status({
      models: [model(TURBO, { installed: true, location: 'user' }), model(SMALL)],
      active_model: TURBO,
    });
    expect(removeQuestion(alone, alone.models[0])).toBe(
      `Remove Whisper large-v3-turbo (q5_0) from ${folder}? It is the model in use and no other installed model can take its place, so dictation will not work until you install one.`
    );
  });

  it('offers nothing but Remove while managed policy turns dictation off', () => {
    const off = {
      allowed: false,
      reason: "Dictation is turned off on this computer by your organization's managed policy.",
    };
    const rows = speechModelRows(
      status({
        install: off,
        models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
        policy: { ...status().policy, managed: true, enabled: off },
      })
    );
    expect(rows.map((row) => [row.canInstall, row.canUse, row.canUpdate])).toEqual([
      [false, false, false],
      [false, false, false],
    ]);
    expect(rows[1].canRemove).toBe(true);
  });
});

describe('the warning before a slow install (DD-22, P-8)', () => {
  it('MSG-TURBO-CPU names the time Test speed measured for that model', async () => {
    const { api } = statusApi([
      status({
        models: [
          model(TURBO, { installed: true, location: 'user', suitability: 'too_slow' }),
          model(SMALL),
        ],
        active_model: TURBO,
      }),
    ]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(
      modelsApi({ bench: vi.fn(async () => ok({ model: TURBO, rtf: 0.87, elapsed_ms: 4785 })) })
    );
    await testDictationSpeed(TURBO);
    expect(getSpeechModelsSnapshot().measured[TURBO]).toEqual({
      model: TURBO,
      rtf: 0.87,
      elapsed_ms: 4785,
    });
    const question = installQuestion(model(TURBO, { suitability: 'too_slow' }));
    expect(question?.id).toBe('MSG-TURBO-CPU');
    expect(question?.text).toBe(
      'This model takes about 4.8 s to transcribe 5.5 s of speech on this computer. Install it anyway?'
    );
  });

  it('names the rule when nothing was measured, and asks nothing of a suitable model', () => {
    expect(installQuestion(model(TURBO, { suitability: 'too_slow' }), {})).toEqual({
      id: 'MSG-TOO-SLOW-SETTINGS',
      text: 'Whisper large-v3-turbo (q5_0) may be too slow for dictation on this computer. After you install it, Test speed measures it. Install it anyway?',
      tone: 'warning',
    });
    expect(installQuestion(model(SMALL, { suitability: 'below_minimum' }), {})?.id).toBe(
      'MSG-BELOW-MINIMUM-SETTINGS'
    );
    expect(installQuestion(model(SMALL, { suitability: 'suitable' }), {})).toBeNull();
    expect(installQuestion(model(TURBO, { suitability: 'recommended' }), {})).toBeNull();
  });
});

describe('language, Rescan and Test speed', () => {
  // The daemon's MSG-LANGUAGE-EN, as `DictationError::EnglishOnly` words it: by catalog id.
  const daemonEnglish = `${SMALL} understands only English. Choose a multilingual model for other languages, or set the language to English.`;
  const pageEnglish =
    'Whisper small.en (q8_0) understands only English. Choose a multilingual model for other languages, or set the language to English.';

  it('saves the language, and words MSG-LANGUAGE-EN with the display name of the model in use', async () => {
    // The page may still think turbo is in use; the read after the refusal names small.en.
    const { api } = statusApi([
      status({
        models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
        active_model: SMALL,
      }),
    ]);
    resetDictationStoreForTests(api);
    const models = modelsApi({
      select: vi.fn(async () => refused(409, 'languageUnsupported', daemonEnglish)),
    });
    resetSpeechModelsForTests(models);
    await setDictationLanguage('de');
    expect(models.select).toHaveBeenCalledWith({ language: 'de' });
    expect(getSpeechModelsSnapshot().language).toEqual({ pending: null, error: pageEnglish });

    // Choosing a multilingual model leaves that refusal behind.
    (models.select as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      ok({ active_model: TURBO, language: 'en' })
    );
    await selectSpeechModel(TURBO);
    expect(getSpeechModelsSnapshot().language.error).toBeNull();
  });

  it('keeps the daemon’s sentence when no English only model is in use to name', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(
      modelsApi({ select: vi.fn(async () => refused(409, 'languageUnsupported', daemonEnglish)) })
    );
    await setDictationLanguage('de');
    expect(getSpeechModelsSnapshot().language.error).toBe(daemonEnglish);
  });

  it('words a Use that a .en model refuses with the row’s display name', async () => {
    const { api } = statusApi([status({ language: 'de' })]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(
      modelsApi({ select: vi.fn(async () => refused(409, 'languageUnsupported', daemonEnglish)) })
    );
    await selectSpeechModel(SMALL, 'Whisper small.en (q8_0)');
    expect(getSpeechModelsSnapshot().errors[SMALL]).toBe(pageEnglish);
  });

  it('drops a refused Use once the model is in use, or once the language it was about changes', async () => {
    const both = [
      model(TURBO, { installed: true, location: 'user' }),
      model(SMALL, { installed: true, location: 'user' }),
    ];
    const { api } = statusApi([status({ models: both, active_model: TURBO, language: 'de' })]);
    resetDictationStoreForTests(api);
    const models = modelsApi({
      select: vi.fn(async () => refused(409, 'languageUnsupported', daemonEnglish)),
    });
    resetSpeechModelsForTests(models);
    await selectSpeechModel(SMALL, 'Whisper small.en (q8_0)');
    const smallRow = (s: DictationStatus) => speechModelRows(s).find((r) => r.model.id === SMALL)!;
    expect(smallRow(status({ models: both, active_model: TURBO })).error).toBe(pageEnglish);
    // small.en in use some other way (turbo removed): its refused Use is moot.
    expect(smallRow(status({ models: both, active_model: SMALL })).error).toBeNull();

    // English saved: the refusal said "or set the language to English", so it goes.
    (models.select as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      ok({ active_model: TURBO, language: 'en' })
    );
    await setDictationLanguage('en');
    expect(getSpeechModelsSnapshot().errors[SMALL]).toBeUndefined();
    // Other refusals stay where they are.
    (models.remove as ReturnType<typeof vi.fn>).mockResolvedValueOnce(refused(409, 'installBusy'));
    await removeSpeechModel(TURBO);
    (models.select as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      ok({ active_model: TURBO, language: 'fr' })
    );
    await setDictationLanguage('fr');
    expect(getSpeechModelsSnapshot().errors[TURBO]).toBe(
      'The Biorouter service refused this (installBusy, HTTP 409).'
    );
  });

  it('words what a Rescan found in the catalog’s sentences', () => {
    const folder = '/Users/wgu/.local/share/biorouter/dictation/models';
    expect(rescanNotice([], [], folder).text).toBe(`No new model file was found in ${folder}.`);
    expect(rescanNotice(['Whisper small.en (q8_0)'], [], folder)).toEqual({
      id: 'MSG-RESCAN-ADDED',
      text: 'Installed Whisper small.en (q8_0) from the model folder.',
      tone: 'success',
    });
    expect(rescanNotice([], ['notes.txt', 'old.bin'], folder).text).toBe(
      'Not Biorouter model files, so not installed: notes.txt, old.bin.'
    );
  });

  it('runs a Rescan with no argument and names the placed models by display name', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    const models = modelsApi({
      rescan: vi.fn(async () => ok({ added: [SMALL, 'silero-vad-v6.2.0'], unrecognized: [] })),
    });
    resetSpeechModelsForTests(models);
    await rescanSpeechModels('/models', (id) => (id === SMALL ? 'Whisper small.en (q8_0)' : id));
    expect(models.rescan).toHaveBeenCalledWith();
    expect(getSpeechModelsSnapshot().rescan.notice?.text).toBe(
      'Installed Whisper small.en (q8_0), silero-vad-v6.2.0 from the model folder.'
    );
  });

  it('keeps a Test speed refusal in the daemon’s words', async () => {
    const { api } = statusApi([status()]);
    resetDictationStoreForTests(api);
    resetSpeechModelsForTests(
      modelsApi({
        bench: vi.fn(async () =>
          refused(409, 'notCompiled', 'This build of Biorouter has no local dictation engine.')
        ),
      })
    );
    await testDictationSpeed(SMALL);
    expect(getSpeechModelsSnapshot().bench).toEqual({
      running: false,
      model: SMALL,
      error: 'This build of Biorouter has no local dictation engine.',
    });
  });

  it('shows the time measured for the model in use only, and nothing while a run is in progress', async () => {
    const both = [
      model(TURBO, { installed: true, location: 'user' }),
      model(SMALL, { installed: true, location: 'user' }),
    ];
    const smallInUse = status({ models: both, active_model: SMALL });
    const { api } = statusApi([smallInUse]);
    resetDictationStoreForTests(api);
    let answer: (
      value: DictationResult<{ model: string; rtf: number; elapsed_ms: number }>
    ) => void = () => undefined;
    const models = modelsApi();
    resetSpeechModelsForTests(models);
    await testDictationSpeed(SMALL);
    expect(speedLine(smallInUse)).toMatchObject({ result: { model: SMALL, elapsed_ms: 220 } });

    // Another model in use: small.en's time is not its time.
    const turboInUse = status({ models: both, active_model: TURBO });
    expect(speedLine(turboInUse)).toBeNull();
    // Removed: small.en is no longer installed, whatever the status still names.
    expect(speedLine(status({ models: [both[0], model(SMALL)], active_model: SMALL }))).toBeNull();
    // In use again: its time comes back.
    expect(speedLine(smallInUse)).toMatchObject({ result: { model: SMALL } });

    // A run in progress hides the earlier line until it answers.
    (models.bench as ReturnType<typeof vi.fn>).mockImplementationOnce(
      () => new Promise((done) => (answer = done))
    );
    const run = testDictationSpeed(SMALL);
    expect(speedLine(smallInUse)).toBeNull();
    answer(ok({ model: SMALL, rtf: 0.03, elapsed_ms: 165 }));
    await run;
    expect(speedLine(smallInUse)).toMatchObject({ result: { elapsed_ms: 165 } });

    // A refusal is the line of the model it was asked for, and of no other.
    (models.bench as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      refused(500, 'engineFailed', 'The dictation worker was stopped.')
    );
    await testDictationSpeed(SMALL);
    expect(speedLine(smallInUse)).toEqual({ error: 'The dictation worker was stopped.' });
    expect(speedLine(turboInUse)).toBeNull();
  });

  it('offers every language the engine takes, Detect automatically and English first', () => {
    const protocol = readFileSync(
      resolve(__dirname, '../../../../../../crates/biorouter-dictation/src/protocol.rs'),
      'utf8'
    );
    const block = protocol.slice(protocol.indexOf('pub const WHISPER_LANGUAGES'));
    const codes = [...block.slice(0, block.indexOf('];')).matchAll(/"([a-z]+)"/g)].map((m) => m[1]);
    expect(codes.length).toBeGreaterThan(90);
    expect([...DICTATION_LANGUAGES]).toEqual(codes);

    const options = languageOptions('Detect automatically');
    expect(options.slice(0, 2)).toEqual([
      { value: 'auto', label: 'Detect automatically' },
      { value: 'en', label: 'English' },
    ]);
    expect(options).toHaveLength(codes.length + 1);
    expect(options.find((option) => option.value === 'de')?.label).toBe('German');
    // A code the list does not hold stays in the menu rather than reading as another choice.
    const kept = languageOptions('Detect automatically', 'xx');
    expect(kept[kept.length - 1]).toEqual({ value: 'xx', label: 'xx' });
  });

  it('names every language without the platform, as Electron’s reduced ICU cannot name bo and ba', () => {
    // Electron's renderer answered `of('bo')` and `of('ba')` with the code itself (2026-11-07).
    const real = Intl.DisplayNames;
    const reduced = class {
      of(code: string) {
        return code === 'bo' || code === 'ba'
          ? code
          : new real(['en'], { type: 'language' }).of(code);
      }
    };
    Object.defineProperty(Intl, 'DisplayNames', { configurable: true, value: reduced });
    try {
      expect(languageName('bo')).toBe('Tibetan');
      expect(languageName('ba')).toBe('Bashkir');
      for (const code of DICTATION_LANGUAGES) {
        expect(languageName(code), code).not.toBe(code);
      }
      const labels = languageOptions('Detect automatically').map((option) => option.label);
      expect(labels.indexOf('Bashkir')).toBe(labels.indexOf('Basque') - 1);
    } finally {
      Object.defineProperty(Intl, 'DisplayNames', { configurable: true, value: real });
    }
  });

  it('offers English alone while an English only model is in use, beside a current other value', () => {
    expect(languageOptions('Detect automatically', 'en', true)).toEqual([
      { value: 'en', label: 'English' },
    ]);
    expect(languageOptions('Detect automatically', 'de', true)).toEqual([
      { value: 'en', label: 'English' },
      { value: 'de', label: 'German' },
    ]);
    expect(languageOptions('Detect automatically', 'auto', true)).toEqual([
      { value: 'auto', label: 'Detect automatically' },
      { value: 'en', label: 'English' },
    ]);
  });

  it('reads English only from a Whisper .en id', () => {
    expect(englishOnly(SMALL)).toBe(true);
    expect(englishOnly('whisper-base.en')).toBe(true);
    expect(englishOnly(TURBO)).toBe(false);
  });
});

describe('the model routes carry the proof of a person (DD-20)', () => {
  beforeEach(() => {
    calls.list = [];
    Object.assign(window, {
      appConfig: {
        get: (key: string) => (key === 'BIOROUTER_API_HOST' ? 'http://127.0.0.1:4567' : undefined),
      },
      electron: {
        ...(window as unknown as { electron?: object }).electron,
        getSecretKey: vi.fn(async () => 'secret'),
        getUserActionKey: vi.fn(async () => 'person'),
      },
    });
    resetDictationApiForTests();
  });

  it('sends userActionHeaders() on install, cancel, delete, select and rescan, and not on bench', async () => {
    await dictationModelsApi.install(SMALL);
    await dictationModelsApi.cancel(SMALL);
    await dictationModelsApi.remove(SMALL);
    await dictationModelsApi.select({ model: SMALL, language: 'en' });
    await dictationModelsApi.rescan();
    await dictationModelsApi.bench();
    const headers = Object.fromEntries(
      calls.list.map(({ name, options }) => [
        name,
        (options as { headers: Record<string, string> }).headers,
      ])
    );
    for (const name of ['install', 'cancel', 'delete', 'select', 'rescan']) {
      expect(headers[name], name).toEqual({ 'X-Secret-Key': 'secret', 'X-User-Action': 'person' });
    }
    expect(headers.bench).toEqual({ 'X-Secret-Key': 'secret' });
    const bodies = Object.fromEntries(
      calls.list.map(({ name, options }) => [name, (options as { body?: unknown }).body])
    );
    expect(bodies.install).toEqual({ model: SMALL });
    expect(bodies.delete).toEqual({ model: SMALL });
    expect(bodies.select).toEqual({ model: SMALL, language: 'en' });
    expect(bodies.rescan).toBeUndefined();
  });
});
