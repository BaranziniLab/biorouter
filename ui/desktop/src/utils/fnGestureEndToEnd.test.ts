/**
 * The fn gestures end to end, from the helper's lines to a recording in a composer (DD-98): fake
 * `biorouter-fnkey` lines go through the real `FnKeyService` (the gesture reducer and its gesture
 * ids), main's real `planFnIntent` (the per-gesture route latch, with the window's activity as its
 * indicator reports say), and this window's real fn client against the real dictation session,
 * whose capture and stream are fakes. What each layer decides alone has its own tests
 * (`fnGesture.test.ts`, `fnKeyHelper.test.ts`, `dictationFnKey.test.ts`, `fnGestureClient.test.ts`);
 * this one checks that they agree:
 *
 *  - a double-tap gives one hands-free recording, and the first tap's capture is discarded;
 *  - a second double-tap stops it, and the text goes into the composer;
 *  - a single tap leaves nothing running;
 *  - with an app that also uses fn running (DD-101), an Option+fn double-tap (the default
 *    modifier) still gives one hands-free recording, Option let go between the taps, and a bare
 *    fn hold starts nothing.
 */
import { EventEmitter } from 'node:events';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  attachDictationComposer,
  getDictationSnapshot,
  isDictationRunning,
  resetDictationSessionForTests,
  startDictation,
} from '../components/dictation/dictationSession';
import { resetDictationStoreForTests } from '../components/dictation/dictationStore';
import {
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  transcript,
} from '../components/dictation/dictationTestKit';
import {
  handleFnIntent,
  registerFnGestureTarget,
  resetFnGestureClientForTests,
} from '../components/dictation/fnGestureClient';
import { windowDictationActivity } from '../components/dictation/indicatorReport';
import { planFnIntent, type FnAction, type FnLatch, type FnWorld } from './dictationFnKey';
import type { FnIntent, FnNote } from './fnGesture';
import { FnKeyService, type HelperProcess } from './fnKeyHelper';

/** The helper process, as `fnKeyHelper.test.ts` fakes it: what it writes is what the test says. */
class FakeHelper extends EventEmitter implements HelperProcess {
  stdout = new EventEmitter();
  stdin = { write: () => true, end: () => undefined };
  kill() {}
  say(line: string) {
    this.stdout.emit('data', Buffer.from(`${line}\n`));
  }
}

const WINDOW = 1;
const KEY = 'tab:a';
// Protocol version 3 (DD-101); this was version 2, which the app now refuses.
const READY = '{"type":"ready","v":3,"permission":"granted","listening":true,"fnUsage":0,"t":0}';
/** What the helper reports when Wispr Flow runs: a listed app beside one Biorouter ignores. */
const WISPR_RUNS =
  '{"type":"listeners","apps":["com.example.hotkeys","com.electron.wispr-flow"],"t":0}';

let now = 0;
let helper: FakeHelper;
let service: FnKeyService;
let latch: FnLatch | null;
let actions: FnAction[];
let intents: Array<{ intent: FnIntent; gesture: number }>;
let notes: FnNote[];
let captures: FakeCapture[];
let streams: FakeStream[];
let field: HTMLTextAreaElement;

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

/** What main knows: this window is in front, and records when its indicator report says so. */
function world(): FnWorld {
  const activity = windowDictationActivity();
  return {
    focusedAppWindowId: WINDOW,
    recordingWindowId: activity === 'arming' || activity === 'recording' ? WINDOW : null,
    anywherePhase: 'off',
    anywhereFnOn: false,
  };
}

/** One helper line, read by main at `at` ms of its clock (the helper stamps the same time). */
async function line(at: number, json: string) {
  now = at;
  helper.say(json);
  await settle();
}
const fn = (at: number, down: boolean) => line(at, `{"type":"fn","down":${down},"t":${at}}`);

/** Two short presses, the second within the double-tap gap of the first release. */
async function doubleTap(at: number) {
  await fn(at, true);
  await fn(at + 90, false);
  await fn(at + 220, true);
  await fn(at + 310, false);
}

