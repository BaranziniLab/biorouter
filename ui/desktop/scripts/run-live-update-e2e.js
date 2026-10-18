#!/usr/bin/env node
/*
 * Repeatable runner for the live electron-updater end-to-end test. Spawns a
 * REAL Electron process (live-update-e2e.js) twice — valid + tampered — and
 * asserts the real engine's behavior against our generated latest-mac.yml.
 *
 *   node scripts/run-live-update-e2e.js
 *
 * Requires a usable Electron binary (node_modules/electron/dist). If absent
 * (e.g. CI that didn't fetch Electron), it SKIPS with a clear message rather
 * than failing — the parser/arch/checksum logic is already covered headlessly
 * by scripts/electron-updater-compat.test.js.
 *
 * What it proves end-to-end, inside real Electron:
 *   • our latest-mac.yml is fetched + parsed → `update-available`,
 *   • the zip really downloads from the served release → `download-progress`,
 *   • a good download passes electron-updater's sha512 verification and reaches
 *     `update-downloaded` (the exact state that enables the one-click
 *     "Restart & Update" button), and
 *   • a corrupted download is rejected with a sha512 checksum-mismatch error
 *     and never reaches the installable state.
 *
 * The only step beyond this is the final quitAndInstall OS swap, which macOS
 * Squirrel.Mac gates on a notarized code signature (see the testing checklist).
 */
const { spawnSync, execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const DESK = path.resolve(__dirname, '..');
const harness = path.join(__dirname, 'live-update-e2e.js');

/**
 * Bundle the app's resumable update download for the harness, which is plain
 * JavaScript inside Electron and cannot import TypeScript. Electron stays
 * external: the harness runs inside it.
 */
function bundleResumableDownload() {
  const outDir = fs.mkdtempSync(path.join(os.tmpdir(), 'br-live-bundle-'));
  const entry = path.join(outDir, 'entry.ts');
  fs.writeFileSync(
    entry,
    `export { installResumableUpdaterDownloads } from ${JSON.stringify(
      path.join(DESK, 'src', 'utils', 'resumableUpdaterDownloads.ts')
    )};\n` +
      `export { electronNetOpenPart } from ${JSON.stringify(
        path.join(DESK, 'src', 'utils', 'electronNetTransport.ts')
      )};\n`
  );
  const outfile = path.join(outDir, 'resumable.cjs');
  require('esbuild').buildSync({
    entryPoints: [entry],
    bundle: true,
    platform: 'node',
    format: 'cjs',
    external: ['electron'],
    outfile,
    logLevel: 'error',
  });
  return outfile;
}

let resumableBundle = null;

// The updater writes a cache dir per run under the OS cache dir; clean any of
// ours so the test leaves no trace.
function cleanCaches() {
  const cacheRoot = path.join(os.homedir(), 'Library', 'Caches');
  try {
    for (const name of fs.readdirSync(cacheRoot)) {
      if (name.startsWith('br-live-e2e-')) {
        fs.rmSync(path.join(cacheRoot, name), { recursive: true, force: true });
      }
    }
  } catch {
    /* not macOS or unreadable — ignore */
  }
}

function electronUsable() {
  try {
    const p = require('electron');
    if (typeof p !== 'string' || !fs.existsSync(p)) return false;
    // It must run as a real Electron (GUI), not as plain node.
    const out = execFileSync(p, ['--version'], {
      env: { ...process.env, ELECTRON_RUN_AS_NODE: '' },
      encoding: 'utf8',
      timeout: 30000,
    });
    return /^v?\d+\./.test(out.trim());
  } catch {
    return false;
  }
}

function runMode(extraArgs) {
  const env = { ...process.env, BR_RESUMABLE_BUNDLE: resumableBundle };
  delete env.ELECTRON_RUN_AS_NODE; // ensure GUI Electron, not node mode
  const res = spawnSync(require('electron'), [harness, ...extraArgs], {
    cwd: DESK,
    env,
    encoding: 'utf8',
    timeout: 120000,
  });
  const line = (res.stdout || '')
    .split('\n')
    .find((l) => l.startsWith('RESULT '));
  if (!line) {
    throw new Error(
      `no RESULT line (exit=${res.status})\nstdout:\n${res.stdout}\nstderr:\n${res.stderr}`
    );
  }
  return JSON.parse(line.slice('RESULT '.length));
}

function assert(cond, msg) {
  if (!cond) throw new Error('ASSERT FAILED: ' + msg);
}

function names(r) {
  return r.events.map((e) => e.name);
}

function main() {
  if (!electronUsable()) {
    console.log(
      'SKIP: no usable Electron runtime (node_modules/electron/dist missing).\n' +
        '      Headless coverage of parse/arch/checksum: scripts/electron-updater-compat.test.js'
    );
    process.exit(0);
  }

  console.log('live electron-updater end-to-end (real Electron process)');
  resumableBundle = bundleResumableDownload();

  // --- valid: full happy path to the installable state ---
  const valid = runMode([]);
  assert(names(valid).includes('update-available'), 'valid: update-available fired');
  const av = valid.events.find((e) => e.name === 'update-available');
  assert(av.data.version === '99.0.0', 'valid: offered version parsed from our manifest');
  assert(names(valid).includes('download-progress'), 'valid: real download-progress fired');
  assert(valid.terminal === 'downloaded', 'valid: reached update-downloaded (one-click ready)');
  console.log('  ✓ valid: checking → available(99.0.0) → download → update-downloaded');

  // --- tampered: integrity gate must reject ---
  const tampered = runMode(['--tampered']);
  assert(names(tampered).includes('update-available'), 'tampered: update-available fired');
  assert(tampered.terminal === 'error', 'tampered: terminated in error (not installable)');
  const err = tampered.events.find((e) => e.name === 'error');
  assert(
    /sha512|checksum|mismatch/i.test(err.data.message),
    `tampered: error is a checksum mismatch (got: ${err.data.message})`
  );
  assert(
    tampered.terminal !== 'downloaded',
    'tampered: never reached the installable state'
  );
  console.log('  ✓ tampered: checking → available → REJECTED (sha512 checksum mismatch)');

  // --- reset: the connection drops 40% in, and the download resumes ---
  const reset = runMode(['--reset']);
  assert(reset.terminal === 'downloaded', `reset: reached update-downloaded (got ${reset.terminal})`);
  assert(reset.zipRequests.length === 2, `reset: two requests for the zip (${reset.zipRequests})`);
  assert(reset.zipRequests[0] === null, 'reset: the first request asks for the whole file');
  assert(
    /^bytes=[1-9]\d*-$/.test(reset.zipRequests[1] || ''),
    `reset: the second asks for the rest (${reset.zipRequests[1]})`
  );
  console.log(`  ✓ reset: dropped 40% in → resumed with ${reset.zipRequests[1]} → update-downloaded`);

  // --- reset, stock electron-updater: the bug this fixes ---
  const stockReset = runMode(['--reset', '--stock']);
  assert(stockReset.terminal === 'error', `stock reset: failed (got ${stockReset.terminal})`);
  const stockErr = stockReset.events.find((e) => e.name === 'error');
  console.log(`  ✓ stock electron-updater on the same reset fails: ${stockErr.data.message}`);

  cleanCaches();
  console.log('\n4 passed (live Electron engine)');
}

try {
  main();
} catch (e) {
  console.error('\nFAILED: ' + e.message);
  process.exit(1);
}
