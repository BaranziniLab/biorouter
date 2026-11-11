/** The served composer records in Chromium and Firefox through the real stream routes. */
import { chromium, expect, firefox, test } from '@playwright/test';
import type { Browser, Page, Response } from '@playwright/test';
import { spawn } from 'node:child_process';
import type { ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { join, resolve } from 'node:path';

const REQUIRED_BIN = process.env.BIOROUTER_SERVE_BIN;
const SERVE_BIN = REQUIRED_BIN ?? resolve(__dirname, '../../../../target/debug/biorouter');
const CHANNEL = process.env.BIOROUTER_PW_CHANNEL || undefined;
const WEB_DIR = resolve(__dirname, '../../src/web');
const TEST_MODEL = resolve(
  __dirname,
  '../../../../vendor/whisper.cpp/source/models/for-tests-ggml-base.en.bin'
);
const TEST_DETECTOR = resolve(
  __dirname,
  '../../../../vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin'
);
const TOKEN = 'pr21-browser-dictation-fixture';
const HOST_PROVIDER = 'versa_azure';
const HOST_MODEL = 'gpt-5.5-2026-04-24';
const SILENT = 'The microphone sent only silence. Check that it is not muted.';
const NOTHING = 'Nothing was heard, so nothing was inserted.';
const NOTICE_START = "Your browser's microphone permission covers every page at";

async function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const listener = createServer();
    listener.once('error', reject);
    listener.listen(0, '127.0.0.1', () => {
      const address = listener.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      listener.close(() => resolvePort(port));
    });
  });
}

function writeSilentWav(path: string): void {
  const frames = 16000 * 5;
  const pcmBytes = frames * 2;
  const wav = Buffer.alloc(44 + pcmBytes);
  wav.write('RIFF', 0);
  wav.writeUInt32LE(36 + pcmBytes, 4);
  wav.write('WAVEfmt ', 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(16000, 24);
  wav.writeUInt32LE(32000, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write('data', 36);
  wav.writeUInt32LE(pcmBytes, 40);
  writeFileSync(path, wav);
}

async function waitForServe(origin: string, child: ChildProcess): Promise<void> {
  for (let attempt = 0; attempt < 120; attempt++) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error('The owned serve process exited before the browser check.');
    }
    try {
      const response = await fetch(`${origin}/status`);
      if (response.status > 0) return;
    } catch {
      // Startup has not opened the port yet.
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  throw new Error('The owned serve process did not answer within 60 seconds.');
}

async function stopServe(child: ChildProcess | null): Promise<void> {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>((resolveExit) => child.once('exit', () => resolveExit()));
  child.kill('SIGTERM');
  await Promise.race([
    exited,
    new Promise<void>((resolveDelay) => setTimeout(resolveDelay, 15_000)),
  ]);
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
}

async function showReadyFixture(page: Page): Promise<void> {
  // The debug model is a pinned test fixture, not a catalog install. Only the
  // status inventory is presented as installed; capture and every stream route
  // still run through the real served daemon and pinned test-model decoder.
  await page.route('**/dictation/status', async (route) => {
    const response = await route.fetch();
    if (response.status() !== 200) {
      await route.fulfill({ response });
      return;
    }
    const status = await response.json();
    const model = status.models?.find((entry: { id: string }) => entry.id.includes('small.en'));
    if (!model) throw new Error('The daemon status has no small.en catalog row.');
    model.installed = true;
    model.location = 'user';
    status.active_model = model.id;
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(status),
    });
  });
}