beforeEach(() => {
  now = 0;
  latch = null;
  actions = [];
  intents = [];
  notes = [];
  captures = [];
  streams = [];
  resetFnGestureClientForTests();
  resetDictationStoreForTests(fakeStatusApi(() => null).api);
  resetDictationSessionForTests({
    startCapture: (options) => {
      const capture = new FakeCapture(options);
      captures.push(capture);
      return Promise.resolve(capture);
    },
    prewarmCapture: vi.fn(),
    createStream: (request) => {
      const stream = new FakeStream(request);
      streams.push(stream);
      return stream;
    },
    statusSnapshot: () => ({ load: 'loaded', status: null, micStatus: 'granted' }),
    ensureStatus: async () => null,
    refreshStatus: async () => null,
    phase: () => ({ phase: 'idle' }),
    insertIntoDraft: vi.fn(),
  });
  Object.assign(window, {
    appConfig: { get: (key: string) => (key === 'BIOROUTER_LOCAL_BACKEND' ? true : undefined) },
    electron: { platform: 'darwin', setDictationBusy: () => undefined },
  });
  if (typeof document.execCommand !== 'function') {
    Object.assign(document, { execCommand: () => false });
  }
  vi.spyOn(document, 'execCommand').mockImplementation((_cmd, _ui, text) => {
    const target = document.activeElement as HTMLTextAreaElement;
    const start = target.selectionStart;
    target.value =
      target.value.slice(0, start) + (text ?? '') + target.value.slice(target.selectionEnd);
    return true;
  });

  // The chat composer, attached and offered to fn as `useDictationComposer` does it.
  field = document.createElement('textarea');
  document.body.append(field);
  attachDictationComposer({
    key: KEY,
    field: () => field,
    setByState: (value) => {
      field.value = value;
    },
    storesDraft: false,
  });
  registerFnGestureTarget(KEY, {
    press: () => void startDictation({ key: KEY, target: { target_kind: 'chat' } }),
    field: () => field,
  });
  field.focus();

  service = new FnKeyService({
    platform: 'darwin',
    helperPath: '/fake/biorouter-fnkey',
    spawn: () => (helper = new FakeHelper()),
    onIntent: (intent, gesture) => {
      intents.push({ intent, gesture });
      const plan = planFnIntent({ intent, gesture }, latch, world());
      latch = plan.latch;
      actions.push(...plan.actions);
      for (const action of plan.actions) {
        // Main sends the intent word to the window over `dictation:fn:intent`.
        if (action.kind === 'send' && action.windowId === WINDOW) handleFnIntent(action.intent);
      }
    },
    onStatus: () => undefined,
    onNote: (note) => notes.push(note),
    now: () => now,
  });
  service.setActive(true);
  helper.say(READY);
});

