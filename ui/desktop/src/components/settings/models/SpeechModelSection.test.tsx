import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';
import type { CaptureOptions } from '../../dictation/capture';
import { DICTATION_MESSAGES } from '../../dictation/copy';
import { resetSharedOriginNoticeForTests } from '../../dictation/sharedOriginGate';
import type {
  DictationApi,
  DictationModelsApi,
  DictationResult,
  DictationStatus,
} from '../../dictation/dictationApi';
import {
  DICTATION_VIEWER_KEY,
  readDictationViewer,
  resetDictationStoreForTests,
} from '../../dictation/dictationStore';
import { managedPolicySentences } from '../../privacy/ManagedPolicyNote';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { speechModelsCopy as copy } from './copy';
import SpeechModelSection, { MIC_TEST_MS, meterPercent } from './SpeechModelSection';
import {
  removeSpeechModel,
  resetSpeechModelsForTests,
  testDictationSpeed,
} from './speechModelStore';

/**
 * Settings > Models > Speech models (DD-66, PR-18), rendered with fake daemons: the status store's
 * API and the model routes' API. The rows' anatomy and their geometry are measured in a real
 * layout engine by `modelsGeometry.browser.test.tsx`; this file holds what each row offers, what
 * it sends, and the sentences it shows.
 */

const capture = vi.hoisted(() => ({
  options: null as null | CaptureOptions,
  cancel: vi.fn(),
  fail: null as null | Error,
}));

vi.mock('../../dictation/capture', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../dictation/capture')>();
  return {
    ...real,
    startCapture: vi.fn(async (options: CaptureOptions) => {
      if (capture.fail) throw capture.fail;
      capture.options = options;
      return {
        sampleRate: 16000,
        mode: 'worklet' as const,
        timings: {},
        stop: async () => undefined,
        cancel: capture.cancel,
        ended: new Promise<'stopped' | 'track-ended'>(() => undefined),
        closed: Promise.resolve(),
      };
    }),
  };
});

/** This window's dictation, as `dictationSession.ts` reports it to Settings. */
const session = vi.hoisted(() => ({ running: false, listeners: new Set<() => void>() }));

vi.mock('../../dictation/dictationSession', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../dictation/dictationSession')>();
  return {
    ...real,
    isDictationRunning: () => session.running,
    subscribeDictationRunning: (listener: () => void) => {
      session.listeners.add(listener);
      return () => session.listeners.delete(listener);
    },
  };
});

function setDictating(running: boolean) {
  act(() => {
    session.running = running;
    for (const listener of [...session.listeners]) listener();
  });
}

const TURBO = 'whisper-large-v3-turbo-q5_0';
const SMALL = 'whisper-small.en-q8_0';
const FOLDER = '/Users/wgu/.local/share/biorouter/dictation/models';
const allowed = { allowed: true, reason: null };

const model = (
  id: string,
  overrides: Partial<DictationStatus['models'][number]> = {}
): DictationStatus['models'][number] => ({
  id,
  display_name: id === TURBO ? 'Whisper large-v3-turbo (q5_0)' : 'Whisper small.en (q8_0)',
  installed: false,
  bytes: id === TURBO ? 574_041_195 : 264_477_561,
  location: null,
  superseded: false,
  suitability: id === TURBO ? 'recommended' : 'suitable',
  recommended: id === TURBO,
  ...overrides,
});

/** Both models installed in this account's folder. */
const both = () => [
  model(TURBO, { installed: true, location: 'user' }),
  model(SMALL, { installed: true, location: 'user' }),
];

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
    models_dir: FOLDER,
    ...overrides,
  };
}

const ok = <T,>(data: T, code = 200): DictationResult<T> => ({ ok: true, status: code, data });

let current: DictationStatus;
let statusApi: DictationApi;
let modelsApi: DictationModelsApi;
const revealFolder = vi.fn(async () => true);

function serve(next: DictationStatus) {
  current = next;
  statusApi = {
    status: vi.fn(async () => ok(current)),
  } as unknown as DictationApi;
  resetDictationStoreForTests(statusApi);
}

