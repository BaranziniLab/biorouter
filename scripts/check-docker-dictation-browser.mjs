import { createRequire } from 'node:module';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { cpus, loadavg } from 'node:os';

const { PR31_UI_DIR: ui, PR31_URL: url, PR31_WAV: wav, PR31_RUN_ROOT: runRoot } = process.env;
if (!ui || !url || !wav || !runRoot) throw new Error('PR31 browser check inputs are missing');
const require = createRequire(join(ui, 'package.json'));
const { chromium } = require('@playwright/test');
// Full Chromium (Playwright's `chromium` channel, its new headless mode), never the
// headless shell `chromium.launch()` picks with no channel: the shell answers
// getUserMedia with "NotSupportedError: Not supported" while the microphone button
// still draws, so the check could never reach recording there (measured 2026-11-16
// with chromium-headless-shell 145 on Linux x64, Linux arm64 and macOS; rust.yml's
// served dictation step made the same choice in c6d619d54). CI installs both with
// `npx playwright install --with-deps chromium`. A local run may name an installed
// browser instead (for example PR31_BROWSER_CHANNEL=chrome) rather than download one.
const channel = process.env.PR31_BROWSER_CHANNEL || 'chromium';
const browser = await chromium.launch({
  channel,
  args: [
    '--use-fake-device-for-media-stream',
    `--use-file-for-fake-audio-capture=${wav}`,
  ],
});
const context = await browser.newContext();
// What the page did, kept for the failure evidence below: console lines, page errors,
// failed requests and error answers, by path (a query can carry the token).
const events = [];
const note = (line) => {
  events.push(String(line).slice(0, 500));
  if (events.length > 200) events.shift();
};
let stage = 'launch';
let page = null;
try {
  await context.grantPermissions(['microphone'], { origin: new URL(url).origin });
  page = await context.newPage();
  page.on('console', (message) => note(`console ${message.type()}: ${message.text()}`));
  page.on('pageerror', (error) => note(`pageerror: ${error.message}`));
  page.on('requestfailed', (request) =>
    note(`requestfailed ${request.method()} ${new URL(request.url()).pathname}: ${request.failure()?.errorText}`));
  page.on('response', (response) => {
    if (response.status() >= 400) {
      note(`http ${response.status()} ${response.request().method()} ${new URL(response.url()).pathname}`);
    }
  });
  await page.addInitScript(() => {
    window.__PR31_CAPTURE_START__ = null;
    window.__PR31_FIRST_FRAME__ = null;
    const NativeAudioWorkletNode = window.AudioWorkletNode;
    if (typeof NativeAudioWorkletNode === 'function') {
      function ObservedAudioWorkletNode(...args) {
        const node = new NativeAudioWorkletNode(...args);
        if (args[1] === 'biorouter-pcm-capture') {
          node.port.addEventListener('message', (event) => {
            if (event.data?.type === 'live' && window.__PR31_FIRST_FRAME__ === null) {
              window.__PR31_FIRST_FRAME__ = performance.now();
            }
          });
        }
        return node;
      }
      ObservedAudioWorkletNode.prototype = NativeAudioWorkletNode.prototype;
      Object.setPrototypeOf(ObservedAudioWorkletNode, NativeAudioWorkletNode);
      window.AudioWorkletNode = ObservedAudioWorkletNode;
    }
  });
  stage = 'token exchange';
  await page.goto(`${url}?t=pr31-local-check`, { waitUntil: 'domcontentloaded' });
  await page.waitForURL((address) => !address.searchParams.has('t'));
  const secureContext = await page.evaluate(() => window.isSecureContext);
  if (!secureContext) throw new Error('the published loopback page is not a secure context');

  stage = 'the composer microphone';
  const mic = page.getByTestId('dictation-button').first();
  await mic.waitFor({ state: 'visible', timeout: 30000 });
  const hostLoadBefore = loadavg();
  // The composer starts on pointerdown (PR-19), and the start press's click never
  // reaches the button, so the press is timed from its pointerdown. A click
  // listener here fired on the stop press instead, after the first frame.
  await mic.evaluate((button) => button.addEventListener('pointerdown', () => {
    window.__PR31_CAPTURE_START__ = performance.now();
  }, { capture: true, once: true }));
  await mic.click();
  const notice = page.getByText("Your browser's microphone permission covers every page at", {
    exact: false,
  });
  if (await notice.waitFor({ state: 'visible', timeout: 10000 }).then(() => true).catch(() => false)) {
    const acknowledge = page.getByTestId('shared-origin-acknowledge');
    await acknowledge.evaluate((button) => button.addEventListener('pointerdown', () => {
      window.__PR31_CAPTURE_START__ = performance.now();
    }, { capture: true, once: true }));
    await acknowledge.click();
  }
  stage = 'recording';
  await page.locator('[data-testid="dictation-button"][data-dictation-phase="recording"]')
    .first().waitFor({ timeout: 30000 });
  await page.waitForTimeout(12000);
  stage = 'transcription into the composer';
  const stoppedAt = Date.now();
  await mic.click();
  await page.waitForFunction(() => {
    const input = document.querySelector('[data-testid="chat-input"]');
    const text = input && ('value' in input ? input.value : input.textContent);
    if (typeof text !== 'string') return false;
    const words = text.toLowerCase().match(/[a-z0-9]+/g)?.join(' ') ?? '';
    return [
      'my fellow americans',
      'ask not what your country can do for you',
      'ask what you can do for your country',
    ].every((fragment) => words.includes(fragment));
  }, undefined, { timeout: 120000 });
  const stopToTextMs = Date.now() - stoppedAt;
  const pressToFirstFrameMs = await page.evaluate(() => {
    const start = window.__PR31_CAPTURE_START__;
    const first = window.__PR31_FIRST_FRAME__;
    return typeof start === 'number' && typeof first === 'number' ? first - start : null;
  });
  if (pressToFirstFrameMs === null || pressToFirstFrameMs < 0) {
    throw new Error('the browser did not observe a live audio frame');
  }
  const status = await page.evaluate(async () => {
    const secret = window.__BIOROUTER_HEADLESS_CONFIG__?.secretKey;
    const response = await fetch(new URL('dictation/status', window.location.href), {
      headers: { 'X-Secret-Key': secret },
    });
    if (!response.ok) throw new Error(`dictation status returned ${response.status}`);
    return response.json();
  });
  await writeFile(join(runRoot, 'browser-metrics.json'), JSON.stringify({
    secure_context: secureContext,
    active_model: status.active_model,
    engine_backend: status.engine?.backend ?? null,
    host_load_1m_before: hostLoadBefore[0],
    host_load_1m_after: loadavg()[0],
    host_logical_cpus: cpus().length,
    press_to_first_frame_ms: pressToFirstFrameMs,
    stop_to_text_ms: stopToTextMs,
    clip: 'jfk.wav',
    clip_semantics_match: true,
  }, null, 2) + '\n', { mode: 0o600 });
  console.log(`PR-31 browser dictation passed; stop to text ${stopToTextMs} ms`);
} catch (error) {
  if (page) await keepFailureEvidence(page, error).catch((evidence) => {
    console.error(`could not keep the browser failure evidence: ${evidence}`);
  });
  throw error;
} finally {
  await context.close();
  await browser.close();
}

