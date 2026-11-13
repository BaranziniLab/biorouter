import React, { Profiler, useRef } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { resetCapturePrewarmForTests, type CaptureOptions } from './capture';
import { DICTATION_MESSAGES } from './copy';
import {
  DictationButton,
  DictationHoldReason,
  DictationNotices,
  levelToMeter,
} from './DictationButton';
import type { DictationStatus, StreamRequest } from './dictationApi';
import {
  getDictationSnapshot,
  requestDictationSetup,
  resetDictationSessionForTests,
  setDictationLevelForTests,
  setDictationSnapshotForTests,
} from './dictationSession';
import { refreshDictationStatus, resetDictationStoreForTests } from './dictationStore';
import { needsSharedOriginNotice, resetSharedOriginNoticeForTests } from './sharedOriginGate';
import {
  DICTATION_CONTEXT_BYTES,
  DICTATION_HOLD_MS,
  dictationContext,
  useDictationComposer,
} from './useDictationComposer';
import {
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  secureMediaPage,
  speechStatus,
  transcript,
} from './dictationTestKit';

const setupReads = vi.hoisted(() => ({
  config: vi.fn(),
  dictionary: vi.fn(),
}));

vi.mock('../../api', async () => ({
  ...(await vi.importActual<typeof import('../../api')>('../../api')),
  readAllConfig: setupReads.config,
  getDictionarySettings: setupReads.dictionary,
}));

let captures: FakeCapture[] = [];
let streams: FakeStream[] = [];
let status: DictationStatus | null = null;
let reads: ReturnType<typeof vi.fn>;
let statusGate: Promise<void> | null = null;

function setup() {
  captures = [];
  streams = [];
  const fake = fakeStatusApi(() => status);
  reads = fake.reads;
  const api = {
    ...fake.api,
    status: async (signal?: AbortSignal) => {
      if (statusGate) await statusGate;
      return fake.api.status(signal);
    },
  };
  resetDictationStoreForTests(api);
  resetDictationSessionForTests({
    startCapture: (options: CaptureOptions) => {
      const capture = new FakeCapture(options);
      captures.push(capture);
      return Promise.resolve(capture);
    },
    createStream: (request: StreamRequest) => {
      const stream = new FakeStream(request);
      streams.push(stream);
      return stream;
    },
  });
}

