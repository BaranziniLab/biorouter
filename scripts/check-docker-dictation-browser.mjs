import { createRequire } from 'node:module';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { cpus, loadavg } from 'node:os';

const { PR31_UI_DIR: ui, PR31_URL: url, PR31_WAV: wav, PR31_RUN_ROOT: runRoot } = process.env;
if (!ui || !url || !wav || !runRoot) throw new Error('PR31 browser check inputs are missing');
const require = createRequire(join(ui, 'package.json'));
const { chromium } = require('@playwright/test');
// CI installs Playwright's Chromium. A local run may name an installed browser
// instead (for example PR31_BROWSER_CHANNEL=chrome) rather than download one.
const channel = process.env.PR31_BROWSER_CHANNEL || undefined;
const browser = await chromium.launch({
  channel,
  args: [
    '--use-fake-device-for-media-stream',
    `--use-file-for-fake-audio-capture=${wav}`,
  ],
});
const context = await browser.newContext();
try {
  await context.grantPermissions(['microphone'], { origin: new URL(url).origin });
  const page = await context.newPage();
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
  await page.goto(`${url}?t=pr31-local-check`, { waitUntil: 'domcontentloaded' });
  await page.waitForURL((address) => !address.searchParams.has('t'));
  const secureContext = await page.evaluate(() => window.isSecureContext);
  if (!secureContext) throw new Error('the published loopback page is not a secure context');

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
  await page.locator('[data-testid="dictation-button"][data-dictation-phase="recording"]')
    .first().waitFor({ timeout: 30000 });
  await page.waitForTimeout(12000);
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
} finally {
  await context.close();
  await browser.close();
}
