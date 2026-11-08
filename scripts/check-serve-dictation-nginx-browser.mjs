/** PR-21 browser and real transport check for check-serve-dictation-nginx.sh. */
import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const env = process.env;
const required = [
  'PR21_RUN_ROOT',
  'PR21_UI_DIR',
  'PR21_WAV',
  'PR21_TOKEN',
  'PR21_LOOPBACK_URL',
  'PR21_PROXY_URL',
  'PR21_HALVE_URL',
];
for (const key of required) if (!env[key]) throw new Error(`missing ${key}`);

const require = createRequire(join(env.PR21_UI_DIR, 'package.json'));
const { chromium, request } = require('@playwright/test');
const esbuild = require('esbuild');
const token = env.PR21_TOKEN;
const runRoot = env.PR21_RUN_ROOT;
const workletPath = /\/assets\/[^/?]*pcmCapture[^/?]*\.js$/;

function need(condition, sentence) {
  if (!condition) throw new Error(sentence);
}

function safeErrorText(value) {
  return String(value)
    .replaceAll(token, '<browser-token>')
    .replace(/https?:\/\/[^\s"'<>]+/gi, '<url>')
    .replace(/([?&]t=)[^&\s"']+/gi, '$1<redacted>')
    .replace(/\b[0-9a-f]{32,}\b/gi, '<opaque-hex>')
    .replace(/\bsk[-_][a-z0-9_-]{8,}\b/gi, '<opaque-key>')
    .replace(/\b(secret|token|api[_-]?key|authorization|password)\s*[:=]\s*[^\s,}]+/gi, '$1=<redacted>')
    .replace(/(["'])[^"']{20,}\1/g, '<quoted-value>')
    .slice(0, 240);
}

async function saveMissingMicDiagnostic(page, label, model, pageErrors, consoleErrors, httpStatuses) {
  const stage = await page.evaluate(() => ({
    pathname: window.location.pathname,
    route: window.location.hash.startsWith('#/pair')
      ? 'pair'
      : window.location.hash === '#/' || window.location.hash === ''
        ? 'root'
        : 'other',
    rootPresent: Boolean(document.getElementById('root')),
    rootChildren: document.getElementById('root')?.childElementCount ?? 0,
    chatInputPresent: Boolean(document.querySelector('[data-testid="chat-input"]')),
    micButtonCount: document.querySelectorAll('[data-testid="dictation-button"]').length,
    browserSurface: document.documentElement.dataset.biorouterSurface === 'headless',
    secureContext: window.isSecureContext,
    getUserMediaAvailable: typeof navigator.mediaDevices?.getUserMedia === 'function',
    audioWorkletAvailable: typeof window.AudioWorkletNode === 'function',
  })).catch(() => ({ evaluation: 'unavailable' }));
  const stem = label === 'loopback' ? 'loopback' : 'proxy';
  await writeFile(join(runRoot, `${stem}-missing-mic-diagnostic.json`), JSON.stringify({
    stage,
    status: model,
    pageErrors,
    consoleErrors,
    httpStatuses,
  }, null, 2) + '\n', { mode: 0o600 });
  await page.screenshot({ path: join(runRoot, `${stem}-missing-mic.png`), fullPage: false }).catch(() => undefined);
}

async function waitForTokenHandoff(page, baseUrl) {
  const base = new URL(baseUrl);
  await page.waitForURL(
    (url) => url.origin === base.origin && url.pathname === base.pathname && !url.searchParams.has('t'),
    { waitUntil: 'domcontentloaded' }
  );
}

async function recordOnce(context, baseUrl, label) {
  const origin = new URL(baseUrl).origin;
  await context.grantPermissions(['microphone'], { origin });
  const page = await context.newPage();
  await page.addInitScript(() => {
    const NativeAudioContext = window.AudioContext;
    const NativeAudioWorkletNode = window.AudioWorkletNode;
    window.__PR21_WORKLET_MODULES__ = [];
    window.__PR21_START_GESTURE_PERF__ = null;
    window.__PR21_FIRST_WORKLET_LIVE__ = null;
    if (typeof NativeAudioContext === 'function') {
      function ObservedAudioContext(...args) {
        const context = new NativeAudioContext(...args);
        const worklet = context.audioWorklet;
        if (worklet && typeof worklet.addModule === 'function') {
          const addModule = worklet.addModule;
          worklet.addModule = function (url, options) {
            let address;
            try { address = new URL(url, window.location.href); } catch { address = null; }
            const call = {
              pathname: address?.pathname ?? null,
              sameOrigin: address?.origin === window.location.origin,
              loaded: false,
              failed: false,
            };
            window.__PR21_WORKLET_MODULES__.push(call);
            const promise = addModule.call(this, url, options);
            void Promise.resolve(promise).then(
              () => { call.loaded = true; },
              () => { call.failed = true; }
            );
            return promise;
          };
        }
        return context;
      }
      ObservedAudioContext.prototype = NativeAudioContext.prototype;
      Object.setPrototypeOf(ObservedAudioContext, NativeAudioContext);
      window.AudioContext = ObservedAudioContext;
    }
    if (typeof NativeAudioWorkletNode === 'function') {
      function ObservedAudioWorkletNode(...args) {
        const node = new NativeAudioWorkletNode(...args);
        if (args[1] === 'biorouter-pcm-capture') {
          node.port.addEventListener('message', (event) => {
            if (event.data?.type === 'live' && window.__PR21_FIRST_WORKLET_LIVE__ === null) {
              window.__PR21_FIRST_WORKLET_LIVE__ = {
                deliveredAtMs: performance.now(),
                frame: Number.isFinite(event.data.frame) ? event.data.frame : null,
              };
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
  const workletResponses = [];
  const pageErrors = [];
  const consoleErrors = [];
  const httpStatuses = [];
  let firstAudioPutAt = null;
  page.on('pageerror', (error) => {
    if (pageErrors.length < 8) pageErrors.push(safeErrorText(error.message));
  });
  page.on('console', (message) => {
    if (message.type() === 'error' && consoleErrors.length < 8) {
      consoleErrors.push(safeErrorText(message.text()));
    }
  });
  page.on('response', (response) => {
    const pathname = new URL(response.url()).pathname;
    if (workletPath.test(pathname)) {
      workletResponses.push({ pathname, status: response.status() });
    }
    const kind = pathname.endsWith('/dictation/status')
      ? 'dictation-status'
      : response.request().resourceType() === 'document'
        ? 'document'
        : pathname.endsWith('.js') && response.status() !== 200
          ? 'script'
          : null;
    if (kind && httpStatuses.length < 16) httpStatuses.push({ kind, status: response.status() });
  });
  page.on('request', (request) => {
    if (firstAudioPutAt !== null || request.method() !== 'PUT') return;
    const pathname = new URL(request.url()).pathname;
    if (/\/dictation\/streams\/[^/]+\/audio$/.test(pathname)) firstAudioPutAt = Date.now();
  });
  try {
    await page.goto(`${baseUrl}?t=${encodeURIComponent(token)}`, { waitUntil: 'domcontentloaded' });
    await waitForTokenHandoff(page, baseUrl);
    need(new URL(page.url()).origin === origin, `${label}: token handoff left the expected origin`);
    const providerFixture = await page.evaluate(async () => {
      const secret = window.__BIOROUTER_HEADLESS_CONFIG__?.secretKey;
      if (!secret) return { providerPinned: false, modelPinned: false, providerConfigured: false };
      const headers = { 'X-Secret-Key': secret, 'Content-Type': 'application/json' };
      const read = async (key) => {
        const response = await fetch(new URL('config/read', window.location.href), {
          method: 'POST', headers, body: JSON.stringify({ key, is_secret: false }),
        });
        return response.ok ? response.json() : null;
      };
      const catalog = await fetch(new URL('config/providers', window.location.href), { headers });
      const providers = catalog.ok ? await catalog.json() : [];
      return {
        providerPinned: (await read('BIOROUTER_PROVIDER')) === 'versa_azure',
        modelPinned: (await read('BIOROUTER_MODEL')) === 'gpt-5.5-2026-04-24',
        providerConfigured: providers.some((row) => row.name === 'versa_azure' && row.is_configured === true),
      };
    });
    need(providerFixture.providerPinned && providerFixture.modelPinned && providerFixture.providerConfigured,
      `${label}: isolated host must report the approved Versa provider and model as configured`);
    const model = await page.evaluate(async () => {
      const secret = window.__BIOROUTER_HEADLESS_CONFIG__?.secretKey;
      if (!secret) return { status: 0, activeModel: null, engineCompiled: null, captureAllowed: null, activeInstalled: null };
      const response = await fetch(new URL('dictation/status', window.location.href), {
        headers: { 'X-Secret-Key': secret },
      });
      const status = await response.json().catch(() => ({}));
      return {
        status: response.status,
        activeModel: status.active_model ?? null,
        engineCompiled: status.engine?.compiled ?? null,
        engineBackend: status.engine?.backend ?? null,
        captureAllowed: status.policy?.capture?.allowed ?? null,
        activeInstalled: status.models?.some((entry) => entry.id === status.active_model && entry.installed) ?? null,
      };
    });
    need(model.status === 200 && model.activeModel, `${label}: cached speech model is not active through this origin`);
    const stem = label === 'loopback' ? 'loopback' : 'proxy';
    const hostLoadBefore = (await readFile(join(runRoot, 'load-before.txt'), 'utf8')).trim();
    await writeFile(join(runRoot, `${stem}-readiness.json`), JSON.stringify({
      provider_fixture: providerFixture,
      active_model: model.activeModel,
      engine_backend: model.engineBackend,
      engine_compiled: model.engineCompiled,
      capture_allowed: model.captureAllowed,
      active_installed: model.activeInstalled,
      host_load_before: hostLoadBefore,
    }, null, 2) + '\n', { mode: 0o600 });
    const mic = page.getByTestId('dictation-button').first();
    await mic.waitFor({ state: 'visible', timeout: 30_000 }).catch(async () => {
      await saveMissingMicDiagnostic(page, label, model, pageErrors, consoleErrors, httpStatuses);
      throw new Error(`${label}: browser microphone button absent; finish PR-21 browser exposure`);
    });
    await mic.click();
    const notice = page.getByText("Your browser's microphone permission covers every page at", {
      exact: false,
    });
    await notice.waitFor({ state: 'visible', timeout: 10_000 }).catch(() => {
      throw new Error(`${label}: MSG-SHARED-ORIGIN did not appear before first capture`);
    });
    let acknowledge = page.getByTestId('shared-origin-acknowledge');
    if ((await acknowledge.count()) === 0) {
      acknowledge = page.getByRole('button', {
        name: /^(Continue|I understand|Start recording|Record now|Got it)$/i,
      });
    }
    need((await acknowledge.count()) === 1, `${label}: shared-origin acknowledgement button needs a stable selector`);
    await acknowledge.evaluate((button) => {
      button.addEventListener('click', () => {
        window.__PR21_START_GESTURE_PERF__ = performance.now();
      }, { capture: true, once: true });
    });
    await acknowledge.click();
    const startGestureAt = await page.evaluate(() => {
      const stamped = window.__PR21_START_GESTURE_PERF__;
      return typeof stamped === 'number' ? performance.timeOrigin + stamped : null;
    });
    need(startGestureAt !== null, `${label}: acknowledgement click was not stamped`);
    await page.locator('[data-testid="dictation-button"][data-dictation-phase="recording"]').first().waitFor({ timeout: 30_000 }).catch(
      async () => {
        const phase = await mic.getAttribute('data-dictation-phase');
        throw new Error(`${label}: microphone did not record after acknowledgement (phase ${phase})`);
      }
    );
    const recordingPhaseAt = Date.now();
    await page.waitForTimeout(5_000);
    const stoppedAt = Date.now();
    await mic.click();
    const input = page.getByTestId('chat-input').first();
    await page.waitForFunction(
      () => {
        const element = document.querySelector('[data-testid="chat-input"]');
        if (!element) return false;
        const value = 'value' in element ? element.value : element.textContent;
        return Boolean(value?.trim());
      },
      undefined,
      { timeout: 120_000 }
    );
    const transcriptEvidence = await input.evaluate((element) => {
      const value = 'value' in element ? element.value : element.textContent;
      const trimmed = value?.trim() ?? '';
      const folded = trimmed.toLocaleLowerCase('en-US');
      return {
        characters: trimmed.length,
        knowledge_graph: folded.includes('knowledge graph'),
        drugs_that_target: folded.includes('drugs that target'),
      };
    });
    const stopToTextMs = Date.now() - stoppedAt;
    const firstWorkletInput = await page.evaluate(() => {
      const start = window.__PR21_START_GESTURE_PERF__;
      const live = window.__PR21_FIRST_WORKLET_LIVE__;
      return {
        startGestureToFirstWorkletLiveMessageMs:
          typeof start === 'number' && live ? live.deliveredAtMs - start : null,
        firstWorkletFrame: live?.frame ?? null,
      };
    });
    await writeFile(join(runRoot, `${stem}-capture-evidence.json`), JSON.stringify({
      active_model: model.activeModel,
      engine_backend: model.engineBackend,
      host_load_before: hostLoadBefore,
      start_gesture_to_recording_phase_ms: recordingPhaseAt - startGestureAt,
      start_gesture_to_first_worklet_live_message_ms: firstWorkletInput.startGestureToFirstWorkletLiveMessageMs,
      first_worklet_frame: firstWorkletInput.firstWorkletFrame,
      start_gesture_to_first_audio_put_ms: firstAudioPutAt === null ? null : firstAudioPutAt - startGestureAt,
      stop_to_text_ms: stopToTextMs,
      transcript_characters: transcriptEvidence.characters,
      semantic_fragments: {
        knowledge_graph: transcriptEvidence.knowledge_graph,
        drugs_that_target: transcriptEvidence.drugs_that_target,
      },
    }, null, 2) + '\n', { mode: 0o600 });
    need(transcriptEvidence.knowledge_graph && transcriptEvidence.drugs_that_target,
      `${label}: synthetic clip semantic check failed (characters=${transcriptEvidence.characters}, knowledge_graph=${transcriptEvidence.knowledge_graph}, drugs_that_target=${transcriptEvidence.drugs_that_target})`);
    need(firstWorkletInput.startGestureToFirstWorkletLiveMessageMs !== null
      && firstWorkletInput.startGestureToFirstWorkletLiveMessageMs >= 0
      && firstWorkletInput.firstWorkletFrame !== null,
      `${label}: no first real worklet live message followed the recording gesture`);
    need(firstAudioPutAt !== null, `${label}: no real audio PUT followed the recording gesture`);
    const workletProof = await page.evaluate(async (pathPattern) => {
      const pattern = new RegExp(pathPattern);
      const calls = (window.__PR21_WORKLET_MODULES__ ?? []).slice(0, 8).map((call) => ({
        pathname: call.pathname && pattern.test(call.pathname) ? call.pathname : null,
        same_origin: call.sameOrigin,
        loaded: call.loaded,
        failed: call.failed,
      }));
      const loaded = calls.find((call) => call.pathname && call.same_origin && call.loaded);
      if (!loaded) return { calls, asset_path: null, asset_status: null };
      const response = await fetch(loaded.pathname, { cache: 'no-store' });
      return { calls, asset_path: loaded.pathname, asset_status: response.status };
    }, workletPath.source);
    await writeFile(join(runRoot, `${stem}-worklet-diagnostic.json`), JSON.stringify({
      module_calls: workletProof.calls,
      direct_asset_status: workletProof.asset_status,
      page_response_events: workletResponses.slice(0, 8),
    }, null, 2) + '\n', { mode: 0o600 });
    need(workletProof.asset_path && workletProof.asset_status === 200,
      `${label}: loaded pcmCapture module lacked a separate HTTP 200 asset; see ${stem}-worklet-diagnostic.json`);
    const loadedWorklet = new URL(workletProof.asset_path, origin);
    if (label === 'HTTPS prefix') {
      need(loadedWorklet.pathname.startsWith('/user/a/biorouter/assets/'),
        'HTTPS prefix: worklet URL did not retain the proxy subpath');
    }

    await page.reload({ waitUntil: 'domcontentloaded' });
    const repeatedMic = page.getByTestId('dictation-button').first();
    await repeatedMic.waitFor({ state: 'visible', timeout: 30_000 });
    await repeatedMic.click();
    need(
      !(await page
        .getByText("Your browser's microphone permission covers every page at", { exact: false })
        .isVisible()),
      `${label}: MSG-SHARED-ORIGIN appeared twice on one origin`
    );
    await page.locator('[data-testid="dictation-button"][data-dictation-phase="recording"]').first().waitFor({ timeout: 30_000 });
    await repeatedMic.click();
    return {
      start_gesture_to_first_worklet_live_message_ms: firstWorkletInput.startGestureToFirstWorkletLiveMessageMs,
      first_worklet_frame: firstWorkletInput.firstWorkletFrame,
      start_gesture_to_first_audio_put_ms: firstAudioPutAt - startGestureAt,
      stop_to_text_ms: stopToTextMs,
      transcript_characters: transcriptEvidence.characters,
      semantic_fragments: {
        knowledge_graph: transcriptEvidence.knowledge_graph,
        drugs_that_target: transcriptEvidence.drugs_that_target,
      },
      worklet_asset_path: loadedWorklet.pathname,
      notice_count_for_origin: 1,
    };
  } finally {
    await page.close();
  }
}

async function loadRealTransport() {
  const outfile = join(runRoot, 'transport-live.mjs');
  await esbuild.build({
    entryPoints: [join(env.PR21_UI_DIR, 'src/components/dictation/transport.ts')],
    outfile,
    bundle: true,
    platform: 'node',
    format: 'esm',
    target: 'node24',
    plugins: [
      {
        name: 'replace-renderer-api-with-live-proxy-adapter',
        setup(build) {
          build.onResolve({ filter: /^\.\/dictationApi$/ }, () => ({
            path: 'dictationApi',
            namespace: 'pr21-live',
          }));
          build.onLoad({ filter: /.*/, namespace: 'pr21-live' }, () => ({
            contents:
              'export const dictationApi = {}; export const isTranscriptPending = value => value?.pending === true;',
            loader: 'js',
          }));
        },
      },
    ],
  });
  return import(pathToFileURL(outfile).href);
}

async function realHalving(context, secret, transport) {
  const apiContext = await request.newContext({
    ignoreHTTPSErrors: true,
    storageState: await context.storageState(),
    extraHTTPHeaders: { 'X-Secret-Key': secret },
  });
  const attempts = [];
  const base = env.PR21_HALVE_URL;
  const endpoint = (suffix) => new URL(suffix, base).toString();
  const answer = async (response, accepted) => {
    const status = response.status();
    const body = await response.json().catch(() => ({}));
    if (accepted.includes(status)) return { ok: true, status, data: body };
    return {
      ok: false,
      status,
      error: {
        code: typeof body.code === 'string' ? body.code : `http${status}`,
        message: typeof body.message === 'string' ? body.message : '',
        expected_seq: body.expected_seq,
      },
      retryAfterS: Number(response.headers()['retry-after']) || null,
    };
  };
  const adapter = {
    createStream: async (body) =>
      answer(
        await apiContext.post(endpoint('dictation/streams'), {
          data: JSON.stringify(body),
          headers: { 'Content-Type': 'application/json' },
        }),
        [200, 201]
      ),
    putAudio: async (id, seq, body) => {
      const response = await apiContext.put(endpoint(`dictation/streams/${encodeURIComponent(id)}/audio?seq=${seq}`), {
        data: Buffer.from(body),
        headers: { 'Content-Type': 'application/octet-stream' },
      });
      attempts.push({ bytes: body.byteLength, status: response.status() });
      return answer(response, [200]);
    },
    finish: async (id, tail) =>
      answer(
        await apiContext.post(
          endpoint(`dictation/streams/${encodeURIComponent(id)}/finish${tail ? `?seq=${tail.seq}` : ''}`),
          tail
            ? { data: Buffer.from(tail.body), headers: { 'Content-Type': 'application/octet-stream' } }
            : {}
        ),
        [200, 202]
      ),
    poll: async (id, waitMs) =>
      answer(
        await apiContext.get(
          endpoint(`dictation/streams/${encodeURIComponent(id)}${waitMs ? `?wait_ms=${waitMs}` : ''}`)
        ),
        [200, 202]
      ),
    drop: async (id) => {
      await apiContext.delete(endpoint(`dictation/streams/${encodeURIComponent(id)}`)).catch(() => undefined);
    },
  };
  try {
    const stream = transport.createDictationStream(
      { sample_rate: 16000, format: 's16le', target_kind: 'external' },
      { api: adapter }
    );
    await stream.ready;
    stream.push(new Int16Array(16000 * 20));
    await new Promise((resolve) => setTimeout(resolve, 2_500));
    const transcript = await stream.finish();
    need(typeof transcript.no_speech === 'boolean', 'halving stream did not finish with a transcript');
    if (stream.id) await adapter.drop(stream.id);
    const firstRejected = attempts.findIndex((attempt) => attempt.status === 413);
    need(firstRejected >= 0, 'test-only 128 KiB nginx port never returned 413 to real DictationStream');
    const rejected = attempts[firstRejected];
    const reduced = attempts.slice(firstRejected + 1).find((attempt) => attempt.status === 200);
    need(reduced && reduced.bytes <= rejected.bytes / 2 && reduced.bytes >= transport.MIN_CHUNK_BYTES,
      'real DictationStream did not halve its rejected PUT and then succeed');
    return { attempts, first_413_bytes: rejected.bytes, reduced_success_bytes: reduced.bytes };
  } finally {
    await apiContext.dispose();
  }
}

let browser;
let context;
try {
  const launchOptions = {
    args: [
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-audio-capture=${env.PR21_WAV}`,
      '--ignore-certificate-errors',
    ],
  };
  try {
    browser = await chromium.launch(launchOptions);
  } catch (bundledError) {
    try {
      browser = await chromium.launch({ ...launchOptions, channel: 'chrome' });
    } catch (chromeError) {
      throw new Error(
        `Neither Playwright Chromium nor the installed Chrome channel could launch: ${String(bundledError)}; ${String(chromeError)}`
      );
    }
  }
  context = await browser.newContext({ ignoreHTTPSErrors: true });
  if (env.PR21_PHASE === 'loopback') {
    const loopback = await recordOnce(context, env.PR21_LOOPBACK_URL, 'loopback');
    await writeFile(join(runRoot, 'loopback-metrics.json'), JSON.stringify(loopback, null, 2) + '\n', {
      mode: 0o600,
    });
  } else if (env.PR21_PHASE === 'proxy') {
  const loopback = JSON.parse(await readFile(join(runRoot, 'loopback-metrics.json'), 'utf8'));
  const proxy = await recordOnce(context, env.PR21_PROXY_URL, 'HTTPS prefix');
  const page = await context.newPage();
  await page.goto(`${env.PR21_PROXY_URL}?t=${encodeURIComponent(token)}`);
  await waitForTokenHandoff(page, env.PR21_PROXY_URL);
  const secret = await page.evaluate(
    () => window.__BIOROUTER_HEADLESS_CONFIG__?.secretKey
  );
  need(typeof secret === 'string' && /^[0-9a-f]{64}$/.test(secret), 'served shell did not expose its daemon secret to its own page');
  await page.close();
  const statusContext = await request.newContext({
    ignoreHTTPSErrors: true,
    storageState: await context.storageState(),
    extraHTTPHeaders: { 'X-Secret-Key': secret },
  });
  const statusResponse = await statusContext.get(new URL('dictation/status', env.PR21_PROXY_URL).toString());
  need(statusResponse.status() === 200, `dictation status through HTTPS prefix returned ${statusResponse.status()}`);
  const statusBody = await statusResponse.json();
  await statusContext.dispose();
  const transport = await loadRealTransport();
  const halving = await realHalving(context, secret, transport);
  await writeFile(
    join(runRoot, 'browser-metrics.json'),
    JSON.stringify({
      conditions: {
        order: ['loopback', 'HTTPS prefix', 'real transport halving'],
        sample_rate_hz: 16000,
        browser_permission: 'Playwright origin-scoped microphone grant; no fake permission UI switch',
        active_model: statusBody.active_model ?? null,
        engine_backend: statusBody.engine?.backend ?? null,
        engine_state: statusBody.engine?.state ?? null,
      },
      loopback,
      proxy,
      halving,
      stop_to_text_proxy_minus_loopback_ms: proxy.stop_to_text_ms - loopback.stop_to_text_ms,
    }, null, 2) + '\n',
    { mode: 0o600 }
  );
  } else {
    throw new Error('PR21_PHASE must be loopback or proxy');
  }
} catch (error) {
  const safe = String(error?.stack ?? error).replaceAll(token, '<browser-token>');
  console.error(safe);
  process.exitCode = 1;
} finally {
  await context?.close();
  await browser?.close();
}
