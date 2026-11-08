import { useRef, useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CaptureOptions } from './capture';
import type { DictationStatus, StreamRequest } from './dictationApi';
import { resetDictationSessionForTests } from './dictationSession';
import { refreshDictationStatus, resetDictationStoreForTests } from './dictationStore';
import {
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  secureMediaPage,
  speechStatus,
  transcript,
} from './dictationTestKit';
import { useDictationComposer } from './useDictationComposer';

const mocks = vi.hoisted(() => ({
  browser: false,
  headers: vi.fn(),
  settings: vi.fn(),
  config: vi.fn(),
}));

vi.mock('../../utils/surface', async () => ({
  ...(await vi.importActual<typeof import('../../utils/surface')>('../../utils/surface')),
  isBrowserSurface: () => mocks.browser,
}));
vi.mock('../../utils/userAction', async () => ({
  ...(await vi.importActual<typeof import('../../utils/userAction')>('../../utils/userAction')),
  userActionHeaders: mocks.headers,
}));
vi.mock('../../api', async () => ({
  ...(await vi.importActual<typeof import('../../api')>('../../api')),
  getDictionarySettings: mocks.settings,
  readAllConfig: mocks.config,
}));

import { CrewDictationButton, crewPrivateTermsAllowed } from './CrewDictationButton';

let captures: FakeCapture[] = [];
let streams: FakeStream[] = [];
let status: DictationStatus;

function Host() {
  const field = useRef<HTMLTextAreaElement>(null);
  const [body, setBody] = useState('');
  const target = () => ({ target_kind: 'crew' as const });
  const control = useDictationComposer({
    key: 'crew:component-test',
    textAreaRef: field,
    storesDraft: false,
    target,
    setByState: (value) => setBody(value),
  });
  return (
    <div className="biorouter-composer-card">
      <textarea ref={field} value={body} onChange={(event) => setBody(event.target.value)} />
      <CrewDictationButton control={control} target={target} onSettings={vi.fn()} />
    </div>
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await act(async () => {});
}

async function openPrivateMenu() {
  fireEvent.contextMenu(screen.getByTestId('dictation-button'));
  return screen.findByRole('menuitem', { name: 'Include my private terms' });
}

beforeEach(() => {
  secureMediaPage();
  Object.assign(window, { electron: { platform: 'darwin' } });
  captures = [];
  streams = [];
  status = speechStatus();
  resetDictationStoreForTests(fakeStatusApi(() => status).api);
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
  mocks.browser = false;
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'desktop-proof' });
  mocks.config.mockReset().mockResolvedValue({
    data: { config: { dictation_dictionary_activation_version: 1 } },
  });
  mocks.settings.mockReset().mockResolvedValue({
    data: {
      suggest_from_chats: false,
      learn_from_chats: false,
      agent_writes: 'on_request',
      authority: { add_private: { allowed: true }, raise_settings: { allowed: false } },
    },
  });
});

afterEach(() => {
  cleanup();
  resetDictationSessionForTests();
  resetDictationStoreForTests();
});

describe('Crew private dictionary option', () => {
  it('appears only for a native caller with proof and private standing', async () => {
    expect(await crewPrivateTermsAllowed()).toBe(true);
    expect(mocks.settings).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'desktop-proof' },
    });

    mocks.headers.mockResolvedValueOnce({});
    expect(await crewPrivateTermsAllowed()).toBe(false);

    mocks.settings.mockResolvedValueOnce({
      data: { authority: { add_private: { allowed: false } } },
    });
    expect(await crewPrivateTermsAllowed()).toBe(false);

    mocks.browser = true;
    expect(await crewPrivateTermsAllowed()).toBe(false);
  });

  it('fails closed when the proof or authority read fails', async () => {
    mocks.headers.mockRejectedValueOnce(new Error('proof unavailable'));
    expect(await crewPrivateTermsAllowed()).toBe(false);
    mocks.settings.mockRejectedValueOnce(new Error('settings unavailable'));
    expect(await crewPrivateTermsAllowed()).toBe(false);
  });
});