async function openShell(page: Page, origin: string): Promise<void> {
  await page.goto(`${origin}/?t=${encodeURIComponent(TOKEN)}`);
  await page.waitForURL(`${origin}/`);
  const providerReady = await page.evaluate(
    async ({ provider, model }) => {
      const secret = (
        window as Window & {
          __BIOROUTER_HEADLESS_CONFIG__?: { secretKey?: string };
        }
      ).__BIOROUTER_HEADLESS_CONFIG__?.secretKey;
      if (!secret) return { providerPinned: false, modelPinned: false, configured: false };
      const headers = { 'X-Secret-Key': secret, 'Content-Type': 'application/json' };
      const read = async (key: string) => {
        const response = await fetch('/config/read', {
          method: 'POST',
          headers,
          body: JSON.stringify({ key, is_secret: false }),
        });
        return response.ok ? response.json() : null;
      };
      const catalog = await fetch('/config/providers', { headers });
      const providers = catalog.ok ? await catalog.json() : [];
      return {
        providerPinned: (await read('BIOROUTER_PROVIDER')) === provider,
        modelPinned: (await read('BIOROUTER_MODEL')) === model,
        configured: providers.some(
          (entry: { name?: string; is_configured?: boolean }) =>
            entry.name === provider && entry.is_configured === true
        ),
      };
    },
    { provider: HOST_PROVIDER, model: HOST_MODEL }
  );
  expect(providerReady).toEqual({ providerPinned: true, modelPinned: true, configured: true });
  await expect(page.getByTestId('dictation-button').first()).toBeVisible({ timeout: 30_000 });
}

type StreamEvidence = {
  rates: number[];
  createdStreamIds: string[];
  audioRequestAt: number[];
  audioStreamIds: string[];
  pcmBytes: number;
  finishRequests: number;
  heldReads: number;
  sseRequests: number;
  transcript: { audio_s: number; no_speech: boolean } | null;
};

function observeStream(page: Page): StreamEvidence {
  const evidence: StreamEvidence = {
    rates: [],
    createdStreamIds: [],
    audioRequestAt: [],
    audioStreamIds: [],
    pcmBytes: 0,
    finishRequests: 0,
    heldReads: 0,
    sseRequests: 0,
    transcript: null,
  };
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      url.pathname.startsWith('/dictation/streams/') &&
      request.headers().accept?.includes('text/event-stream')
    )
      evidence.sseRequests += 1;
    if (request.method() === 'POST' && url.pathname.endsWith('/dictation/streams')) {
      const body = JSON.parse(request.postData() ?? '{}');
      if (typeof body.sample_rate === 'number') evidence.rates.push(body.sample_rate);
    }
    const audioPath = /\/dictation\/streams\/([^/]+)\/audio$/.exec(url.pathname);
    if (request.method() === 'PUT' && audioPath) {
      evidence.audioRequestAt.push(Date.now());
      evidence.audioStreamIds.push(audioPath[1]);
      evidence.pcmBytes += request.postDataBuffer()?.byteLength ?? 0;
    }
    if (request.method() === 'POST' && /\/dictation\/streams\/[^/]+\/finish$/.test(url.pathname)) {
      evidence.finishRequests += 1;
      evidence.pcmBytes += request.postDataBuffer()?.byteLength ?? 0;
    }
    if (request.method() === 'GET' && /\/dictation\/streams\/[^/]+$/.test(url.pathname)) {
      evidence.heldReads += 1;
    }
  });
  page.on('response', (response: Response) => {
    const path = new URL(response.url()).pathname;
    if (
      response.request().method() === 'POST' &&
      path.endsWith('/dictation/streams') &&
      response.status() === 201
    ) {
      void response
        .json()
        .then((body) => {
          if (typeof body.stream_id === 'string') evidence.createdStreamIds.push(body.stream_id);
        })
        .catch(() => undefined);
      return;
    }
    if (!/\/dictation\/streams\/[^/]+(?:\/finish)?$/.test(path) || response.status() !== 200)
      return;
    void response
      .json()
      .then((body) => {
        if (typeof body.audio_s === 'number' && typeof body.no_speech === 'boolean') {
          evidence.transcript = { audio_s: body.audio_s, no_speech: body.no_speech };
        }
      })
      .catch(() => undefined);
  });
  return evidence;
}