function Host({
  onRender,
  sessionId = 'chat-1',
  privatePress = false,
}: {
  onRender?: () => void;
  sessionId?: string;
  privatePress?: boolean;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = React.useState('');
  const control = useDictationComposer({
    key: 'tab:a',
    textAreaRef: ref,
    storesDraft: false,
    target: () => ({ target_kind: 'chat', session_id: sessionId }),
    setByState: (next) => setValue(next),
  });
  return (
    <div className="biorouter-composer-card">
      <DictationNotices dictationKey="tab:a" />
      <textarea
        data-testid="field"
        ref={ref}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        onFocus={control.noteInterest}
        onKeyDown={(event) => control.handleKeyDown(event)}
      />
      <Profiler id="mic" onRender={() => onRender?.()}>
        <DictationButton control={control} setView={vi.fn()} />
      </Profiler>
      {privatePress && (
        <button
          type="button"
          onClick={() =>
            control.pressWithTarget({ target_kind: 'crew', include_private_terms: true })
          }
        >
          Include once
        </button>
      )}
    </div>
  );
}

const mic = () => screen.getByTestId('dictation-button');
const settle = async () => {
  for (let i = 0; i < 6; i++) await act(async () => {});
};

beforeEach(() => {
  setupReads.config.mockReset().mockResolvedValue({
    data: { config: { dictation_dictionary_activation_version: 1 } },
  });
  setupReads.dictionary.mockReset().mockResolvedValue({
    data: {
      suggest_from_chats: false,
      learn_from_chats: false,
      agent_writes: 'on_request',
      authority: { raise_settings: { allowed: false } },
    },
  });
  resetSharedOriginNoticeForTests();
  secureMediaPage();
  Object.assign(window, { electron: { platform: 'darwin' } });
  status = speechStatus();
  statusGate = null;
  setup();
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  cleanup();
  resetDictationSessionForTests();
  resetDictationStoreForTests();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('the status read', () => {
  it('is not sent when the composer mounts', async () => {
    render(<Host />);
    await settle();
    expect(reads).not.toHaveBeenCalled();
  });

  it.each([
    ['the field gains focus', () => fireEvent.focus(screen.getByTestId('field'))],
    ['the pointer reaches the microphone', () => fireEvent.pointerEnter(mic())],
    ['the microphone takes focus', () => fireEvent.focus(mic())],
    ['the microphone is pressed', () => fireEvent.pointerDown(mic(), { button: 0 })],
  ])('loads once when %s (noteDictationInterest)', async (_name, act1) => {
    render(<Host />);
    act1();
    await settle();
    act1();
    await settle();
    expect(reads).toHaveBeenCalledTimes(1);
  });

  it.each([
    ['a press on the microphone', () => fireEvent.pointerDown(mic(), { button: 0 })],
    [
      'the chord as the first key',
      () =>
        fireEvent.keyDown(screen.getByTestId('field'), {
          key: 'D',
          code: 'KeyD',
          metaKey: true,
          shiftKey: true,
        }),
    ],
  ])('%s starts the capture before it notes interest', async (_name, act1) => {
    // Interest noted first would make the capture context inside the press, beside
    // getUserMedia, which holds the device back (capture.ts).
    resetCapturePrewarmForTests();
    const order: string[] = [];
    (navigator.mediaDevices.enumerateDevices as ReturnType<typeof vi.fn>).mockImplementation(
      async () => {
        order.push('interest');
        return [];
      }
    );
    resetDictationSessionForTests({
      startCapture: (options: CaptureOptions) => {
        order.push('capture');
        const capture = new FakeCapture(options);
        captures.push(capture);
        return Promise.resolve(capture);
      },
      createStream: (request: StreamRequest) => {
        const stream = new FakeStream(request);
        streams.push(stream);
        return stream;
      },
    });
    render(<Host />);
    await settle();
    act1();
    await settle();
    expect(order).toEqual(['capture', 'interest']);
  });
});

describe('phases and names', () => {
  it('reads Dictate with the chord, and is not pressed, at rest', async () => {
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'idle');
    expect(mic()).toHaveAccessibleName('Dictate');
    expect(mic()).toHaveAttribute('aria-keyshortcuts', 'Meta+Shift+D');
    expect(mic()).toHaveAttribute('aria-pressed', 'false');
  });

  it('arms, records with its timer, then transcribes and inserts nothing it was not given', async () => {
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(mic()).toHaveAttribute('data-dictation-phase', 'arming');
    expect(mic()).toHaveAccessibleName('Starting microphone…');
    expect(mic()).toHaveAttribute('aria-pressed', 'true');
    expect(mic()).toHaveAttribute('aria-busy', 'true');
    await settle();
    act(() => captures[0].live());
    expect(mic()).toHaveAttribute('data-dictation-phase', 'recording');
    act(() => captures[0].speak(2.2));
    expect(mic()).toHaveAccessibleName('Stop dictation, 0:02');
    expect(screen.getByTestId('dictation-timer')).toHaveTextContent('0:02');
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(mic()).toHaveAttribute('data-dictation-phase', 'transcribing');
    expect(mic()).toHaveAccessibleName('Transcribing…');
    // A press while transcribing does nothing.
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(streams[0].finishCalls).toBe(1);
    await act(async () => streams[0].finishDeferred.resolve(transcript('EPAS1')));
    await settle();
    expect(screen.getByTestId('field')).toHaveValue('EPAS1');
    expect(mic()).toHaveAttribute('data-dictation-phase', 'idle');
  });

  it('opens the setup popover before a model is installed, and downloads nothing on that press', async () => {
    status = speechStatus({ active_model: null }, { installed: false });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'not-installed');
    expect(mic()).toHaveAccessibleName('Set up dictation');
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    expect(screen.getByTestId('dictation-setup')).toHaveTextContent(
      'It needs a speech model: Whisper small.en (Light) (264 MB)'
    );
    expect(captures).toHaveLength(0);
  });

  it('shows the daemon install lock after the first browser acknowledgement', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    status = speechStatus(
      {
        active_model: null,
        install: { allowed: false, reason: DICTATION_MESSAGES['MSG-INSTALL-LOCKED'] },
      },
      { installed: false }
    );
    render(<Host />);
    await settle();
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    expect(screen.getByTestId('shared-origin-acknowledge')).toBeVisible();
    expect(screen.queryByTestId('dictation-setup')).toBeNull();
    expect(captures).toHaveLength(0);
    expect(streams).toHaveLength(0);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('shared-origin-acknowledge'));
    await settle();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'unavailable');
    expect(mic()).toHaveAccessibleName(DICTATION_MESSAGES['MSG-INSTALL-LOCKED']);
    expect(mic()).toBeDisabled();
    expect(screen.queryByTestId('dictation-setup')).toBeNull();
    expect(streams).toHaveLength(0);
  });

  it('shows the daemon install lock before a hovered browser microphone can be pressed', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    status = speechStatus(
      {
        active_model: null,
        install: { allowed: false, reason: DICTATION_MESSAGES['MSG-INSTALL-LOCKED'] },
      },
      { installed: false }
    );
    render(<Host />);
    fireEvent.pointerEnter(mic());
    await settle();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'unavailable');
    expect(mic()).toHaveAccessibleName(DICTATION_MESSAGES['MSG-INSTALL-LOCKED']);
    expect(mic()).toBeDisabled();
    expect(screen.queryByTestId('shared-origin-acknowledge')).toBeNull();
    expect(captures).toHaveLength(0);
    expect(streams).toHaveLength(0);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
  });

  it('opens the popover on a setup request while the status still reads idle (a 409 noModel)', async () => {
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    act(() => requestDictationSetup('tab:a'));
    await settle();
    expect(screen.getByTestId('dictation-setup')).toBeInTheDocument();
  });

  it('closes the popover once the install it was opened for gives a usable model', async () => {
    status = speechStatus({ active_model: null }, { installed: false });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    expect(screen.getByTestId('dictation-setup')).toBeInTheDocument();
    status = speechStatus();
    await act(async () => void (await refreshDictationStatus()));
    await settle();
    expect(screen.queryByTestId('dictation-setup')).toBeNull();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'idle');
  });

  it('hidden by the viewer, a chord that finds no model shows the button with the popover, then hides it', async () => {
    localStorage.setItem('biorouter:dictation-viewer', JSON.stringify({ showButton: false }));
    try {
      status = speechStatus({ active_model: null }, { installed: false });
      render(<Host />);
      const field = screen.getByTestId('field');
      fireEvent.focus(field);
      await settle();
      expect(screen.queryByTestId('dictation-button')).toBeNull();
      fireEvent.keyDown(field, { code: 'KeyD', key: 'D', metaKey: true, shiftKey: true });
      await settle();
      expect(screen.getByTestId('dictation-setup')).toBeInTheDocument();
      expect(mic()).toHaveAttribute('data-dictation-phase', 'not-installed');
      expect(captures).toHaveLength(0);
      fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
      await settle();
      expect(screen.queryByTestId('dictation-setup')).toBeNull();
      expect(screen.queryByTestId('dictation-button')).toBeNull();
    } finally {
      localStorage.removeItem('biorouter:dictation-viewer');
    }
  });

  it('is disabled with the reason as its name when this build has no engine', async () => {
    status = speechStatus({
      engine: { ...speechStatus().engine, compiled: false },
    });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toBeDisabled();
    expect(mic()).toHaveAccessibleName('This build of Biorouter has no local dictation engine.');
  });

  it('stays in place, disabled, with the policy sentence as its reason when policy turns capture off', async () => {
    const reason = DICTATION_MESSAGES['MSG-POLICY-CAPTURE'];
    status = speechStatus({
      policy: { ...speechStatus().policy, capture: { allowed: false, reason } },
    });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toBeInTheDocument();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'unavailable');
    expect(mic()).toBeDisabled();
    expect(mic()).toHaveAccessibleName(reason);
    // The disabled button gets no pointer events; the focusable slot around it carries the
    // reason, and its tooltip says it before the control is touched (here reached with Tab).
    const slot = mic().closest('span[tabindex="0"]') as HTMLElement;
    expect(slot).toHaveAccessibleName(reason);
    fireEvent.keyDown(document.body, { key: 'Tab' });
    act(() => slot.focus());
    expect(await screen.findByRole('tooltip')).toHaveTextContent(reason);
    // A press starts nothing, and the chord says the same sentence in the notice row.
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    expect(captures).toHaveLength(0);
    fireEvent.keyDown(screen.getByTestId('field'), {
      code: 'KeyD',
      key: 'D',
      metaKey: true,
      shiftKey: true,
    });
    await settle();
    expect(captures).toHaveLength(0);
    expect(getDictationSnapshot().notice?.text).toBe(reason);
    expect(mic()).toBeDisabled();
  });

  it('stays disabled with MSG-POLICY when policy turns dictation off', async () => {
    status = speechStatus({
      policy: {
        ...speechStatus().policy,
        enabled: { allowed: false, reason: DICTATION_MESSAGES['MSG-POLICY'] },
      },
    });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toBeDisabled();
    expect(mic()).toHaveAccessibleName(DICTATION_MESSAGES['MSG-POLICY']);
  });

  // Checklist row S17.4: an external backend's status carries the backend host's policy, so this
  // computer's own comes from Electron main, and the microphone says it before a press.
  it("stays disabled with this computer's policy sentence when only the laptop's policy turns capture off", async () => {
    const reason = DICTATION_MESSAGES['MSG-POLICY-CAPTURE'];
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        dictationAnywhereGet: vi.fn(async () => ({
          enabled: false,
          shortcut: null,
          disclosureShown: false,
          tray: true,
          gnomeWayland: false,
          captureAllowed: false,
          capturePolicyReason: 'MSG-POLICY-CAPTURE',
        })),
      },
    });
    render(<Host />);
    fireEvent.focus(screen.getByTestId('field'));
    await settle();
    expect(mic()).toHaveAttribute('data-dictation-phase', 'unavailable');
    expect(mic()).toBeDisabled();
    expect(mic()).toHaveAccessibleName(reason);
    fireEvent.keyDown(screen.getByTestId('field'), {
      code: 'KeyD',
      key: 'D',
      metaKey: true,
      shiftKey: true,
    });
    await settle();
    expect(captures).toHaveLength(0);
    expect(getDictationSnapshot().notice?.text).toBe(reason);
  });
});