function models(overrides: Partial<DictationModelsApi> = {}) {
  modelsApi = {
    install: vi.fn(async (id: string) => ok({ model: id, phase: 'downloading' as const }, 202)),
    cancel: vi.fn(async () => ok({ cancelled: true })),
    remove: vi.fn(async (id: string) => ok({ removed: [id] })),
    select: vi.fn(async () => ok({ active_model: TURBO, language: 'en' })),
    rescan: vi.fn(async () => ok({ added: [], unrecognized: [] })),
    // The daemon's factor is the timed decode over the 5.5 s clip.
    bench: vi.fn(async () => ok({ model: TURBO, rtf: 281 / 5500, elapsed_ms: 281 })),
    ...overrides,
  };
  resetSpeechModelsForTests(modelsApi);
}

function desktop({ local = true }: { local?: boolean } = {}) {
  Object.assign(window, {
    appConfig: {
      get: (key: string) =>
        key === 'BIOROUTER_LOCAL_BACKEND'
          ? local
          : key === 'BIOROUTER_API_HOST'
            ? local
              ? 'http://127.0.0.1:4567'
              : 'https://lab.example:8765'
            : undefined,
    },
    electron: {
      platform: 'darwin',
      dictationMicStatus: vi.fn(async () => 'granted'),
      dictationMicRequest: vi.fn(async () => true),
      dictationOpenMicSettings: vi.fn(async () => true),
      dictationShowModelsFolder: revealFolder,
    },
  });
}

async function renderSection() {
  const view = render(<SpeechModelSection />);
  await screen.findByTestId('speech-privacy-line');
  return view;
}

const row = (id: string) => screen.getByTestId(`speech-model-${id}`);

beforeEach(() => {
  resetSharedOriginNoticeForTests();
  window.localStorage.removeItem(DICTATION_VIEWER_KEY);
  capture.options = null;
  capture.fail = null;
  capture.cancel.mockClear();
  revealFolder.mockClear();
  session.running = false;
  desktop();
  serve(status());
  models();
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  vi.useRealTimers();
});

describe('Speech models in Settings', () => {
  it('is one section of the Models tab, with the id the dictation deep link scrolls to', async () => {
    const { container } = await renderSection();
    const section = container.querySelector(`#${SETTINGS_SECTION_IDS.dictation}`);
    expect(section).not.toBeNull();
    expect(
      within(section as HTMLElement).getByRole('heading', { name: copy.section })
    ).toBeInTheDocument();
  });

  it('shows models on a serve browser without a host folder reveal action', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    await renderSection();
    expect(screen.getByTestId('speech-privacy-line')).toBeInTheDocument();
    expect(statusApi.status).toHaveBeenCalled();
    expect(
      within(screen.getByTestId('speech-model-folder')).queryByRole('button', {
        name: copy.showInFinder,
      })
    ).toBeNull();
    expect(revealFolder).not.toHaveBeenCalled();
  });

  it('shows the origin notice before the first browser microphone test and starts from its acknowledgement', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const user = userEvent.setup();
    await renderSection();
    await user.click(screen.getByRole('button', { name: copy.testMicrophone }));
    expect(capture.options).toBeNull();
    expect(
      screen.getByText(/Your browser's microphone permission covers every page at/)
    ).toBeVisible();
    await user.click(screen.getByTestId('shared-origin-acknowledge'));
    expect(capture.options).not.toBeNull();
  });

  it('says where speech is transcribed in visible text, never an InfoTip (DD-92)', async () => {
    await renderSection();
    expect(screen.getByTestId('speech-privacy-line')).toHaveTextContent(
      'Speech is transcribed by Biorouter on this computer. Audio is not saved.'
    );
  });

  it('names the backend host, and offers no Show in Finder, on an external backend', async () => {
    desktop({ local: false });
    serve(status({ host: 'lab-gpu-1' }));
    await renderSection();
    expect(screen.getByTestId('speech-privacy-line')).toHaveTextContent(
      "Audio is transcribed on lab-gpu-1, the computer running Biorouter's service. Models install there, not on this device."
    );
    expect(screen.queryByRole('button', { name: copy.showInFinder })).not.toBeInTheDocument();
  });

  it('reads the status once when it opens', async () => {
    await renderSection();
    expect(statusApi.status).toHaveBeenCalledTimes(1);
  });
});