afterEach(() => {
  service.dispose();
  resetFnGestureClientForTests();
  resetDictationSessionForTests();
  resetDictationStoreForTests();
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('fn from the helper to the composer', () => {
  it('a double-tap gives one hands-free recording; a second double-tap stops it', async () => {
    await doubleTap(1000);
    expect(intents.map((step) => step.intent)).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    // One gesture, one id, from the first press to the toggle.
    expect(new Set(intents.map((step) => step.gesture)).size).toBe(1);
    // The first tap's capture was discarded; the second press's goes on, hands-free.
    expect(captures).toHaveLength(2);
    expect(captures[0].cancelled).toBe(true);
    expect(captures[1].cancelled).toBe(false);
    expect(captures[1].stopped).toBe(false);
    captures[1].live();
    expect(getDictationSnapshot()).toMatchObject({ key: KEY, activity: 'recording' });
    captures[1].speak(2);

    // Five seconds later, the stop.
    await doubleTap(6000);
    expect(intents.slice(4).map((step) => step.intent)).toEqual([
      'prime',
      'cancel-hold',
      'prime',
      'toggle',
    ]);
    // Its presses started nothing; its toggle stopped the recording.
    expect(captures).toHaveLength(2);
    expect(captures[1].stopped).toBe(true);
    expect(captures[1].cancelled).toBe(false);
    expect(getDictationSnapshot().activity).toBe('transcribing');
    streams[streams.length - 1].finishDeferred.resolve(transcript('EPAS1 levels'));
    await settle();
    expect(field.value).toBe('EPAS1 levels');
    expect(getDictationSnapshot()).toMatchObject({ activity: 'idle', announcement: 'Inserted' });
    expect(isDictationRunning()).toBe(false);
  });

  it('a single tap leaves nothing running', async () => {
    await fn(1000, true);
    await fn(1090, false);
    expect(intents.map((step) => step.intent)).toEqual(['prime', 'cancel-hold']);
    expect(captures).toHaveLength(1);
    expect(captures[0].cancelled).toBe(true);
    expect(getDictationSnapshot().activity).toBe('idle');
    expect(isDictationRunning()).toBe(false);
    // The wait for a second tap ends by itself: a press much later is a new gesture.
    await fn(3000, true);
    expect(intents[2]).toEqual({ intent: 'prime', gesture: intents[0].gesture + 1 });
  });

  it('a hold talks while held and puts the text in at the release', async () => {
    await fn(1000, true);
    await line(1300, '{"type":"held","t":1300}');
    captures[0].live();
    captures[0].speak(1);
    await fn(2500, false);
    expect(intents.map((step) => step.intent)).toEqual(['prime', 'start-hold', 'end-hold']);
    expect(captures).toHaveLength(1);
    expect(captures[0].stopped).toBe(true);
    streams[0].finishDeferred.resolve(transcript('hypoxia'));
    await settle();
    expect(field.value).toBe('hypoxia');
  });

  // The service's keys are never set here, so it uses the default modifier: Option since the
  // owner's ruling O-49 (2026-11-27). This test pressed Control+fn while O-46 made Control the
  // default, and was rewritten to Option.
  it('with Wispr Flow running, an Option+fn double-tap gives one hands-free recording; a second one stops it', async () => {
    await line(500, WISPR_RUNS);
    expect(service.status().collidingApp?.name).toBe('Wispr Flow');
    // Option held from before the first press, let go after it, and pressed again just after
    // fn went down for the second (a chord within the grace). The helper writes nothing for
    // Option between the taps.
    await line(1000, '{"type":"fn","down":true,"mod":"option","t":1000}');
    await line(1090, '{"type":"fn","down":false,"t":1090}');
    await line(1220, '{"type":"fn","down":true,"mod":null,"t":1220}');
    await line(1250, '{"type":"chord","mod":"option","t":1250}');
    await line(1310, '{"type":"fn","down":false,"t":1310}');
    expect(intents.map((step) => step.intent)).toEqual(['prime', 'cancel-hold', 'prime', 'toggle']);
    expect(new Set(intents.map((step) => step.gesture)).size).toBe(1);
    expect(captures).toHaveLength(2);
    expect(captures[0].cancelled).toBe(true);
    expect(captures[1].cancelled).toBe(false);
    expect(captures[1].stopped).toBe(false);
    captures[1].live();
    expect(getDictationSnapshot()).toMatchObject({ key: KEY, activity: 'recording' });
    captures[1].speak(2);

    // The stop, with Option held for both taps.
    for (const [at, json] of [
      [6000, '{"type":"fn","down":true,"mod":"option","t":6000}'],
      [6090, '{"type":"fn","down":false,"t":6090}'],
      [6220, '{"type":"fn","down":true,"mod":"option","t":6220}'],
      [6310, '{"type":"fn","down":false,"t":6310}'],
    ] as const)
      await line(at, json);
    expect(intents.slice(4).map((step) => step.intent)).toEqual([
      'prime',
      'cancel-hold',
      'prime',
      'toggle',
    ]);
    expect(captures).toHaveLength(2);
    expect(captures[1].stopped).toBe(true);
    streams[streams.length - 1].finishDeferred.resolve(transcript('EPAS1 levels'));
    await settle();
    expect(field.value).toBe('EPAS1 levels');
    expect(isDictationRunning()).toBe(false);
    expect(notes).toEqual([]);
  });

  it('with Wispr Flow running, a bare fn hold gives no action and one note', async () => {
    await line(500, WISPR_RUNS);
    await fn(1000, true);
    await line(1300, '{"type":"held","t":1300}');
    await fn(2500, false);
    expect(intents).toEqual([]);
    expect(actions).toEqual([]);
    expect(notes).toEqual(['ignored-bare-hold']);
    expect(captures).toHaveLength(0);
    expect(getDictationSnapshot().activity).toBe('idle');
    expect(isDictationRunning()).toBe(false);
  });
});