describe('gestures', () => {
  it('shows the shared origin notice before the first browser capture and starts on its acknowledgement click', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<Host />);
    await settle();
    expect(mic()).not.toHaveAttribute('aria-keyshortcuts');
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(captures).toHaveLength(0);
    expect(
      screen.getByText(/Your browser's microphone permission covers every page at/)
    ).toBeVisible();
    fireEvent.click(screen.getByTestId('shared-origin-acknowledge'));
    expect(captures).toHaveLength(1);
    expect(needsSharedOriginNotice()).toBe(false);
  });

  it('starts browser capture after acknowledgement when local storage is blocked', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Blocked', 'SecurityError');
    });
    render(<Host />);
    await settle();
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(captures).toHaveLength(0);
    fireEvent.click(screen.getByTestId('shared-origin-acknowledge'));
    expect(captures).toHaveLength(1);
    expect(needsSharedOriginNotice()).toBe(false);
  });

  it('leaves the dictation shortcut to the browser', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<Host />);
    await settle();
    const handled = fireEvent.keyDown(screen.getByTestId('field'), {
      key: 'D',
      code: 'KeyD',
      metaKey: true,
      shiftKey: true,
    });
    expect(handled).toBe(true);
    expect(captures).toHaveLength(0);
  });

  it('starts on pointerdown, never on a pointer click', async () => {
    render(<Host />);
    await settle();
    fireEvent.click(mic(), { detail: 1 });
    expect(captures).toHaveLength(0);
    fireEvent.pointerDown(mic(), { button: 0 });
    // Capture starts inside the press, before any await.
    expect(captures).toHaveLength(1);
  });

  it('presses on a click from the keyboard, which has no pointer down', async () => {
    render(<Host />);
    await settle();
    fireEvent.click(mic(), { detail: 0 });
    expect(captures).toHaveLength(1);
  });

  it('keeps the focus in the field: its mousedown is cancelled', async () => {
    render(<Host />);
    const field = screen.getByTestId('field');
    field.focus();
    expect(fireEvent.mouseDown(mic())).toBe(false);
  });

  const chord = (type: 'keyDown' | 'keyUp', target: Element) =>
    fireEvent[type](target, { code: 'KeyD', key: 'D', metaKey: true, shiftKey: true });

  it('a tap of the chord toggles: shorter than 500 ms leaves the recording on', async () => {
    let now = 1_000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
    render(<Host />);
    const field = screen.getByTestId('field');
    chord('keyDown', field);
    expect(getDictationSnapshot().activity).toBe('arming');
    now += DICTATION_HOLD_MS - 1;
    chord('keyUp', window as unknown as Element);
    await settle();
    act(() => captures[0].live());
    expect(getDictationSnapshot().activity).toBe('recording');
    // A second tap stops it.
    act(() => captures[0].speak(1));
    chord('keyDown', field);
    expect(getDictationSnapshot().activity).toBe('transcribing');
  });

  it('a hold of 500 ms or more talks while held, and the key up stops and inserts', async () => {
    let now = 1_000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
    render(<Host />);
    const field = screen.getByTestId('field');
    chord('keyDown', field);
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    now += DICTATION_HOLD_MS;
    fireEvent.keyUp(window, { key: 'Meta', code: 'MetaLeft' });
    expect(getDictationSnapshot().activity).toBe('transcribing');
  });

  it('a window blur during a hold stops and inserts', async () => {
    render(<Host />);
    chord('keyDown', screen.getByTestId('field'));
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    fireEvent.blur(window);
    expect(getDictationSnapshot().activity).toBe('transcribing');
  });

  it('ignores a repeat of the chord and Ctrl+Shift+D on a Mac', async () => {
    render(<Host />);
    const field = screen.getByTestId('field');
    fireEvent.keyDown(field, { code: 'KeyD', metaKey: true, shiftKey: true, repeat: true });
    fireEvent.keyDown(field, { code: 'KeyD', ctrlKey: true, shiftKey: true });
    expect(captures).toHaveLength(0);
  });

  it('Escape while recording cancels, inserts nothing, and is marked handled', async () => {
    render(<Host />);
    const field = screen.getByTestId('field');
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    expect(fireEvent.keyDown(field, { key: 'Escape' })).toBe(false);
    expect(getDictationSnapshot().activity).toBe('idle');
    expect(captures[0].cancelled).toBe(true);
    expect(streams[0].cancelled).toBe(true);
  });

  it('Enter while recording stops and inserts, and is marked handled', async () => {
    render(<Host />);
    const field = screen.getByTestId('field');
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    expect(fireEvent.keyDown(field, { key: 'Enter' })).toBe(false);
    expect(getDictationSnapshot().activity).toBe('transcribing');
  });
});

