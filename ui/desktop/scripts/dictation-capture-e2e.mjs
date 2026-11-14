// Dictation capture in a real Electron renderer (PR-17, the `dictation-capture-electron`
// job in .github/workflows/frontend.yml; docs/dictation/testing-and-release.md, "CI jobs").
//
// What jsdom cannot show, measured here:
//
// 1. The worklet survives the build. The renderer's own Vite config builds a page that
//    imports the real `components/dictation/capture.ts`, and the built worklet must be a
//    file under `assets/`, never a `data:` URL. A second build with Vite's default
//    inline limit (the config's refusal taken away) must still emit the file, so the
//    import's own `?url&no-inline` is checked on its own: removing it turns this red,
//    which BIOROUTER_DICTATION_CAPTURE_DROP_NO_INLINE=1 shows without editing the file.
// 2. The page is packed into `app.asar` and loaded over `file://` under the content
//    security policy copied from `index.html`. Electron runs with the fake microphone
//    switches main.ts adds (`utils/micPermissionSettings.ts`, bundled from source) and
//    the permission handler main.ts installs (`utils/permissionPolicy.ts`).
// 3. A 440 Hz tone at amplitude 0.5 comes out at 16 kHz with RMS 0.3535 +/- 0.005 and
//    440 +/- 2 Hz, with samples for 98 % or more of the time the capture's audio graph
//    rendered (what capture.ts can lose; the track's delivered time, then the wall time,
//    stand in where it is missing) and 75 % or more of the wall time, a stall guard a
//    starved runner's audio thread still clears; audio alone is
//    granted; audio with video, and a document outside the renderer, are refused with
//    NotAllowedError; a hidden window captures; after stop the tracks are ended and the
//    context is closed.
// 4. One whole dictation through `dictationSession.ts` and `transport.ts` against a
//    loopback stand in for the daemon: one PUT in flight, the final chunk and `finish`
//    right after the stop, the text inserted into a textarea. The audio the stand in
//    received is left in `received.wav` beside the evidence. A speech run
//    (BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV, see below) hears a recording instead of
//    the tone and runs only this step, so a real model can transcribe that file.
//    With BIOROUTER_DICTATION_CAPTURE_DAEMON set to a running biorouterd's address
//    (its secret in BIOROUTER_DICTATION_CAPTURE_DAEMON_SECRET, the user action proof in
//    BIOROUTER_DICTATION_CAPTURE_DAEMON_PROOF), the stand in records the same timings
//    and passes every request on to that daemon, so the text in the field is what the
//    real routes and a real model answered.
//
// Latency is reported, not only asserted: press to `getUserMedia`, to the first audio
// frame in the worklet, to the first 100 ms chunk, and stop to the worklet's last
// chunk, to the final PUT, to `finish`, and from the transcript to the text in the
// field. In the shown window every press and stop is a click, as a person makes it:
// after an input event Chromium renders a frame before it delivers the worklet's
// answer, which a stop sent through `page.evaluate` would not show (7 to 9 ms against
// 0.5 ms, measured).
//
// Run from ui/desktop: `node scripts/dictation-capture-e2e.mjs`. Evidence goes to
// $BIOROUTER_DICTATION_CAPTURE_OUTPUT_DIR (default: a temporary folder).
import assert from 'node:assert/strict';
import { rmSync } from 'node:fs';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
// Two imports resolve only because the lockfile hoists them: `@electron/asar`
// (3.4.1) through `@electron/packager` under `@electron-forge/cli`, and
// `esbuild` (0.27.3) through `vite`. Neither is in package.json, so a lockfile
// change that stops hoisting either one fails this job for a reason that has
// nothing to do with dictation; declaring them is a follow up, because adding
// a dependency means regenerating the lockfile.
import { createPackage } from '@electron/asar';
import { _electron as electron } from '@playwright/test';
import { build as esbuild } from 'esbuild';
import { build as viteBuild, loadConfigFromFile } from 'vite';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const desktopDir = path.resolve(scriptDir, '..');
const srcDir = path.join(desktopDir, 'src');
// The real path, so relative imports from the page agree with what Vite resolves
// (macOS's temporary folder sits behind a /var to /private/var link).
const work = await fs.realpath(
  await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-dictation-capture-'))
);
const outputDir = process.env.BIOROUTER_DICTATION_CAPTURE_OUTPUT_DIR
  ? path.resolve(process.env.BIOROUTER_DICTATION_CAPTURE_OUTPUT_DIR)
  : path.join(work, 'evidence');
await fs.mkdir(outputDir, { recursive: true });
// The build, the tone, the packed app and Electron's profile are scratch, whichever
// way the run ends; they are kept only when the evidence was written inside them.
process.on('exit', () => {
  if (!outputDir.startsWith(work)) rmSync(work, { recursive: true, force: true });
});
// A run stopped from outside cleans up too: without these, a signal skips `exit`.
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => process.exit(1));

const TONE_HZ = 440;
const TONE_AMPLITUDE = 0.5;
const EXPECTED_RMS = TONE_AMPLITUDE / Math.SQRT2;
const results = {
  startedAt: new Date().toISOString(),
  platform: process.platform,
  arch: process.arch,
};
const posix = (p) => p.split(path.sep).join('/');

