/*
 * LIVE end-to-end test of the REAL electron-updater engine inside a REAL
 * Electron process, consuming the REAL latest-mac.yml we generate.
 *
 * Runs inside Electron (launched by run-live-update-e2e.js). It:
 *   1. fabricates an "update" payload zip (multi-MB so progress events fire),
 *   2. generates latest-mac.yml from it with our production generator,
 *   3. serves the release dir over local HTTP (generic provider layout),
 *   4. points electron-updater at it (forceDevUpdateConfig, served version
 *      99.0.0 > the running 39.x so an update is "available"),
 *   5. drives autoUpdater.checkForUpdates() and records the real events.
 *
 * mode=valid    → expect: update-available, real download to 100%, and the
 *                 sha512 integrity check PASSES (no checksum error). The only
 *                 thing that can fail afterwards is the macOS Squirrel
 *                 code-signature swap, which needs a notarized app — that
 *                 boundary is the documented, environment-gated limit.
 * mode=tampered → serve bytes whose sha512 does NOT match the manifest; expect
 *                 electron-updater to REJECT with a sha512/checksum error and
 *                 never reach "downloaded". Proves the integrity gate is real.
 * mode=reset    → the server RESETS the connection 40% into the zip, the way
 *                 the 1.93.0 → 1.93.1 update failed with
 *                 net::ERR_CONNECTION_RESET. With Biorouter's resumable
 *                 download installed (the default, as in the app) the next
 *                 request asks for `Range: bytes=<n>-` and the update still
 *                 reaches "downloaded"; with `--stock` it fails as it used to.
 *
 * Unless `--stock` is given, the harness installs the app's own resumable
 * download (src/utils/resumableUpdaterDownloads.ts + electronNetTransport.ts,
 * bundled by the runner and passed in BR_RESUMABLE_BUNDLE), so every mode
 * exercises the code the app ships.
 *
 * Prints a single line: RESULT <json> and exits.
 */
const { app } = require('electron');
const http = require('http');
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { autoUpdater } = require('electron-updater');
const { buildMacManifest } = require('./generate-update-manifests.js');

const mode = process.argv.includes('--tampered')
  ? 'tampered'
  : process.argv.includes('--reset')
    ? 'reset'
    : 'valid';
const stock = process.argv.includes('--stock');
/** The Range header of every request for the zip, null when there was none. */
const zipRequests = [];
const VERSION = '99.0.0';
const events = [];
const cleanupPaths = [];
let server;

function finish(extra) {
  const result = { mode, stock, events, zipRequests, ...extra };
  // Single, greppable result line.
  process.stdout.write('\nRESULT ' + JSON.stringify(result) + '\n');
  try {
    server && server.close();
  } catch {
    /* ignore */
  }
  for (const p of cleanupPaths) {
    try {
      fs.unlinkSync(p);
    } catch {
      /* ignore */
    }
  }
  app.exit(0);
}

app.disableHardwareAcceleration();