describe('the status has not loaded yet', () => {
  it('a first press still reaches recording, capturing before the status answers', async () => {
    let open!: () => void;
    statusGate = new Promise<void>((resolve) => (open = resolve));
    render(<Host />);
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(captures).toHaveLength(1);
    expect(streams).toHaveLength(0);
    act(() => captures[0].live());
    await act(async () => open());
    await settle();
    expect(streams).toHaveLength(1);
    expect(getDictationSnapshot().activity).toBe('recording');
  });

  it('so does the chord', async () => {
    let open!: () => void;
    statusGate = new Promise<void>((resolve) => (open = resolve));
    render(<Host />);
    fireEvent.keyDown(screen.getByTestId('field'), {
      code: 'KeyD',
      metaKey: true,
      shiftKey: true,
    });
    expect(captures).toHaveLength(1);
    await act(async () => open());
    await settle();
    act(() => captures[0].live());
    expect(streams).toHaveLength(1);
    expect(getDictationSnapshot().activity).toBe('recording');
  });
});

describe('the level bars', () => {
  it('read the level in their animation frame, with no React render per chunk', async () => {
    type Frame = (time: number) => void;
    const frames: Frame[] = [];
    vi.stubGlobal('requestAnimationFrame', (cb: Frame) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => {});
    let renders = 0;
    render(<Host onRender={() => renders++} />);
    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    act(() => captures[0].live());
    const pill = mic().querySelector('.br-dictation-pill') as HTMLElement;
    const before = renders;
    for (const level of [0.001, 0.05, 0.3]) {
      setDictationLevelForTests(level);
      frames.shift()?.(0);
    }
    expect(renders).toBe(before);
    expect(Number(pill.style.getPropertyValue('--br-dictation-level'))).toBeGreaterThan(0);
    vi.unstubAllGlobals();
  });

  it('map -60 dBFS to empty and full scale to full', () => {
    expect(levelToMeter(0)).toBe(0);
    expect(levelToMeter(0.001)).toBeCloseTo(0, 5);
    expect(levelToMeter(1)).toBe(1);
    expect(levelToMeter(0.0316)).toBeCloseTo(0.5, 1);
  });
});