/**
 * What the page showed when the check failed, beside the other evidence in the run root:
 * a screenshot, and a JSON record of the stage, the microphone and composer it found, the
 * page's text, whether this browser can open a microphone at all, and the page's events.
 * The daemon secret is cut out of every string before it is written.
 */
async function keepFailureEvidence(page, error) {
  await page.screenshot({ path: join(runRoot, 'browser-failure.png') }).catch(() => undefined);
  const seen = await page.evaluate(async () => {
    const mics = [...document.querySelectorAll('[data-testid="dictation-button"]')];
    let microphone;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      stream.getTracks().forEach((track) => track.stop());
      microphone = 'opened';
    } catch (refusal) {
      microphone = `${refusal?.name ?? 'Error'}: ${refusal?.message ?? refusal}`;
    }
    return {
      secret: window.__BIOROUTER_HEADLESS_CONFIG__?.secretKey ?? '',
      path: `${location.pathname}${location.hash}`,
      title: document.title,
      microphone_buttons: mics.length,
      microphone_phases: mics.map((button) => button.getAttribute('data-dictation-phase')),
      chat_inputs: document.querySelectorAll('[data-testid="chat-input"]').length,
      user_agent: navigator.userAgent,
      microphone,
      text: (document.body?.innerText ?? '').slice(0, 4000),
    };
  });
  const { secret, ...rest } = seen;
  const record = JSON.stringify({ stage, error: String(error?.message ?? error), channel, ...rest, events }, null, 2);
  await writeFile(
    join(runRoot, 'browser-failure.json'),
    (secret ? record.split(secret).join('<daemon secret>') : record) + '\n',
    { mode: 0o600 },
  );
  console.error(`the browser check failed at ${stage}; evidence in ${join(runRoot, 'browser-failure.json')}`);
}