app.whenReady().then(async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'br-live-'));
  const zipName = `Biorouter-darwin-arm64-${VERSION}.zip`;
  const zipPath = path.join(tmp, zipName);

  // ~6 MB of deterministic bytes so multiple download-progress events fire.
  const realBytes = crypto.createHash('sha256').update('seed').digest();
  const payload = Buffer.alloc(6 * 1024 * 1024);
  for (let i = 0; i < payload.length; i += 32) realBytes.copy(payload, i);
  fs.writeFileSync(zipPath, payload);

  // Manifest is always computed from the REAL bytes...
  const { yaml } = buildMacManifest({
    version: VERSION,
    arm64Zip: zipPath,
    releaseDate: '2026-06-20T00:00:00.000Z',
  });
  fs.writeFileSync(path.join(tmp, 'latest-mac.yml'), yaml);

  // ...but in tampered mode we overwrite the served file with different bytes,
  // so its hash no longer matches the manifest the updater trusts.
  if (mode === 'tampered') {
    const bad = Buffer.from(payload);
    bad[0] = bad[0] ^ 0xff;
    bad[bad.length - 1] = bad[bad.length - 1] ^ 0xff;
    fs.writeFileSync(zipPath, bad);
  }

  // Serve the release dir. The zip honours Range, as GitHub's CDN does.
  server = http.createServer((req, res) => {
    const name = decodeURIComponent((req.url || '').split('?')[0].replace(/^\//, ''));
    const file = path.join(tmp, name);
    if (!file.startsWith(tmp) || !fs.existsSync(file)) {
      res.writeHead(404);
      res.end('not found');
      return;
    }
    const stat = fs.statSync(file);
    if (name !== zipName) {
      res.writeHead(200, { 'content-length': stat.size });
      fs.createReadStream(file).pipe(res);
      return;
    }
    const range = /bytes=(\d+)-/.exec(req.headers.range || '');
    zipRequests.push(req.headers.range || null);
    const start = range ? Number(range[1]) : 0;
    const body = fs.readFileSync(file).subarray(start);
    res.writeHead(range ? 206 : 200, {
      'content-length': body.length,
      'accept-ranges': 'bytes',
      ...(range ? { 'content-range': `bytes ${start}-${stat.size - 1}/${stat.size}` } : {}),
    });
    if (mode === 'reset' && zipRequests.length === 1) {
      // Send 40%, then a TCP RST: what a network that drops long transfers does.
      res.write(body.subarray(0, Math.floor(stat.size * 0.4)), () =>
        setTimeout(() => res.socket.resetAndDestroy(), 50)
      );
      return;
    }
    res.end(body);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const port = server.address().port;
  const url = `http://127.0.0.1:${port}`;

  // In dev, electron-updater loads its provider config from dev-app-update.yml
  // next to the entry script (even with setFeedURL). Write it with the live port.
  const devCfgPath = path.join(__dirname, 'dev-app-update.yml');
  // Unique cache dir per run so each invocation actually re-downloads and
  // re-verifies (a shared cache would let a tampered run reuse a good download).
  const cacheName = `br-live-e2e-${mode}-${process.pid}`;
  fs.writeFileSync(devCfgPath, `provider: generic\nurl: ${url}\nupdaterCacheDirName: ${cacheName}\n`);
  cleanupPaths.push(devCfgPath);

  autoUpdater.forceDevUpdateConfig = true;
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = false;
  autoUpdater.logger = null;
  autoUpdater.setFeedURL({ provider: 'generic', url });

  if (!stock) {
    const bundle = process.env.BR_RESUMABLE_BUNDLE;
    if (!bundle) {
      finish({ terminal: 'no-bundle' });
      return;
    }
    const { installResumableUpdaterDownloads, electronNetOpenPart } = require(bundle);
    if (!installResumableUpdaterDownloads(autoUpdater, electronNetOpenPart(), () => {})) {
      finish({ terminal: 'resumable-not-installed' });
      return;
    }
  }

  const record = (name, data) => events.push({ name, data });
  autoUpdater.on('checking-for-update', () => record('checking-for-update'));
  autoUpdater.on('update-available', (i) => record('update-available', { version: i.version }));
  autoUpdater.on('update-not-available', () => record('update-not-available'));
  autoUpdater.on('download-progress', (p) => record('download-progress', { percent: Math.round(p.percent) }));
  autoUpdater.on('update-downloaded', (i) => {
    record('update-downloaded', { version: i.version });
    finish({ terminal: 'downloaded' });
  });
  autoUpdater.on('error', (e) => {
    record('error', { message: String(e && e.message ? e.message : e) });
    finish({ terminal: 'error' });
  });

  // Safety timeout.
  setTimeout(() => finish({ terminal: 'timeout' }), 60000);

  try {
    await autoUpdater.checkForUpdates();
  } catch (e) {
    record('check-threw', { message: String(e && e.message ? e.message : e) });
    finish({ terminal: 'check-threw' });
  }
});