describe('the model rows', () => {
  it('offers Install with each model’s size, languages and the recommendation', async () => {
    await renderSection();
    expect(
      within(row(TURBO)).getByText(/574 MB · Many languages · Recommended for this computer/)
    ).toBeInTheDocument();
    expect(within(row(SMALL)).getByText(/264 MB · English only/)).toBeInTheDocument();
    expect(
      within(row(TURBO)).getByRole('button', { name: 'Install Whisper large-v3-turbo (q5_0)' })
    ).toBeEnabled();
  });

  it('installs on a click, and shows the download’s progress with a Cancel', async () => {
    const user = userEvent.setup();
    await renderSection();
    current = status({
      install_job: {
        model: TURBO,
        bytes_done: 287_020_597,
        bytes_total: 574_926_293,
        bytes_per_second: 30_000_000,
        phase: 'downloading',
        error: null,
      },
    });
    expect(within(row(TURBO)).getByText(/^574 MB · /)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Install Whisper large-v3-turbo (q5_0)' }));
    expect(modelsApi.install).toHaveBeenCalledWith(TURBO);
    // The job also fetches the voice detector, so its total is 575 MB; the row counts the
    // model's own megabytes, against the 574 MB it showed before the click.
    expect(
      await within(row(TURBO)).findByText('Downloading 49% · 286 of 574 MB')
    ).toBeInTheDocument();
    expect(within(row(TURBO)).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '49');
    // One install at a time: the other model waits.
    expect(within(row(SMALL)).getByRole('button', { name: /^Install / })).toBeDisabled();

    await user.click(
      within(row(TURBO)).getByRole('button', { name: `Cancel ${model(TURBO).display_name}` })
    );
    expect(modelsApi.cancel).toHaveBeenCalledWith(TURBO);
  });

  it('shows no count while the installer connects, when the daemon reports the job at its total', async () => {
    serve(
      status({
        install_job: {
          model: SMALL,
          bytes_done: 264_477_561,
          bytes_total: 264_477_561,
          bytes_per_second: 0,
          phase: 'downloading',
          error: null,
        },
      })
    );
    await renderSection();
    expect(within(row(SMALL)).getByText(copy.downloadingUncounted)).toBeInTheDocument();
    expect(within(row(SMALL)).queryByText(/100%/)).not.toBeInTheDocument();
    expect(within(row(SMALL)).getByRole('progressbar')).not.toHaveAttribute('aria-valuenow');
  });

  it('asks before installing a model the daemon rated too slow (MSG-TOO-SLOW-SETTINGS)', async () => {
    const user = userEvent.setup();
    serve(status({ models: [model(TURBO, { suitability: 'too_slow' }), model(SMALL)] }));
    await renderSection();
    expect(within(row(TURBO)).getByText('Slow on this computer')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Install Whisper large-v3-turbo (q5_0)' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(
      'Whisper large-v3-turbo (q5_0) may be too slow for dictation on this computer. After you install it, Test speed measures it. Install it anyway?'
    );
    expect(modelsApi.install).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole('button', { name: 'Install anyway' }));
    await waitFor(() => expect(modelsApi.install).toHaveBeenCalledWith(TURBO));
  });

  it('shows In use on the active model, and Use on another installed one', async () => {
    const user = userEvent.setup();
    serve(
      status({
        models: [
          model(TURBO, { installed: true, location: 'user' }),
          model(SMALL, { installed: true, location: 'user' }),
        ],
        active_model: TURBO,
      })
    );
    await renderSection();
    expect(within(row(TURBO)).getByText(copy.inUse)).toBeInTheDocument();
    expect(within(row(TURBO)).queryByRole('button', { name: /^Use / })).not.toBeInTheDocument();
    await user.click(
      within(row(SMALL)).getByRole('button', { name: 'Use Whisper small.en (q8_0)' })
    );
    expect(modelsApi.select).toHaveBeenCalledWith({ model: SMALL });
  });

  async function openRemove(id: string) {
    const user = userEvent.setup();
    await user.click(
      within(row(id)).getByRole('button', { name: copy.more(model(id).display_name) })
    );
    await user.click(await screen.findByRole('menuitem', { name: copy.remove }));
    return { user, dialog: await screen.findByRole('dialog') };
  }

  it('removes after MSG-REMOVE-CONFIRM, from the row’s menu', async () => {
    serve(status({ models: both(), active_model: TURBO }));
    await renderSection();
    const { user, dialog } = await openRemove(SMALL);
    expect(dialog).toHaveTextContent(`Remove Whisper small.en (q8_0) from ${FOLDER}?`);
    expect(dialog).not.toHaveTextContent('in use');
    await user.click(within(dialog).getByRole('button', { name: copy.remove }));
    await waitFor(() => expect(modelsApi.remove).toHaveBeenCalledWith(SMALL));
  });

  it('says which model takes the place of the one in use before removing it', async () => {
    serve(status({ models: both(), active_model: TURBO }));
    await renderSection();
    const { user, dialog } = await openRemove(TURBO);
    expect(dialog).toHaveTextContent(
      `Remove Whisper large-v3-turbo (q5_0) from ${FOLDER}? It is the model in use, so dictation will switch to Whisper small.en (q8_0).`
    );
    await user.click(within(dialog).getByRole('button', { name: copy.remove }));
    await waitFor(() => expect(modelsApi.remove).toHaveBeenCalledWith(TURBO));
  });

  it('says dictation stops working when no installed model can take the place of the one in use', async () => {
    serve(
      status({
        models: [model(TURBO, { installed: true, location: 'user' }), model(SMALL)],
        active_model: TURBO,
      })
    );
    await renderSection();
    const { dialog } = await openRemove(TURBO);
    expect(dialog).toHaveTextContent(
      `Remove Whisper large-v3-turbo (q5_0) from ${FOLDER}? It is the model in use and no other installed model can take its place, so dictation will not work until you install one.`
    );
  });

  it('names the folder of a copy Remove cannot reach, and offers no Remove', async () => {
    const user = userEvent.setup();
    serve(
      status({
        models: [model(TURBO, { installed: true, location: 'system' }), model(SMALL)],
        active_model: TURBO,
      })
    );
    await renderSection();
    expect(within(row(TURBO)).getByText(/In the shared folder/)).toBeInTheDocument();
    await user.click(
      within(row(TURBO)).getByRole('button', { name: copy.more(model(TURBO).display_name) })
    );
    expect(await screen.findByRole('menu')).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: copy.remove })).not.toBeInTheDocument();
  });

  it('offers Update for an earlier revision', async () => {
    const user = userEvent.setup();
    serve(
      status({
        models: [
          model(TURBO, { installed: true, location: 'user', superseded: true }),
          model(SMALL),
        ],
        active_model: TURBO,
      })
    );
    await renderSection();
    expect(within(row(TURBO)).getByText(/A newer revision exists/)).toBeInTheDocument();
    await user.click(
      within(row(TURBO)).getByRole('button', { name: 'Update Whisper large-v3-turbo (q5_0)' })
    );
    expect(modelsApi.install).toHaveBeenCalledWith(TURBO);
  });

  it('shows a failed install’s sentence on its row, with Install there to try again', async () => {
    serve(
      status({
        install_job: {
          model: SMALL,
          bytes_done: 1,
          bytes_total: 2,
          bytes_per_second: 1,
          phase: 'failed',
          error: 'Installing Whisper small.en (q8_0) needs 0.5 GB free on /; 0.1 GB is available.',
        },
      })
    );
    await renderSection();
    expect(within(row(SMALL)).getByRole('alert')).toHaveTextContent(
      'Installing Whisper small.en (q8_0) needs 0.5 GB free on /; 0.1 GB is available.'
    );
    expect(within(row(SMALL)).getByRole('button', { name: /^Install / })).toBeEnabled();
  });

  it('shows status.install.reason in place of every Install button when the daemon refuses it', async () => {
    const reason =
      "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one.";
    serve(status({ install: { allowed: false, reason } }));
    await renderSection();
    expect(screen.getByTestId('speech-install-refusal')).toHaveTextContent(reason);
    expect(screen.queryByRole('button', { name: /^Install / })).not.toBeInTheDocument();
  });
});