async function acknowledgeOrigin(page: Page): Promise<void> {
  await expect(page.getByText(NOTICE_START, { exact: false })).toBeVisible();
  await page.getByTestId('shared-origin-acknowledge').click();
}

async function micCallCount(page: Page): Promise<number> {
  return page.evaluate(() => (window as Window & { __pr21MicCalls?: number }).__pr21MicCalls ?? 0);
}

async function captureNothing(
  page: Page,
  acknowledge: boolean,
  evidence: StreamEvidence,
  firefoxFakeAudio: boolean
): Promise<{
  startGestureToRecordingPhaseMs: number;
  stopToNoticeMs: number;
  outcome: 'silent' | 'no_speech';
}> {
  const mic = page.getByTestId('dictation-button').first();
  evidence.transcript = null;
  const streamsBeforePress = evidence.rates.length;
  const createdStreamsBeforePress = evidence.createdStreamIds.length;
  const audioRequestsBeforePress = evidence.audioRequestAt.length;
  const finishRequestsBeforePress = evidence.finishRequests;
  const heldReadsBeforePress = evidence.heldReads;
  const sseRequestsBeforePress = evidence.sseRequests;
  const micCallsBeforePress = await micCallCount(page);
  let startedAt = Date.now();
  await mic.click();
  if (acknowledge) {
    await expect(mic).toHaveAttribute('data-dictation-phase', 'idle');
    expect(evidence.rates).toHaveLength(streamsBeforePress);
    expect(evidence.audioRequestAt).toHaveLength(audioRequestsBeforePress);
    expect(await micCallCount(page)).toBe(micCallsBeforePress);
    await expect(page.getByText(NOTICE_START, { exact: false })).toBeVisible();
    startedAt = Date.now();
    await page.getByTestId('shared-origin-acknowledge').click();
  } else await expect(page.getByText(NOTICE_START, { exact: false })).toHaveCount(0);
  try {
    await expect(mic).toHaveAttribute('data-dictation-phase', 'recording', { timeout: 30_000 });
  } catch (error) {
    if (!firefoxFakeAudio) throw error;
    const diagnostic = await page.evaluate(() => {
      const probe = window as Window & {
        __pr21MicProbe?: { calls: number; settled: boolean };
        __pr21AudioState?: string;
      };
      const phase = document
        .querySelector('[data-testid="dictation-button"]')
        ?.getAttribute('data-dictation-phase');
      return {
        getUserMediaCalls: probe.__pr21MicProbe?.calls ?? 0,
        getUserMediaSettled: probe.__pr21MicProbe?.settled ?? false,
        audioContextState: probe.__pr21AudioState ?? 'none',
        capturePhase:
          phase && ['idle', 'arming', 'recording', 'error'].includes(phase) ? phase : 'unknown',
      };
    });
    throw new Error(`Firefox capture did not reach recording: ${JSON.stringify(diagnostic)}`);
  }
  const startGestureToRecordingPhaseMs = Date.now() - startedAt;
  await expect.poll(() => micCallCount(page)).toBeGreaterThan(micCallsBeforePress);
  const currentStreamHasAudio = () =>
    evidence.createdStreamIds
      .slice(createdStreamsBeforePress)
      .some((id) => evidence.audioStreamIds.slice(audioRequestsBeforePress).includes(id));
  await expect.poll(currentStreamHasAudio, { timeout: 30_000 }).toBe(true);
  const stoppedAt = Date.now();
  await mic.click();
  const notice = page
    .getByTestId('dictation-notice')
    .first()
    .locator('span.text-supporting')
    .first();
  await expect
    .poll(
      async () => {
        const text = (await notice.textContent())?.trim();
        return text === SILENT || (firefoxFakeAudio && text === NOTHING);
      },
      { timeout: 120_000 }
    )
    .toBe(true);
  const stopToNoticeMs = Date.now() - stoppedAt;
  const outcome = (await notice.textContent())?.trim() === SILENT ? 'silent' : 'no_speech';
  const inputHasText = await page
    .getByTestId('chat-input')
    .first()
    .evaluate((element) => {
      const value = 'value' in element ? element.value : element.textContent;
      return Boolean(value?.trim());
    });
  expect(inputHasText).toBe(false);
  expect(evidence.rates.length).toBeGreaterThan(streamsBeforePress);
  expect(currentStreamHasAudio()).toBe(true);
  expect(evidence.sseRequests).toBe(sseRequestsBeforePress);
  if (outcome === 'silent') {
    expect(evidence.finishRequests).toBe(finishRequestsBeforePress);
    expect(evidence.heldReads).toBe(heldReadsBeforePress);
    expect(evidence.transcript).toBeNull();
  } else {
    expect(evidence.finishRequests).toBeGreaterThan(finishRequestsBeforePress);
    await expect.poll(() => evidence.transcript).not.toBeNull();
    expect(evidence.transcript!.no_speech).toBe(true);
  }
  return { startGestureToRecordingPhaseMs, stopToNoticeMs, outcome };
}