describe('why Send waits', () => {
  it('says Finish dictating first while recording, and Transcribing… after the stop', () => {
    render(<DictationHoldReason dictationKey="tab:a" />);
    act(() => setDictationSnapshotForTests({ key: 'tab:a', activity: 'recording' }));
    expect(document.body).toHaveTextContent('Finish dictating first');
    act(() => setDictationSnapshotForTests({ key: 'tab:a', activity: 'transcribing' }));
    expect(document.body).toHaveTextContent('Transcribing…');
  });
});

describe('dictationContext', () => {
  it('keeps the end of the draft, at most 2 KiB of UTF-8, on a character boundary', () => {
    expect(dictationContext('short')).toBe('short');
    const long = 'é'.repeat(2000);
    const cut = dictationContext(long);
    expect(new TextEncoder().encode(cut).length).toBeLessThanOrEqual(DICTATION_CONTEXT_BYTES);
    expect(cut).toBe('é'.repeat(1024));
    expect(dictationContext('ab😀', 5)).toBe('b😀');
  });
});

describe('one recording with Crew private terms', () => {
  it('does not carry a confirmed target into the next ordinary press', async () => {
    render(<Host privatePress />);
    fireEvent.click(screen.getByRole('button', { name: 'Include once' }));
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    expect(streams[0].request).toMatchObject({
      target_kind: 'crew',
      include_private_terms: true,
    });
    fireEvent.pointerDown(mic(), { button: 0 });
    await act(async () => streams[0].finishDeferred.resolve(transcript('first')));
    await settle();

    fireEvent.pointerDown(mic(), { button: 0 });
    await settle();
    expect(streams[1].request.target_kind).toBe('chat');
    expect(streams[1].request.include_private_terms).toBeUndefined();
  });
});