describe('managed policy (ManagedPolicyNote)', () => {
  const off = {
    allowed: false,
    reason: "Dictation is turned off on this computer by your organization's managed policy.",
  };

  it('says what the policy turns off, and locks the rows it locks', async () => {
    serve(
      status({
        install: off,
        policy: { ...status().policy, managed: true, enabled: off },
      })
    );
    await renderSection();
    expect(screen.getByTestId('managed-policy-note')).toHaveTextContent(off.reason);
    // The refusal is the policy's, so it is said once, in the note.
    expect(screen.queryByTestId('speech-install-refusal')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Install / })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: copy.rescan })).toBeDisabled();
    expect(screen.getByRole('button', { name: copy.testMicrophone })).toBeDisabled();
    expect(screen.getByRole('button', { name: copy.testSpeed })).toBeDisabled();
  });

  it('lists the models an organization allows and marks the others', async () => {
    serve(
      status({
        install: {
          allowed: false,
          reason:
            "Your organization's managed policy allows only these speech models here: whisper-small.en-q8_0.",
        },
        policy: { ...status().policy, managed: true, allowed_models: [SMALL] },
      })
    );
    await renderSection();
    expect(screen.getByTestId('managed-policy-note')).toHaveTextContent(
      "Your organization's managed policy allows only these speech models here: whisper-small.en-q8_0."
    );
    expect(within(row(TURBO)).getByText(copy.notAllowed)).toBeInTheDocument();
    expect(within(row(TURBO)).queryByRole('button', { name: /^Install / })).not.toBeInTheDocument();
    expect(within(row(SMALL)).getByRole('button', { name: /^Install / })).toBeEnabled();
  });

  it('turns off the microphone test alone when only live capture is off', async () => {
    const capture = {
      allowed: false,
      reason:
        "Your organization's managed policy turns off live microphone dictation on this computer. Transcribing a recording still works.",
    };
    serve(status({ policy: { ...status().policy, managed: true, capture } }));
    await renderSection();
    expect(screen.getByRole('button', { name: copy.testMicrophone })).toBeDisabled();
    expect(screen.getAllByRole('button', { name: /^Install / })).toHaveLength(2);
    expect(screen.getByTestId('managed-policy-note')).toHaveTextContent(capture.reason);
  });

  it('says nothing for a policy that narrows nothing, and words each refused door', () => {
    const policy = status().policy;
    expect(managedPolicySentences({ ...policy, managed: true })).toEqual([]);
    expect(managedPolicySentences({ ...policy, managed: false, allowed_models: [] })).toEqual([]);
    expect(
      managedPolicySentences({
        ...policy,
        managed: true,
        allowed_models: [],
        install: { allowed: false, reason: null },
      })
    ).toEqual([
      "Your organization's managed policy turns off model downloads on this computer. Speech models come only from <folder>; ask your administrator to place one there.",
      "Your organization's managed policy allows only these speech models here: none.",
    ]);
  });
});