describe('Crew microphone confirmation', () => {
  it.each([
    { key: 'F10', shiftKey: true },
    { key: 'ContextMenu', shiftKey: false },
  ])('opens the private menu from $key', async (key) => {
    render(<Host />);
    const mic = screen.getByTestId('dictation-button');
    fireEvent.focus(mic);
    await waitFor(() => expect(mocks.settings).toHaveBeenCalled());
    fireEvent.keyDown(mic, key);
    expect(await screen.findByRole('menuitem', { name: 'Include my private terms' })).toBeVisible();
  });

  it('requires the warning before Include and uses private terms for one recording', async () => {
    render(<Host />);
    fireEvent.pointerEnter(screen.getByTestId('dictation-button'));
    const item = await openPrivateMenu();
    expect(captures).toHaveLength(0);
    fireEvent.click(item);
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'Your private dictionary terms may appear in this text'
    );
    expect(captures).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: /^Include$/ }));
    await settle();
    expect(streams[0].request).toMatchObject({
      target_kind: 'crew',
      include_private_terms: true,
    });
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await act(async () => streams[0].finishDeferred.resolve(transcript('EGLN1')));
    await settle();
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await settle();
    expect(streams[1].request.target_kind).toBe('crew');
    expect(streams[1].request.include_private_terms).toBeUndefined();
  });

  it('Public terms only starts without private terms, while dismissing starts nothing', async () => {
    render(<Host />);
    fireEvent.pointerEnter(screen.getByTestId('dictation-button'));
    fireEvent.click(await openPrivateMenu());
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await settle();
    expect(captures).toHaveLength(0);

    fireEvent.click(await openPrivateMenu());
    fireEvent.click(screen.getByRole('button', { name: 'Public terms only' }));
    await settle();
    expect(streams[0].request.target_kind).toBe('crew');
    expect(streams[0].request.include_private_terms).toBeUndefined();
  });

  it('hides the item without proof or standing, including after a failed refresh', async () => {
    mocks.headers.mockResolvedValue({});
    render(<Host />);
    fireEvent.contextMenu(screen.getByTestId('dictation-button'));
    await settle();
    expect(screen.queryByRole('menuitem', { name: 'Include my private terms' })).toBeNull();
    expect(mocks.settings).not.toHaveBeenCalled();

    mocks.headers.mockResolvedValue({ 'X-User-Action': 'desktop-proof' });
    mocks.settings.mockResolvedValue({
      data: { authority: { add_private: { allowed: false } } },
    });
    fireEvent.contextMenu(screen.getByTestId('dictation-button'));
    await settle();
    expect(screen.queryByRole('menuitem', { name: 'Include my private terms' })).toBeNull();

    mocks.settings.mockRejectedValue(new Error('offline'));
    fireEvent.contextMenu(screen.getByTestId('dictation-button'));
    await settle();
    expect(screen.queryByRole('menuitem', { name: 'Include my private terms' })).toBeNull();
  });

  it('ignores an older allowed answer after a newer refusal', async () => {
    let answerOld: (value: unknown) => void = () => undefined;
    mocks.settings.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answerOld = resolve;
        })
    );
    mocks.settings.mockResolvedValueOnce({
      data: { authority: { add_private: { allowed: false } } },
    });
    render(<Host />);
    const mic = screen.getByTestId('dictation-button');
    fireEvent.pointerEnter(mic);
    await waitFor(() => expect(mocks.settings).toHaveBeenCalledTimes(1));
    fireEvent.contextMenu(mic);
    await waitFor(() => expect(mocks.settings).toHaveBeenCalledTimes(2));
    await act(async () =>
      answerOld({
        data: { authority: { add_private: { allowed: true } } },
      })
    );
    expect(screen.queryByRole('menuitem', { name: 'Include my private terms' })).toBeNull();
  });

  it('does not remember Include after a setup outcome', async () => {
    status = speechStatus({ active_model: null }, { installed: false });
    render(<Host />);
    fireEvent.pointerEnter(screen.getByTestId('dictation-button'));
    await settle();
    fireEvent.click(await openPrivateMenu());
    fireEvent.click(screen.getByRole('button', { name: /^Include$/ }));
    await settle();
    expect(streams).toHaveLength(0);
    expect(captures).toHaveLength(0);
    status = speechStatus();
    await act(async () => refreshDictationStatus());
    await settle();
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await settle();
    expect(streams[0].request.target_kind).toBe('crew');
    expect(streams[0].request.include_private_terms).toBeUndefined();
  });
});