// ── the tone ────────────────────────────────────────────────────────────────
function toneWav(file, seconds, rate = 16000) {
  const n = Math.round(seconds * rate);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    const s = TONE_AMPLITUDE * Math.sin((2 * Math.PI * TONE_HZ * i) / rate);
    data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  return fs.writeFile(file, monoWav(data, rate));
}
/** 16-bit mono PCM with a WAV header. */
function monoWav(data, rate) {
  const header = Buffer.alloc(44);
  header.write('RIFF', 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write('WAVE', 8);
  header.write('fmt ', 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write('data', 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}
/** Seconds of audio in a PCM WAV file, from its `fmt ` and `data` chunks. */
async function wavSeconds(file) {
  const bytes = await fs.readFile(file);
  assert.equal(bytes.toString('ascii', 0, 4), 'RIFF', `${file} is not a WAV file`);
  let byteRate = 0;
  for (let at = 12; at + 8 <= bytes.length; ) {
    const id = bytes.toString('ascii', at, at + 4);
    const size = bytes.readUInt32LE(at + 4);
    if (id === 'fmt ') byteRate = bytes.readUInt32LE(at + 16);
    if (id === 'data') {
      assert(byteRate > 0, `${file} has no fmt chunk before its data`);
      return Math.min(size, bytes.length - at - 8) / byteRate;
    }
    at += 8 + size + (size % 2);
  }
  throw new Error(`${file} has no data chunk`);
}
const wav = path.join(work, 'tone440.wav');
// Long enough that %noloop never runs out during the run.
await toneWav(wav, 120);

// BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV names a speech recording, for a check with
// a real model after the run: the fake microphone hears it instead of the tone,
// the tone checks (1 to 3 below) are skipped, the dictation lasts as long as the
// clip plus 0.7 s, and `received.wav` beside the evidence holds the audio the
// stand in received, for the model to transcribe.
const speechWav = process.env.BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV
  ? path.resolve(process.env.BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV)
  : null;
const dictateMs = speechWav ? Math.round((await wavSeconds(speechWav)) * 1000) + 700 : 3000;
results.speechWav = speechWav;
results.dictateMs = dictateMs;
// Milliseconds a stand in render takes on each change of the session's snapshot
// (see `prepareDictation` in the probe page); 0, the default, renders nothing.
const renderMs = Math.max(0, Number(process.env.BIOROUTER_DICTATION_CAPTURE_RENDER_MS ?? 0) || 0);
results.renderMs = renderMs;

// ── the page ────────────────────────────────────────────────────────────────
const indexHtml = await fs.readFile(path.join(desktopDir, 'index.html'), 'utf8');
const cspMeta = indexHtml.match(/<meta http-equiv="Content-Security-Policy"[^>]*\/>/)?.[0];
assert(cspMeta, 'index.html has no Content-Security-Policy meta to copy');
results.csp = cspMeta.match(/content="([^"]*)"/)[1];

const pageDir = path.join(work, 'page');
await fs.mkdir(pageDir, { recursive: true });
await fs.writeFile(
  path.join(pageDir, 'index.html'),
  `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    ${cspMeta}
    <title>Dictation capture probe</title>
  </head>
  <body>
    <div class="biorouter-composer-card">
      <textarea id="field" rows="4" cols="60"></textarea>
      <button id="capture">capture</button>
      <button id="stop">stop</button>
      <button id="dictate">dictate</button>
    </div>
    <script type="module" src="./main.ts"></script>
  </body>
</html>
`
);
// Relative, so Vite never reads a leading slash as the page root.
const fromSrc = (p) => JSON.stringify(`./${posix(path.relative(pageDir, path.join(srcDir, p)))}`);
await fs.writeFile(
  path.join(pageDir, 'main.ts'),
  `// Probe page. Stubs the two things a Biorouter window gets from its preload and
// main process (the daemon address and the secret), then exposes the real modules.
const params = new URLSearchParams(location.hash.slice(1));
const api = params.get('api') ?? '';
(window as any).appConfig = { get: (key: string) => (key === 'BIOROUTER_API_HOST' ? api : undefined), getAll: () => ({}) };
(window as any).electron = {
  platform: 'probe',
  getSecretKey: async () => 'probe-secret',
  getUserActionKey: async () => 'probe-proof',
  setDictationBusy: (busy: boolean) => ((window as any).__busy ??= []).push(busy),
  onDictationLockScreen: () => () => undefined,
};
const violations: string[] = [];
document.addEventListener('securitypolicyviolation', (e) => violations.push(e.violatedDirective + ' ' + e.blockedURI));
// When each request left the page, to split client time from transport time.
const fetches: Array<{ at: number; method: string; path: string }> = [];
const realFetch = window.fetch.bind(window);
window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
  // The generated client passes a Request; the audio routes pass a URL and an init.
  const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
  fetches.push({ at: Date.now(), method, path: url.pathname });
  return realFetch(input, init);
};

import { startCapture } from ${fromSrc('components/dictation/capture.ts')};
import {
  attachDictationComposer,
  getDictationSnapshot,
  startDictation,
  subscribeDictation,
} from ${fromSrc('components/dictation/dictationSession.ts')};
import { ensureDictationStatus, noteDictationInterest } from ${fromSrc('components/dictation/dictationStore.ts')};

// Watch what capture.ts opens, without changing it.
const streams: MediaStream[] = [];
const contexts: AudioContext[] = [];
const realGetUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
navigator.mediaDevices.getUserMedia = async (constraints) => {
  const stream = await realGetUserMedia(constraints);
  streams.push(stream);
  return stream;
};
const RealAudioContext = window.AudioContext;
(window as any).AudioContext = class extends RealAudioContext {
  constructor(options?: AudioContextOptions) {
    super(options);
    contexts.push(this);
  }
};
// When the stop's flush left for the worklet and when its last chunk came back,
// to split the stop path into capture time and transport time.
const portEvents: Array<{ at: number; kind: 'flush' | 'final' }> = [];
const realPostMessage = MessagePort.prototype.postMessage;
MessagePort.prototype.postMessage = function (this: MessagePort, message: any, ...rest: any[]) {
  if (message && message.type === 'flush') portEvents.push({ at: Date.now(), kind: 'flush' });
  return (realPostMessage as any).call(this, message, ...rest);
};
const RealAudioWorkletNode = window.AudioWorkletNode;
(window as any).AudioWorkletNode = class extends RealAudioWorkletNode {
  constructor(context: BaseAudioContext, name: string, options?: AudioWorkletNodeOptions) {
    super(context, name, options);
    this.port.addEventListener('message', (event: MessageEvent) => {
      if (event.data && event.data.final) portEvents.push({ at: Date.now(), kind: 'final' });
    });
  }
};

const now = () => performance.now();
const run: any = { chunks: [] as Int16Array[], levels: [] as number[] };
// What the microphone's track says it delivered (MediaStreamTrackAudioStats), in
// milliseconds of audio, so coverage can be measured against what reached the page: a
// loaded hosted runner starves the audio thread, and the track then delivers less than the
// wall time (95.95 % of 4.08 s on one run, 94.6 % on another).
const trackDeliveredMs = (): number | null => {
  const stats = (streams.at(-1)?.getAudioTracks()[0] as any)?.stats;
  return typeof stats?.deliveredFramesDuration === 'number'
    ? stats.deliveredFramesDuration
    : null;
};
// The seconds the capture's own audio graph has rendered (its AudioContext's currentTime).
// The worklet turns every rendered frame into samples, so samples over this is what
// capture.ts and its worklet can lose. A starved runner's audio thread renders less than the
// track delivers (94.0 % of 4.10 s on one run) and less than the wall time (86.6 % in a
// hidden window on another): the runner's loss, which only these two other measures see.
const contextSeconds = (): number | null => contexts.at(-1)?.currentTime ?? null;
let capture: any = null;

document.getElementById('capture')!.addEventListener('click', (event) => {
  run.pressAt = now();
  run.eventLagMs = run.pressAt - event.timeStamp;
  startCapture({
    noiseSuppression: false,
    onLive: () => {
      run.liveAt ??= now();
      run.liveDeliveredMs ??= trackDeliveredMs();
      run.liveContextS ??= contextSeconds();
    },
    onChunk: (pcm, level) => {
      run.firstChunkAt ??= now();
      run.chunks.push(pcm);
      run.levels.push(level.rms);
    },
  }).then(
    (c) => {
      capture = c;
      run.mode = c.mode;
      run.sampleRate = c.sampleRate;
      run.timings = c.timings;
      run.readyAt = now();
    },
    (error) => (run.error = { name: error?.name, message: String(error?.message ?? error) })
  );
});

function analyse(chunks: Int16Array[], rate: number) {
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const all = new Float32Array(total);
  let o = 0;
  for (const c of chunks) {
    for (let i = 0; i < c.length; i++) all[o++] = c[i] / 32768;
  }
  let first = all.findIndex((s) => Math.abs(s) > 1e-4);
  if (first < 0) first = total;
  const a = Math.min(total, first + Math.round(rate * 0.1));
  const b = Math.max(a, total - Math.round(rate * 0.05));
  let sum = 0;
  let crossings = 0;
  for (let i = a; i < b; i++) {
    sum += all[i] * all[i];
    if (i > a && all[i - 1] < 0 !== all[i] < 0) crossings++;
  }
  const seconds = (b - a) / rate;
  return {
    samples: total,
    firstNonSilentSample: first,
    rms: b > a ? Math.sqrt(sum / (b - a)) : 0,
    hz: seconds > 0 ? crossings / 2 / seconds : 0,
  };
}

(window as any).probe = {
  showInterest: () => noteDictationInterest(),
  violations: () => violations,
  captureState: () => ({ ...run, chunks: run.chunks.length, levels: run.levels.slice(0, 3) }),
  async stopCapture() {
    const stopAt = now();
    const stopDeliveredMs = trackDeliveredMs();
    const stopContextS = contextSeconds();
    await capture.stop();
    const stopped = now();
    await capture.closed;
    const closed = now();
    const stats = analyse(run.chunks, run.sampleRate);
    const wallS = (stopAt - run.liveAt) / 1000;
    const deliveredS =
      run.liveDeliveredMs != null && stopDeliveredMs != null
        ? (stopDeliveredMs - run.liveDeliveredMs) / 1000
        : null;
    const renderedS =
      run.liveContextS != null && stopContextS != null ? stopContextS - run.liveContextS : null;
    return {
      mode: run.mode,
      sampleRate: run.sampleRate,
      error: run.error ?? null,
      ...stats,
      wallS,
      sampleCoverage: stats.samples / (wallS * run.sampleRate),
      deliveredS,
      trackCoverage:
        deliveredS != null && deliveredS > 0 ? stats.samples / (deliveredS * run.sampleRate) : null,
      renderedS,
      graphCoverage:
        renderedS != null && renderedS > 0 ? stats.samples / (renderedS * run.sampleRate) : null,
      pressToGetUserMediaMs: run.timings?.streamMs ?? null,
      pressToModuleMs: run.timings?.moduleMs ?? null,
      pressToConnectedMs: run.timings?.connectedMs ?? null,
      pressToLiveMs: run.liveAt - run.pressAt,
      pressToFirstChunkMs: run.firstChunkAt - run.pressAt,
      pressToFirstNonSilentSampleMs:
        run.liveAt - run.pressAt + (stats.firstNonSilentSample / run.sampleRate) * 1000,
      stopToFinalChunkMs: stopped - stopAt,
      stopToContextClosedMs: closed - stopAt,
      tracksEnded: streams.every((s) => s.getTracks().every((t) => t.readyState === 'ended')),
      contextsClosed: contexts.every((c) => c.state === 'closed'),
      tracks: streams.length,
      contexts: contexts.length,
    };
  },
  async tryGetUserMedia(constraints: MediaStreamConstraints) {
    try {
      const stream = await realGetUserMedia(constraints);
      stream.getTracks().forEach((t) => t.stop());
      return { ok: true };
    } catch (error: any) {
      return { ok: false, name: error?.name, message: String(error?.message ?? error) };
    }
  },
  // One whole dictation through the session and the transport.
  async prepareDictation() {
    const field = document.getElementById('field') as HTMLTextAreaElement;
    attachDictationComposer({
      key: 'probe',
      field: () => field,
      setByState: (value: string) => (field.value = value),
      storesDraft: false,
    });
    field.addEventListener('input', () => ((window as any).__insertedAt ??= Date.now()));
    // The session shows "recording" on the first audio frame (capture's onLive).
    subscribeDictation(() => {
      if (getDictationSnapshot().activity === 'recording') (window as any).__recordingAt ??= Date.now();
    });
    // In the app a composer renders when the snapshot changes, in a microtask
    // (React's useSyncExternalStore). BIOROUTER_DICTATION_CAPTURE_RENDER_MS stands in
    // for the cost of that render, so where it falls on the stop path shows.
    const renderMs = Number(params.get('render') ?? '0');
    if (renderMs > 0) {
      subscribeDictation(() =>
        queueMicrotask(() => {
          const end = performance.now() + renderMs;
          while (performance.now() < end);
        })
      );
    }
    // A composer's focus loads the status store before the press (dictationStore.ts).
    field.focus();
    await ensureDictationStatus();
    const button = document.getElementById('dictate')!;
    // The real microphone keeps focus in the textarea on mousedown (PR-19).
    button.addEventListener('mousedown', (e) => e.preventDefault());
    button.addEventListener(
      'click',
      () => {
        ((window as any).__presses ??= []).push({ at: Date.now(), activity: getDictationSnapshot().activity });
        // The first press starts; a press while recording stops and inserts.
        void startDictation({ key: 'probe', target: { target_kind: 'chat', session_id: 'probe-chat' } });
      },
      true
    );
    return { focused: document.activeElement === field };
  },
  dictationState: () => ({
    snapshot: getDictationSnapshot(),
    value: (document.getElementById('field') as HTMLTextAreaElement).value,
    presses: (window as any).__presses ?? [],
    insertedAt: (window as any).__insertedAt ?? null,
    recordingAt: (window as any).__recordingAt ?? null,
    busy: (window as any).__busy ?? [],
    fetches,
    portEvents,
  }),
};
// The shown window's stop is a press, as a person makes it.
document.getElementById('stop')!.addEventListener('click', () => {
  (window as any).__stopped = (window as any).probe.stopCapture();
});
`
);

// ── build it with the renderer's own Vite config ────────────────────────────
const rendererConfig = path.join(desktopDir, 'vite.renderer.config.mts');
const loaded = await loadConfigFromFile({ command: 'build', mode: 'production' }, rendererConfig);
const inlineLimit = loaded?.config?.build?.assetsInlineLimit;
assert.equal(
  typeof inlineLimit,
  'function',
  'vite.renderer.config.mts sets no assetsInlineLimit function'
);
assert.equal(inlineLimit('/x/components/dictation/pcmCapture.worklet.js', Buffer.alloc(10)), false);
assert.equal(inlineLimit('/x/icon.png', Buffer.alloc(10)), undefined);

// BIOROUTER_DICTATION_CAPTURE_DROP_NO_INLINE=1 takes `no-inline` off the import in
// capture.ts for both builds, as the build reads the file, without changing it on
// disk: the default inline limit build below must then turn this run red, which is
// how the acceptance "removing no-inline turns the job red" is shown.
const dropNoInline = process.env.BIOROUTER_DICTATION_CAPTURE_DROP_NO_INLINE === '1';
results.dropNoInline = dropNoInline;
const dropNoInlinePlugin = {
  name: 'dictation-capture-drop-no-inline',
  enforce: 'pre',
  transform(code, id) {
    if (!posix(id).endsWith('/components/dictation/capture.ts')) return null;
    const next = code.replace(
      './pcmCapture.worklet.js?url&no-inline',
      './pcmCapture.worklet.js?url'
    );
    assert.notEqual(next, code, 'capture.ts no longer imports the worklet as ?url&no-inline');
    return next;
  },
};

async function buildPage(outDir, override = {}) {
  await viteBuild({
    configFile: rendererConfig,
    root: pageDir,
    base: './',
    publicDir: false,
    logLevel: 'warn',
    plugins: dropNoInline ? [dropNoInlinePlugin] : [],
    build: { outDir, emptyOutDir: true, ...override },
  });
  const assets = await fs.readdir(path.join(outDir, 'assets'));
  const worklet = assets.find((name) => /^pcmCapture\.worklet-.*\.js$/.test(name));
  const chunks = await Promise.all(
    assets
      .filter((n) => n.endsWith('.js'))
      .map((n) => fs.readFile(path.join(outDir, 'assets', n), 'utf8'))
  );
  // Vite inlines a script asset as `data:text/javascript;base64,...`; nothing else in
  // this page builds such a URL.
  const inlined = chunks.some((text) => /data:(?:text|application)\/javascript/.test(text));
  return { worklet, referenced: chunks.some((text) => worklet && text.includes(worklet)), inlined };
}

const distDir = path.join(work, 'app', 'dist');
const repoBuild = await buildPage(distDir);
assert(repoBuild.worklet, 'the worklet was not emitted as a file under assets/');
assert(repoBuild.referenced, 'no chunk references the emitted worklet file');
assert(!repoBuild.inlined, 'the worklet was inlined as a data: URL');
// The import's own `no-inline`, with the config's refusal taken away.
const defaultLimitBuild = await buildPage(path.join(work, 'dist-default-limit'), {
  assetsInlineLimit: 4096,
});
assert(
  defaultLimitBuild.worklet && defaultLimitBuild.referenced && !defaultLimitBuild.inlined,
  'with Vite’s default inline limit the worklet was inlined: the import lost ?no-inline'
);
results.build = { worklet: repoBuild.worklet, defaultLimitWorklet: defaultLimitBuild.worklet };

// ── the main process, from the app's own modules ────────────────────────────
const appDir = path.join(work, 'app');
const mainEntry = path.join(work, 'main-entry.mjs');
await fs.writeFile(
  mainEntry,
  `import { app, BrowserWindow, session } from 'electron';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { isAllowedRendererPermission } from ${JSON.stringify(posix(path.join(srcDir, 'utils/permissionPolicy.ts')))};
import { FAKE_MIC_WAV_ENV, fakeMicrophoneSwitches } from ${JSON.stringify(posix(path.join(srcDir, 'utils/micPermissionSettings.ts')))};

// As main.ts does at load, before ready.
for (const fakeMic of fakeMicrophoneSwitches({
  isPackaged: app.isPackaged,
  wavPath: process.env[FAKE_MIC_WAV_ENV],
  platform: process.platform,
  existingDisabledFeatures: app.commandLine.getSwitchValue('disable-features'),
})) {
  if (fakeMic.value === undefined) app.commandLine.appendSwitch(fakeMic.name);
  else app.commandLine.appendSwitch(fakeMic.name, fakeMic.value);
}

globalThis.__permissionLog = [];
globalThis.__requests = [];
app.whenReady().then(() => {
  const appEntryUrl = pathToFileURL(path.join(app.getAppPath(), 'dist', 'index.html'));
  const ses = session.fromPartition('persist:dictation-probe');
  // As main.ts installSessionHooks does: both handlers through one policy.
  ses.setPermissionCheckHandler((_wc, permission, requestingOrigin, details) =>
    isAllowedRendererPermission(permission, details.requestingUrl || requestingOrigin, appEntryUrl, [details.mediaType ?? 'unknown'], details.isMainFrame)
  );
  ses.setPermissionRequestHandler((_wc, permission, callback, details) => {
    const mediaTypes = 'mediaTypes' in details ? details.mediaTypes ?? [] : [];
    const answer = isAllowedRendererPermission(permission, details.requestingUrl, appEntryUrl, mediaTypes, details.isMainFrame);
    globalThis.__permissionLog.push({ permission, mediaTypes, url: details.requestingUrl, mainFrame: details.isMainFrame, answer });
    callback(answer);
  });
  ses.webRequest.onBeforeRequest((details, callback) => {
    globalThis.__requests.push(details.url);
    callback({});
  });
  const win = new BrowserWindow({
    width: 720,
    height: 420,
    show: true,
    webPreferences: { partition: 'persist:dictation-probe', contextIsolation: true, sandbox: true, nodeIntegration: false },
  });
  void win.loadURL(
    appEntryUrl.href +
      '#api=' + encodeURIComponent(process.env.PROBE_API ?? '') +
      '&render=' + encodeURIComponent(process.env.PROBE_RENDER_MS ?? '0')
  );
});
app.on('window-all-closed', () => app.quit());
`
);
await esbuild({
  entryPoints: [mainEntry],
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node20',
  external: ['electron'],
  outfile: path.join(appDir, 'main.cjs'),
  logLevel: 'warning',
});
await fs.writeFile(
  path.join(appDir, 'package.json'),
  JSON.stringify({
    name: 'biorouter-dictation-capture-probe',
    version: '0.0.0',
    main: 'main.cjs',
  })
);
const asarPath = path.join(work, 'app.asar');
await createPackage(appDir, asarPath);

// A document outside the renderer, for the foreign document refusal.
const foreignDir = path.join(work, 'foreign');
await fs.mkdir(foreignDir, { recursive: true });
await fs.writeFile(
  path.join(foreignDir, 'foreign.html'),
  '<!doctype html><title>foreign</title><p>foreign</p>'
);

// ── a loopback stand in for the daemon's /dictation/* routes ────────────────
const daemon = {
  creates: [],
  puts: [],
  finishes: [],
  deletes: [],
  inflight: 0,
  maxInflight: 0,
  bytes: 0,
  statusReads: 0,
};
const TRANSCRIPT = {
  text: 'EPAS1 is expressed in the lung',
  raw_text: 'a pass one is expressed in the lung',
  changes: [],
  segments: [],
  model: 'whisper-small.en-q8_0',
  audio_s: 0,
  elapsed_ms: 0,
  bias_scope: 'public',
  retried: null,
  no_speech: false,
};
const STATUS = {
  engine: {
    compiled: true,
    state: 'ready',
    backend: 'cpu',
    device: null,
    threads: 4,
    missing_cpu_features: [],
    rss_bytes: null,
  },
  models: [
    {
      id: 'whisper-small.en-q8_0',
      display_name: 'small.en',
      installed: true,
      bytes: 264477561,
      location: 'user',
      superseded: false,
      suitability: 'recommended',
      recommended: true,
    },
  ],
  active_model: 'whisper-small.en-q8_0',
  language: 'en',
  install: { allowed: true, reason: null },
  install_job: null,
  policy: {
    managed: false,
    enabled: { allowed: true, reason: null },
    capture: { allowed: true, reason: null },
    install: { allowed: true, reason: null },
    external_engine: { allowed: true, reason: null },
    dictionary: { allowed: true, reason: null },
    agent_writes: { allowed: true, reason: null },
    allowed_models: null,
  },
  limits: { max_seconds: 600, max_chunk_bytes: 524288, max_streams: 8 },
  queue: { streams: 0, segments: 0 },
  account: 'probe',
  host: 'probe',
  models_dir: '/tmp',
};
// A real daemon to pass requests on to, instead of the canned answers above.
const realDaemon = process.env.BIOROUTER_DICTATION_CAPTURE_DAEMON ?? null;
results.realDaemon = Boolean(realDaemon);
/** Sends one request on to the real daemon with its secret, and relays its answer. */
function forward(req, url, body, res, onAnswer) {
  const headers = { 'X-Secret-Key': process.env.BIOROUTER_DICTATION_CAPTURE_DAEMON_SECRET ?? '' };
  if (req.headers['x-user-action']) {
    headers['X-User-Action'] = process.env.BIOROUTER_DICTATION_CAPTURE_DAEMON_PROOF ?? '';
  }
  if (req.headers['content-type']) headers['Content-Type'] = req.headers['content-type'];
  if (body.length > 0) headers['Content-Length'] = body.length;
  const upstream = http.request(
    new URL(url.pathname + url.search, realDaemon),
    { method: req.method, headers },
    (answer) => {
      const parts = [];
      answer.on('data', (c) => parts.push(c));
      answer.on('end', () => {
        const raw = Buffer.concat(parts);
        onAnswer?.(answer.statusCode, raw);
        const out = {};
        if (answer.headers['content-type']) out['Content-Type'] = answer.headers['content-type'];
        if (answer.headers['retry-after']) out['Retry-After'] = answer.headers['retry-after'];
        res.writeHead(answer.statusCode, out).end(raw);
      });
    }
  );
  upstream.on('error', (error) => res.writeHead(502).end(String(error)));
  upstream.end(body.length > 0 ? body : undefined);
}
/** Keeps the real daemon's transcript and when it was relayed. */
function keepTranscript(status, raw) {
  if (status !== 200) return;
  daemon.finishAnsweredAt = Date.now();
  daemon.transcript = JSON.parse(raw.toString('utf8'));
}

const server = http.createServer((req, res) => {
  const at = Date.now();
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
  if (req.method === 'OPTIONS') {
    daemon.preflights = (daemon.preflights ?? 0) + 1;
    return res.writeHead(204).end();
  }
  const url = new URL(req.url, 'http://127.0.0.1');
  const parts = [];
  req.on('data', (c) => parts.push(c));
  req.on('end', () => {
    const body = Buffer.concat(parts);
    const json = (status, value) =>
      res.writeHead(status, { 'Content-Type': 'application/json' }).end(JSON.stringify(value));
    const secretOk = req.headers['x-secret-key'] === 'probe-secret';
    if (!secretOk) return json(401, { code: 'unauthorized', message: '' });
    if (realDaemon) {
      // The same records as below; the answers are the real daemon's.
      if (req.method === 'GET' && url.pathname === '/dictation/status') daemon.statusReads++;
      if (req.method === 'POST' && url.pathname === '/dictation/streams') {
        daemon.creates.push({
          at,
          proof: req.headers['x-user-action'] ?? null,
          body: JSON.parse(body.toString('utf8')),
        });
      }
      if (req.method === 'PUT') {
        daemon.inflight++;
        daemon.maxInflight = Math.max(daemon.maxInflight, daemon.inflight);
        daemon.bytes += body.length;
        const seq = Number(url.searchParams.get('seq'));
        daemon.puts.push({ at, seq, bytes: body.length, type: req.headers['content-type'] });
        (daemon.bodies ??= new Map()).set(seq, body);
        return forward(req, url, body, res, () => daemon.inflight--);
      }
      if (req.method === 'POST' && url.pathname.endsWith('/finish')) {
        const seq = url.searchParams.has('seq') ? Number(url.searchParams.get('seq')) : null;
        daemon.finishes.push({ at, seq, bytes: body.length, type: req.headers['content-type'] });
        if (seq !== null && body.length > 0) {
          daemon.bytes += body.length;
          (daemon.bodies ??= new Map()).set(seq, body);
        }
        return forward(req, url, body, res, keepTranscript);
      }
      if (req.method === 'GET' && url.pathname.startsWith('/dictation/streams/')) {
        daemon.reads = (daemon.reads ?? 0) + 1;
        return forward(req, url, body, res, keepTranscript);
      }
      if (req.method === 'DELETE') daemon.deletes.push({ at, path: url.pathname });
      return forward(req, url, body, res);
    }
    if (req.method === 'GET' && url.pathname === '/dictation/status') {
      daemon.statusReads++;
      return json(200, STATUS);
    }
    if (req.method === 'POST' && url.pathname === '/dictation/streams') {
      daemon.creates.push({
        at,
        proof: req.headers['x-user-action'] ?? null,
        body: JSON.parse(body.toString('utf8')),
      });
      return json(201, {
        stream_id: `probe${daemon.creates.length}`,
        max_chunk_bytes: 524288,
        idle_timeout_s: 60,
        max_seconds: 600,
      });
    }
    if (req.method === 'PUT') {
      daemon.inflight++;
      daemon.maxInflight = Math.max(daemon.maxInflight, daemon.inflight);
      daemon.bytes += body.length;
      const seq = Number(url.searchParams.get('seq'));
      daemon.puts.push({ at, seq, bytes: body.length, type: req.headers['content-type'] });
      // The last body for each seq, as the daemon treats a repeat.
      (daemon.bodies ??= new Map()).set(seq, body);
      // A little decode work on the daemon's side, so overlapping PUTs would show.
      setTimeout(() => {
        daemon.inflight--;
        json(200, { received_s: daemon.bytes / 32000, queue_position: 0 });
      }, 15);
      return;
    }
    if (req.method === 'POST' && url.pathname.endsWith('/finish')) {
      // `finish` carries the last chunk (PR-15), with its seq, as the daemon takes it.
      const seq = url.searchParams.has('seq') ? Number(url.searchParams.get('seq')) : null;
      daemon.finishes.push({ at, seq, bytes: body.length, type: req.headers['content-type'] });
      if (seq !== null && body.length > 0) {
        daemon.bytes += body.length;
        (daemon.bodies ??= new Map()).set(seq, body);
      }
      daemon.finishAnsweredAt = Date.now();
      return json(200, { ...TRANSCRIPT, audio_s: daemon.bytes / 32000 });
    }
    if (req.method === 'DELETE') {
      daemon.deletes.push({ at, path: url.pathname });
      return res.writeHead(204).end();
    }
    return json(404, { code: 'notFound', message: '' });
  });
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const apiUrl = `http://127.0.0.1:${server.address().port}`;

// ── run ─────────────────────────────────────────────────────────────────────
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let app;
let failure = null;
try {
  app = await electron.launch({
    // A fresh profile in the scratch folder: every run starts with no stored
    // permission or device state, and nothing is left in the person's own
    // application support folder.
    args: [asarPath, `--user-data-dir=${path.join(work, 'profile')}`],
    cwd: desktopDir,
    env: {
      ...process.env,
      ELECTRON_RUN_AS_NODE: '',
      BIOROUTER_TEST_FAKE_MIC_WAV: speechWav ?? wav,
      PROBE_API: apiUrl,
      PROBE_RENDER_MS: String(renderMs),
    },
  });
  const page = await app.firstWindow();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.waitForFunction(() => Boolean(window.probe));
  const pageUrl = page.url();
  assert(
    pageUrl.startsWith('file://') && pageUrl.includes('/app.asar/dist/index.html'),
    `not loaded from app.asar: ${pageUrl}`
  );

  // As in the app, interest in dictation (a composer's field taking focus) comes
  // before the press and warms the device list; BIOROUTER_DICTATION_CAPTURE_COLD=1
  // measures a press with nothing before it.
  results.cold = process.env.BIOROUTER_DICTATION_CAPTURE_COLD === '1';
  if (!results.cold) {
    await page.evaluate(() => window.probe.showInterest());
    await sleep(1000);
  }

  // 1 to 3 hear the tone; a speech run goes straight to 4.
  if (!speechWav) {
    // 1. Capture a tone in the shown window.
    await page.click('#capture');
    await page.waitForFunction(
      () => window.probe.captureState().readyAt !== undefined || window.probe.captureState().error
    );
    const early = await page.evaluate(() => window.probe.captureState());
    assert.equal(early.error, undefined, `capture failed: ${JSON.stringify(early.error)}`);
    await sleep(4000);
    await page.click('#stop');
    const shown = await page.evaluate(() => window.__stopped);
    results.capture = shown;
    assert.equal(
      shown.mode,
      'worklet',
      'capture fell back to MediaRecorder: the worklet did not load'
    );
    assert.equal(shown.sampleRate, 16000);
    assert(
      Math.abs(shown.rms - EXPECTED_RMS) <= 0.005,
      `RMS ${shown.rms} is not ${EXPECTED_RMS.toFixed(4)} +/- 0.005`
    );
    assert(Math.abs(shown.hz - TONE_HZ) <= 2, `frequency ${shown.hz} Hz is not 440 +/- 2`);
    // What capture.ts and its worklet can lose: the samples against the time the capture's
    // own audio graph rendered, which the worklet turns into samples frame for frame. What
    // the track delivered and the wall time also count what a starved runner's audio thread
    // never rendered (94.0 % of the track and 86.6 % of the wall time on two runs), so the
    // wall time is a stall guard only: a suspended or stopped capture renders far less.
    // Where the graph's time is missing, the track's and then the wall time's stand in.
    const coverage =
      shown.graphCoverage != null
        ? {
            of: `the ${shown.renderedS.toFixed(2)} s the audio graph rendered`,
            is: shown.graphCoverage,
          }
        : shown.trackCoverage != null
          ? {
              of: `the ${shown.deliveredS.toFixed(2)} s the track delivered`,
              is: shown.trackCoverage,
            }
          : { of: 'wall time', is: shown.sampleCoverage };
    assert(
      coverage.is >= 0.98,
      `samples cover ${(coverage.is * 100).toFixed(1)} % of ${coverage.of}`
    );
    assert(
      shown.sampleCoverage >= 0.75,
      `samples cover ${(shown.sampleCoverage * 100).toFixed(1)} % of wall time: the capture stalled`
    );
    assert(shown.tracksEnded, 'a track is still live after stop');
    assert(shown.contextsClosed, 'an AudioContext is still open after stop');
    // A loose bound that only a capture waiting on something it should not (the
    // daemon, a status read) would cross; the numbers themselves are printed below.
    assert(
      shown.pressToLiveMs < 1000,
      `the first audio frame came ${shown.pressToLiveMs} ms after the press`
    );

    const requests = await app.evaluate(() => globalThis.__requests);
    const workletUrl = requests.find((url) =>
      /\/app\.asar\/dist\/assets\/pcmCapture\.worklet-[^/]+\.js$/.test(url)
    );
    assert(workletUrl, 'the worklet was not loaded from app.asar/dist/assets');
    results.workletUrl = workletUrl.slice(workletUrl.indexOf('app.asar'));
    const violations = await page.evaluate(() => window.probe.violations());
    assert.deepEqual(
      violations,
      [],
      `content security policy violations: ${violations.join(', ')}`
    );

    // 2. Refusals: audio with video, then a document outside the renderer.
    const audioVideo = await page.evaluate(() =>
      window.probe.tryGetUserMedia({ audio: true, video: true })
    );
    assert.equal(
      audioVideo.name,
      'NotAllowedError',
      `audio with video: ${JSON.stringify(audioVideo)}`
    );
    const foreign = await app.evaluate(
      async ({ BrowserWindow }, href) => {
        const win = new BrowserWindow({
          show: false,
          webPreferences: {
            partition: 'persist:dictation-probe',
            contextIsolation: true,
            sandbox: true,
          },
        });
        await win.loadURL(href);
        const answer = await win.webContents.executeJavaScript(
          `navigator.mediaDevices.getUserMedia({ audio: true }).then(() => ({ ok: true }), (e) => ({ ok: false, name: e.name }))`
        );
        win.destroy();
        return answer;
      },
      pathToFileURL(path.join(foreignDir, 'foreign.html')).href
    );
    assert.equal(foreign.name, 'NotAllowedError', `foreign document: ${JSON.stringify(foreign)}`);
    results.refusals = { audioVideo, foreign };

    // 3. A hidden window captures.
    const hidden = await app.evaluate(async ({ BrowserWindow }, href) => {
      const win = new BrowserWindow({
        show: false,
        webPreferences: {
          partition: 'persist:dictation-probe',
          contextIsolation: true,
          sandbox: true,
        },
      });
      await win.loadURL(href);
      const wait = (ms) => new Promise((r) => setTimeout(r, ms));
      for (
        let i = 0;
        i < 100 && !(await win.webContents.executeJavaScript('Boolean(window.probe)'));
        i++
      )
        await wait(50);
      await win.webContents.executeJavaScript(`document.getElementById('capture').click()`);
      await wait(2500);
      const answer = await win.webContents.executeJavaScript('window.probe.stopCapture()');
      win.destroy();
      return answer;
    }, page.url().split('#')[0]);
    results.hiddenWindow = hidden;
    assert.equal(hidden.mode, 'worklet');
    if (hidden.graphCoverage != null) {
      assert(
        hidden.graphCoverage >= 0.98,
        `a hidden window's samples cover ${(hidden.graphCoverage * 100).toFixed(1)} % of what its audio graph rendered`
      );
    }
    // A hidden window whose capture were suspended would render far below this; a starved
    // runner's has measured 86.6 %.
    assert(
      hidden.sampleCoverage >= 0.75,
      `a hidden window captured ${(hidden.sampleCoverage * 100).toFixed(1)} % of wall time`
    );
    assert(
      hidden.tracksEnded && hidden.contextsClosed,
      'the hidden window left a track or a context open'
    );

    const permissions = await app.evaluate(() => globalThis.__permissionLog);
    assert(
      permissions.some(
        (p) =>
          p.permission === 'media' &&
          p.mediaTypes.length === 1 &&
          p.mediaTypes[0] === 'audio' &&
          p.answer === true
      ),
      'audio alone was never granted by the handler'
    );
    results.permissionLog = permissions;
  }

  // 4. One whole dictation through the session and the transport.
  const prepared = await page.evaluate(() => window.probe.prepareDictation());
  assert(prepared.focused, 'the field did not take focus');
  assert.equal(daemon.statusReads, 1, 'the status was not read exactly once before the press');
  await page.click('#dictate');
  await sleep(dictateMs);
  await page.click('#dictate');
  await page.waitForFunction(() => window.probe.dictationState().value.length > 0, null, {
    timeout: 15000,
  });
  await sleep(200);
  const dictated = await page.evaluate(() => window.probe.dictationState());
  // The page's own clock at each press, not the moment this script asked Playwright to click.
  assert.equal(dictated.presses.length, 2, `${dictated.presses.length} presses reached the page`);
  const pressAt = dictated.presses[0].at;
  const stopAt = dictated.presses[1].at;
  assert.equal(
    dictated.presses[1].activity,
    'recording',
    'the second press did not land while recording'
  );
  assert.equal(
    dictated.value,
    realDaemon ? daemon.transcript?.text : TRANSCRIPT.text,
    `the field holds ${JSON.stringify(dictated.value)}`
  );
  assert.equal(daemon.creates.length, 1);
  assert.equal(daemon.creates[0].proof, 'probe-proof', 'stream creation did not carry the proof');
  assert.deepEqual(daemon.creates[0].body, {
    target_kind: 'chat',
    session_id: 'probe-chat',
    sample_rate: 16000,
    format: 's16le',
  });
  assert.equal(daemon.maxInflight, 1, `${daemon.maxInflight} PUTs were in flight at once`);
  assert.deepEqual(
    daemon.puts.map((p) => p.seq),
    daemon.puts.map((_, i) => i),
    'PUT seqs are not 0, 1, 2, ...'
  );
  assert(daemon.puts.every((p) => p.type === 'application/octet-stream'));
  assert(
    daemon.puts.filter((p) => p.at < stopAt).length >= 1,
    'no audio went up while the person spoke'
  );
  assert.equal(daemon.finishes.length, 1);
  // The stop costs one round trip: what was left went up with `finish`, not a PUT.
  const putsAfterStop = daemon.puts.filter((p) => p.at >= stopAt);
  assert.equal(putsAfterStop.length, 0, `${putsAfterStop.length} PUTs went up after the stop`);
  const [finish] = daemon.finishes;
  assert(
    finish.bytes === 0 || finish.type === 'application/octet-stream',
    'the tail chunk on finish was not raw audio'
  );
  assert.equal(
    finish.seq,
    finish.bytes > 0 ? daemon.puts.length : null,
    'finish carried the wrong seq'
  );
  const audioSeconds = daemon.bytes / 32000;
  assert(
    audioSeconds > dictateMs / 1000 - 0.5 && audioSeconds < dictateMs / 1000 + 0.6,
    `the daemon received ${audioSeconds} s of audio for a ${dictateMs / 1000} s dictation`
  );
  // What the stand in received, in seq order, for a model to transcribe.
  const received = [...daemon.bodies.entries()].sort(([a], [b]) => a - b).map(([, body]) => body);
  await fs.writeFile(
    path.join(outputDir, 'received.wav'),
    monoWav(Buffer.concat(received), daemon.creates[0].body.sample_rate)
  );
  assert(
    dictated.busy[0] === true && dictated.busy.at(-1) === false,
    `dictation:busy went ${JSON.stringify(dictated.busy)}`
  );
  assert(daemon.deletes.length >= 1, 'the stream was not released after the insert');
  const sent = (method, test) =>
    dictated.fetches.filter((f) => f.method === method && test(f.path));
  const finishSent = sent('POST', (p) => p.endsWith('/finish'))[0];
  const createSent = sent('POST', (p) => p === '/dictation/streams')[0];
  const afterStop = (kind) => dictated.portEvents.find((e) => e.kind === kind && e.at >= stopAt);
  const flushPosted = afterStop('flush');
  const finalChunk = afterStop('final');
  assert(flushPosted && finalChunk, 'the stop did not flush the worklet');
  results.dictation = {
    audioSeconds,
    preflights: daemon.preflights ?? 0,
    puts: daemon.puts.map((p) => ({ seq: p.seq, bytes: p.bytes, msAfterPress: p.at - pressAt })),
    pressToRecordingMs: dictated.recordingAt === null ? null : dictated.recordingAt - pressAt,
    pressToCreateSentMs: createSent.at - pressAt,
    pressToCreateMs: daemon.creates[0].at - pressAt,
    pressToFirstPutMs: daemon.puts[0].at - pressAt,
    stopToFlushPostedMs: flushPosted.at - stopAt,
    stopToFinalChunkMs: finalChunk.at - stopAt,
    finishTailBytes: finish.bytes,
    stopToFinishSentMs: finishSent.at - stopAt,
    stopToFinishMs: daemon.finishes[0].at - stopAt,
    finishAnswerToInsertedMs: dictated.insertedAt - daemon.finishAnsweredAt,
    stopToInsertedMs: dictated.insertedAt - stopAt,
    snapshot: dictated.snapshot,
    heldReads: daemon.reads ?? 0,
    transcript: realDaemon
      ? {
          text: daemon.transcript?.text,
          model: daemon.transcript?.model,
          elapsedMs: daemon.transcript?.elapsed_ms,
          timings: daemon.transcript?.timings,
          biasScope: daemon.transcript?.bias_scope,
        }
      : null,
  };
  assert.deepEqual(errors, [], `renderer errors: ${errors.join(' | ')}`);
} catch (error) {
  failure = error;
} finally {
  await app?.close().catch(() => undefined);
  server.close();
  await fs.writeFile(
    path.join(outputDir, 'dictation-capture.json'),
    JSON.stringify(results, null, 2)
  );
}

const ms = (n) => (typeof n === 'number' ? `${n.toFixed(1)} ms` : 'n/a');
if (results.capture) {
  const c = results.capture;
  console.log('capture (shown window, 16 kHz worklet from app.asar):');
  console.log(
    `  RMS ${c.rms.toFixed(5)} (expected ${EXPECTED_RMS.toFixed(5)}), ${c.hz.toFixed(2)} Hz, samples cover ${(c.sampleCoverage * 100).toFixed(2)} % of ${c.wallS.toFixed(2)} s` +
      (c.trackCoverage != null
        ? ` and ${(c.trackCoverage * 100).toFixed(2)} % of the ${c.deliveredS.toFixed(2)} s the track delivered`
        : ', the track gave no statistics') +
      (c.graphCoverage != null
        ? `, ${(c.graphCoverage * 100).toFixed(2)} % of the ${c.renderedS.toFixed(2)} s the audio graph rendered`
        : '')
  );
  console.log(
    `  press to getUserMedia ${ms(c.pressToGetUserMediaMs)}, to worklet module ${ms(c.pressToModuleMs)}, to connected ${ms(c.pressToConnectedMs)}`
  );
  console.log(
    `  press to first audio frame ${ms(c.pressToLiveMs)}, to first non-silent sample ${ms(c.pressToFirstNonSilentSampleMs)}, to first 100 ms chunk ${ms(c.pressToFirstChunkMs)}`
  );
  console.log(
    `  stop to final chunk ${ms(c.stopToFinalChunkMs)}, to context closed ${ms(c.stopToContextClosedMs)}`
  );
}
if (results.hiddenWindow) {
  console.log(
    `hidden window: samples cover ${(results.hiddenWindow.sampleCoverage * 100).toFixed(2)} %, press to first frame ${ms(results.hiddenWindow.pressToLiveMs)}`
  );
}
if (results.dictation) {
  const d = results.dictation;
  console.log(
    `dictation through the session and the transport (${realDaemon ? `the daemon at ${realDaemon}` : 'loopback daemon'}):`
  );
  console.log(
    `  ${d.audioSeconds.toFixed(2)} s of audio in ${d.puts.length} PUTs, ${d.preflights} CORS preflights; press to recording (first audio frame) ${d.pressToRecordingMs} ms, to create sent ${d.pressToCreateSentMs} ms (received ${d.pressToCreateMs} ms), to first PUT ${d.pressToFirstPutMs} ms`
  );
  console.log(
    `  stop to flush posted ${d.stopToFlushPostedMs} ms, to the worklet's last chunk ${d.stopToFinalChunkMs} ms, to finish sent with ${d.finishTailBytes} B of tail ${d.stopToFinishSentMs} ms (received ${d.stopToFinishMs} ms)`
  );
  console.log(
    realDaemon
      ? `  transcript to text in the field ${d.finishAnswerToInsertedMs} ms, stop to text ${d.stopToInsertedMs} ms; the daemon's finish to transcript ${d.transcript.elapsedMs} ms (${JSON.stringify(d.transcript.timings)}), ${d.heldReads} held reads, ${d.transcript.model}: ${JSON.stringify(d.transcript.text)}`
      : `  transcript to text in the field ${d.finishAnswerToInsertedMs} ms, stop to text ${d.stopToInsertedMs} ms (the loopback daemon adds 15 ms per PUT)`
  );
}
console.log(`evidence: ${path.join(outputDir, 'dictation-capture.json')}`);
if (failure) {
  console.error(failure);
  process.exit(1);
}