for (const [name, nativeFallback] of [
  ['Chromium', false],
  ['Firefox', true],
] as Array<[string, boolean]>) {
  test(`${name}: served composer records PCM and shows the origin notice once`, async ({}, testInfo) => {
    test.setTimeout(180_000);
    for (const path of [SERVE_BIN, WEB_DIR, TEST_MODEL, TEST_DETECTOR]) {
      if (!existsSync(path)) throw new Error(`PR-21 serve browser prerequisite missing: ${path}`);
    }
    const root = testInfo.outputPath('serve-root');
    mkdirSync(root, { recursive: true });
    const temp = testInfo.outputPath('tmp');
    mkdirSync(temp, { recursive: true });
    const wav = testInfo.outputPath('silence.wav');
    writeSilentWav(wav);
    const port = await freePort();
    const origin = `http://127.0.0.1:${port}`;
    const child = spawn(SERVE_BIN, ['serve', '--port', String(port), '--web-dir', WEB_DIR], {
      env: {
        PATH: process.env.PATH ?? '/usr/bin:/bin',
        HOME: root,
        TMPDIR: temp,
        BIOROUTER_PATH_ROOT: root,
        BIOROUTER_DISABLE_KEYRING: 'true',
        BIOROUTER_BROWSER_TOKEN: TOKEN,
        BIOROUTER_DICTATION_TEST_MODEL: TEST_MODEL,
        BIOROUTER_PROVIDER: HOST_PROVIDER,
        BIOROUTER_MODEL: HOST_MODEL,
        VERSA_AZURE_API_KEY: 'pr21-test-only-no-network',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    child.stdout?.resume();
    child.stderr?.resume();
    let browser: Browser | null = null;
    try {
      await waitForServe(origin, child);
      browser = nativeFallback
        ? await firefox.launch({
            firefoxUserPrefs: {
              'media.navigator.streams.fake': true,
              'media.navigator.permission.disabled': true,
            },
          })
        : await chromium.launch({
            channel: CHANNEL,
            args: [
              '--use-fake-device-for-media-stream',
              `--use-file-for-fake-audio-capture=${wav}%noloop`,
            ],
          });
      const context = await browser.newContext();
      if (!nativeFallback) await context.grantPermissions(['microphone'], { origin });
      const page = await context.newPage();
      await showReadyFixture(page);
      await page.addInitScript(() => {
        const media = navigator.mediaDevices;
        const original = media.getUserMedia.bind(media);
        let calls = 0;
        let settled = false;
        Object.defineProperty(window, '__pr21MicCalls', { get: () => calls });
        Object.defineProperty(window, '__pr21MicProbe', { get: () => ({ calls, settled }) });
        media.getUserMedia = (...args) => {
          calls += 1;
          const pending = original(...args);
          void pending.then(
            () => {
              settled = true;
            },
            () => {
              settled = true;
            }
          );
          return pending;
        };
      });
      if (nativeFallback) {
        await page.addInitScript(() => {
          let latestContext: AudioContext | null = null;
          const originalResume = AudioContext.prototype.resume;
          AudioContext.prototype.resume = function (this: AudioContext) {
            latestContext = this;
            return originalResume.call(this);
          };
          Object.defineProperty(window, '__pr21AudioState', {
            get: () => latestContext?.state ?? 'none',
          });
          const original = AudioContext.prototype.createMediaStreamSource;
          let first = true;
          AudioContext.prototype.createMediaStreamSource = function (stream) {
            latestContext = this;
            if (first && this.sampleRate === 16000) {
              first = false;
              throw new DOMException('Native device rate required', 'NotSupportedError');
            }
            return original.call(this, stream);
          };
        });
      }
      const evidence = observeStream(page);
      await openShell(page, origin);
      const timings = await captureNothing(page, true, evidence, nativeFallback);
      const sampleRate = evidence.rates.at(-1);
      expect(sampleRate).toBeDefined();
      if (nativeFallback) expect(sampleRate).not.toBe(16000);
      else expect(sampleRate).toBe(16000);
      expect(evidence.pcmBytes).toBeGreaterThan(0);
      expect(evidence.pcmBytes / (sampleRate! * 2)).toBeGreaterThan(1);
      const firstPcmBytes = evidence.pcmBytes;

      await page.reload();
      await expect(page.getByTestId('dictation-button').first()).toBeVisible();
      await captureNothing(page, false, evidence, nativeFallback);
      await page.goto(`${origin}/#/dictionary`);
      await expect(page.getByTestId('dictionary-view')).toBeVisible();
      await page.goto(`${origin}/#/settings`);
      await expect(page.getByTestId('speech-models-section')).toBeVisible();
      if (!nativeFallback) {
        const secondOrigin = `http://localhost:${port}`;
        await context.grantPermissions(['microphone'], { origin: secondOrigin });
        await openShell(page, secondOrigin);
        await page.goto(`${secondOrigin}/#/settings`);
        await expect(page.getByTestId('speech-models-section')).toBeVisible();
        const streamsBeforeTest = evidence.rates.length;
        const audioBeforeTest = evidence.audioRequestAt.length;
        const micCallsBeforeTest = await micCallCount(page);
        await page.getByRole('button', { name: 'Test microphone' }).click();
        await expect(page.getByTestId('speech-mic-meter')).toHaveCount(0);
        expect(evidence.rates).toHaveLength(streamsBeforeTest);
        expect(evidence.audioRequestAt).toHaveLength(audioBeforeTest);
        expect(await micCallCount(page)).toBe(micCallsBeforeTest);
        await acknowledgeOrigin(page);
        await expect(page.getByTestId('speech-mic-meter')).toBeVisible();
        await expect.poll(() => micCallCount(page)).toBeGreaterThan(micCallsBeforeTest);
      }
      await testInfo.attach('serve-dictation-metrics', {
        body: Buffer.from(
          JSON.stringify({
            browser: name,
            sampleRate,
            uploadedAudioSeconds: firstPcmBytes / (sampleRate! * 2),
            pcmBytes: firstPcmBytes,
            startGestureToRecordingPhaseMs: timings.startGestureToRecordingPhaseMs,
            stopToNoticeMs: timings.stopToNoticeMs,
            noInsertionOutcome: timings.outcome,
            model: 'pinned for-tests fixture',
            backend: 'debug fixture',
          })
        ),
        contentType: 'application/json',
      });
      // The page keeps polling /dictation/status through the route above, so a
      // poll can be in flight when the context closes. Remove the routes first,
      // ignoring a callback that was already running, so teardown is not
      // reported as the test failing (seen once on Firefox in rust.yml).
      await page.unrouteAll({ behavior: 'ignoreErrors' });
      await context.close();
    } finally {
      await browser?.close();
      await stopServe(child);
    }
  });
}