describe('the folder, language, microphone and speed rows', () => {
  it('shows the folder and the account, opens it in Finder, and Rescans with no argument', async () => {
    const user = userEvent.setup();
    models({
      rescan: vi.fn(async () => ok({ added: [SMALL, 'silero-vad-v6.2.0'], unrecognized: [] })),
    });
    await renderSection();
    const folder = screen.getByTestId('speech-model-folder');
    expect(folder).toHaveTextContent(`${FOLDER} · account wgu`);
    await user.click(within(folder).getByRole('button', { name: copy.showInFinder }));
    expect(revealFolder).toHaveBeenCalledWith(FOLDER);
    await user.click(within(folder).getByRole('button', { name: copy.rescan }));
    expect(modelsApi.rescan).toHaveBeenCalledWith();
    expect(await within(folder).findByRole('status')).toHaveTextContent(
      'Installed Whisper small.en (q8_0), Silero voice detector v6.2.0 from the model folder.'
    );
  });

  it('says Show in Explorer on Windows', async () => {
    Object.assign(window.electron, { platform: 'win32' });
    await renderSection();
    expect(screen.getByRole('button', { name: copy.showInExplorer })).toBeInTheDocument();
  });

  const englishRefusal = (message: string) => ({
    ok: false as const,
    status: 409,
    error: { code: 'languageUnsupported', message },
    retryAfterS: null,
  });

  it('saves a language with a multilingual model in use', async () => {
    const user = userEvent.setup();
    models({ select: vi.fn(async () => ok({ active_model: TURBO, language: 'de' })) });
    serve(
      status({ models: [model(TURBO, { installed: true, location: 'user' })], active_model: TURBO })
    );
    await renderSection();
    const language = screen.getByTestId('speech-language');
    await user.click(within(language).getByRole('button', { name: /^Language/ }));
    expect(await screen.findByRole('menuitemradio', { name: 'Tibetan' })).toBeInTheDocument();
    await user.click(screen.getByRole('menuitemradio', { name: 'German' }));
    expect(modelsApi.select).toHaveBeenCalledWith({ language: 'de' });
  });

  it('offers no other language while an English only model is in use, and says why', async () => {
    serve(
      status({
        models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
        active_model: SMALL,
      })
    );
    await renderSection();
    const language = screen.getByTestId('speech-language');
    expect(within(language).getByRole('button', { name: /^Language/ })).toBeDisabled();
    expect(language).toHaveTextContent(
      'Whisper small.en (q8_0) understands only English. To dictate in another language, use a multilingual model.'
    );
  });

  it('warns when the saved language is one the English only model in use refuses', async () => {
    const user = userEvent.setup();
    models({ select: vi.fn(async () => ok({ active_model: SMALL, language: 'en' })) });
    serve(
      status({
        models: [model(TURBO), model(SMALL, { installed: true, location: 'user' })],
        active_model: SMALL,
        language: 'de',
      })
    );
    await renderSection();
    const language = screen.getByTestId('speech-language');
    expect(within(language).getByRole('status')).toHaveTextContent(
      'Whisper small.en (q8_0) understands only English. Choose a multilingual model for other languages, or set the language to English.'
    );
    await user.click(within(language).getByRole('button', { name: /^Language/ }));
    const items = await screen.findAllByRole('menuitemradio');
    expect(items.map((item) => item.textContent)).toEqual(['English', 'German']);
    await user.click(screen.getByRole('menuitemradio', { name: 'English' }));
    expect(modelsApi.select).toHaveBeenCalledWith({ language: 'en' });
  });

  it('words a refused language with the display name, never the catalog id', async () => {
    const user = userEvent.setup();
    models({
      select: vi.fn(async () =>
        englishRefusal(
          `${SMALL} understands only English. Choose a multilingual model for other languages, or set the language to English.`
        )
      ),
    });
    // The page thinks turbo is in use; the daemon has small.en, which the next read shows.
    serve(status({ models: both(), active_model: TURBO }));
    await renderSection();
    current = status({ models: both(), active_model: SMALL });
    const language = screen.getByTestId('speech-language');
    await user.click(within(language).getByRole('button', { name: /^Language/ }));
    await user.click(await screen.findByRole('menuitemradio', { name: 'German' }));
    expect(modelsApi.select).toHaveBeenCalledWith({ language: 'de' });
    const alert = await within(language).findByRole('alert');
    expect(alert).toHaveTextContent(
      'Whisper small.en (q8_0) understands only English. Choose a multilingual model for other languages, or set the language to English.'
    );
    expect(alert).not.toHaveTextContent(SMALL);
  });

  it('lists the microphones the browser reports and keeps the choice for this viewer', async () => {
    const user = userEvent.setup();
    const enumerateDevices = vi.fn(async () => [
      { kind: 'audioinput', deviceId: 'default', label: 'Default', groupId: 'a' },
      { kind: 'audioinput', deviceId: 'mic-1', label: 'MacBook Pro Microphone', groupId: 'a' },
      { kind: 'videoinput', deviceId: 'cam', label: 'Camera', groupId: 'b' },
    ]);
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { enumerateDevices, addEventListener: vi.fn(), removeEventListener: vi.fn() },
    });
    await renderSection();
    const microphone = screen.getByTestId('speech-microphone');
    // Named by the row's label and the chosen device.
    await user.click(
      within(microphone).getByRole('button', { name: `${copy.microphone} ${copy.systemDefault}` })
    );
    expect(
      await screen.findByRole('menuitemradio', { name: copy.systemDefault })
    ).toBeInTheDocument();
    expect(screen.queryByRole('menuitemradio', { name: 'Default' })).not.toBeInTheDocument();
    expect(screen.queryByRole('menuitemradio', { name: 'Camera' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('menuitemradio', { name: 'MacBook Pro Microphone' }));
    expect(readDictationViewer()).toMatchObject({
      deviceId: 'mic-1',
      deviceLabel: 'MacBook Pro Microphone',
    });
  });

  it('tests the microphone for 5 s with a level meter, sends nothing, and lets the device go', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await renderSection();
    const microphone = screen.getByTestId('speech-microphone');
    await user.click(within(microphone).getByRole('button', { name: copy.testMicrophone }));
    await waitFor(() => expect(capture.options).not.toBeNull());
    act(() => capture.options!.onChunk(new Int16Array(1600), { rms: 0.1, peak: 0.3 }));
    expect(within(microphone).getByTestId('speech-mic-meter')).toHaveAttribute(
      'aria-valuenow',
      String(meterPercent(0.1))
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MIC_TEST_MS);
    });
    expect(capture.cancel).toHaveBeenCalled();
    expect(within(microphone).getByRole('status')).toHaveTextContent(copy.heard);
    // Nothing was sent: no model route, and no read beyond the one the section made on opening.
    for (const call of Object.values(modelsApi)) expect(call).not.toHaveBeenCalled();
    expect(statusApi.status).toHaveBeenCalledTimes(1);
  });

  it('says so when the test heard nothing, and words a refused microphone', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await renderSection();
    const microphone = screen.getByTestId('speech-microphone');
    await user.click(within(microphone).getByRole('button', { name: copy.testMicrophone }));
    await waitFor(() => expect(capture.options).not.toBeNull());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MIC_TEST_MS);
    });
    expect(within(microphone).getByRole('status')).toHaveTextContent(
      'No audio detected. Check that your microphone is not muted.'
    );

    capture.fail = Object.assign(new Error('denied'), { name: 'NotAllowedError' });
    (window.electron.dictationMicStatus as ReturnType<typeof vi.fn>).mockResolvedValue('denied');
    await user.click(within(microphone).getByRole('button', { name: copy.testMicrophone }));
    expect(await within(microphone).findByRole('alert')).toHaveTextContent(
      'Microphone access is off for Biorouter. Turn it on in System Settings > Privacy & Security > Microphone, then restart Biorouter.'
    );
    await user.click(within(microphone).getByRole('button', { name: 'Open settings' }));
    expect(window.electron.dictationOpenMicSettings).toHaveBeenCalled();
  });

  it('stops a test when Settings closes, so the microphone light goes off', async () => {
    const user = userEvent.setup();
    const view = await renderSection();
    await user.click(screen.getByRole('button', { name: copy.testMicrophone }));
    await waitFor(() => expect(capture.options).not.toBeNull());
    view.unmount();
    expect(capture.cancel).toHaveBeenCalled();
  });

  it('runs Test speed and shows the measured factor in MSG-BENCH', async () => {
    const user = userEvent.setup();
    serve(
      status({
        models: [model(TURBO, { installed: true, location: 'user' }), model(SMALL)],
        active_model: TURBO,
      })
    );
    await renderSection();
    const speed = screen.getByTestId('speech-speed');
    await user.click(within(speed).getByRole('button', { name: copy.testSpeed }));
    expect(modelsApi.bench).toHaveBeenCalled();
    expect(await within(speed).findByRole('status')).toHaveTextContent(
      'Whisper large-v3-turbo (q5_0) on Metal, 8 threads: 5.5 s of speech in 0.28 s (real time factor 0.05, recommended).'
    );
  });

  it('shows the time of the model in use only, and drops the old line while a new run is in progress', async () => {
    const user = userEvent.setup();
    serve(status({ models: both(), active_model: TURBO }));
    await renderSection();
    const speed = screen.getByTestId('speech-speed');
    await user.click(within(speed).getByRole('button', { name: copy.testSpeed }));
    expect(await within(speed).findByRole('status')).toHaveTextContent(
      'Whisper large-v3-turbo (q5_0) on Metal'
    );

    // Use small.en: turbo's time is not the time of the model in use.
    current = status({ models: both(), active_model: SMALL });
    await user.click(
      within(row(SMALL)).getByRole('button', { name: 'Use Whisper small.en (q8_0)' })
    );
    await waitFor(() => expect(within(speed).queryByRole('status')).not.toBeInTheDocument());

    // A run in progress shows no earlier line above Testing….
    let answer: (
      value: DictationResult<{ model: string; rtf: number; elapsed_ms: number }>
    ) => void = () => undefined;
    (modelsApi.bench as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(ok({ model: SMALL, rtf: 90 / 5500, elapsed_ms: 90 }))
      .mockImplementationOnce(() => new Promise((done) => (answer = done)));
    await user.click(within(speed).getByRole('button', { name: copy.testSpeed }));
    expect(await within(speed).findByRole('status')).toHaveTextContent(
      'Whisper small.en (q8_0) on Metal, 8 threads: 5.5 s of speech in 0.09 s'
    );
    await user.click(within(speed).getByRole('button', { name: copy.testSpeed }));
    expect(within(speed).getByRole('button', { name: copy.testing })).toBeDisabled();
    expect(within(speed).queryByRole('status')).not.toBeInTheDocument();
    await act(async () => answer(ok({ model: SMALL, rtf: 80 / 5500, elapsed_ms: 80 })));
    expect(await within(speed).findByRole('status')).toHaveTextContent('in 0.08 s');

    // small.en removed and turbo in use again: turbo's own time comes back, small.en's does not.
    current = status({ models: [both()[0], model(SMALL)], active_model: TURBO });
    await act(async () => {
      await removeSpeechModel(SMALL);
    });
    expect(await within(speed).findByRole('status')).toHaveTextContent(
      'Whisper large-v3-turbo (q5_0) on Metal, 8 threads: 5.5 s of speech in 0.28 s'
    );
  });

  it('holds Test speed until a model is in use', async () => {
    await renderSection();
    expect(screen.getByRole('button', { name: copy.testSpeed })).toBeDisabled();
  });

  it('holds Test speed while a dictation in this window runs, so a bench never sits on its path', async () => {
    const user = userEvent.setup();
    serve(
      status({
        models: [model(TURBO, { installed: true, location: 'user' }), model(SMALL)],
        active_model: TURBO,
      })
    );
    await renderSection();
    const button = within(screen.getByTestId('speech-speed')).getByRole('button', {
      name: copy.testSpeed,
    });
    expect(button).toBeEnabled();
    setDictating(true);
    expect(button).toBeDisabled();
    // The store refuses it too, whoever asks.
    await act(async () => {
      await testDictationSpeed();
    });
    expect(modelsApi.bench).not.toHaveBeenCalled();
    setDictating(false);
    expect(button).toBeEnabled();
    await user.click(button);
    expect(modelsApi.bench).toHaveBeenCalledTimes(1);
  });

  it('gives the section and its rows their help from the message catalog', async () => {
    await renderSection();
    for (const [label, id] of [
      [copy.section, 'MSG-SETTINGS-HELP'],
      [copy.folder, 'MSG-FOLDER-HELP'],
      [copy.language, 'MSG-LANGUAGE-HELP'],
      [copy.microphone, 'MSG-MIC-HELP'],
      [copy.speed, 'MSG-SPEED-HELP'],
    ] as const) {
      expect(
        screen.getByRole('button', { name: `About ${label}` }),
        label
      ).toHaveAccessibleDescription(DICTATION_MESSAGES[id]);
    }
  });
});
