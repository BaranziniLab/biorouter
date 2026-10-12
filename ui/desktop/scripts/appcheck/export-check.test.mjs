import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkExport } from './export-check-lib.mjs';

function rawStaticExport() {
  return {
    files: {
      'manifest.json': JSON.stringify({ id: 'legacyID_1', entry: 'index.html' }),
      'index.html': '<html>raw entry</html>',
      'package.json': '{}',
      'serve.mjs': '',
      'biorouter-launch.sh': '',
      'run.sh': '',
      'run.command': '',
      'run.ps1': '',
      'run.bat': '',
    },
    skipped_binary: ['assets/logo.png'],
  };
}

test('legacy IDs and raw static entries pass and report skipped binaries', async (context) => {
  context.mock.method(globalThis, 'fetch', async (url) => {
    assert.equal(url.pathname, '/apps/legacyID_1/export');
    return { ok: true, json: async () => rawStaticExport() };
  });
  const result = await checkExport('http://127.0.0.1:3000', 'legacyID_1');
  assert.equal(result.ok, true);
  assert.equal(result.rawEntry, true);
  assert.deepEqual(result.skippedBinary, ['assets/logo.png']);
});

test('an assembled runtime config island fails the export check', async (context) => {
  const data = rawStaticExport();
  data.files['index.html'] = '<script id="biorouter-app-config">stale</script>';
  context.mock.method(globalThis, 'fetch', async () => ({ ok: true, json: async () => data }));
  const result = await checkExport('http://127.0.0.1:3000', 'legacyID_1');
  assert.equal(result.ok, false);
  assert.match(result.err, /stale runtime config island/);
});

test('traversal in an ID or an exported file path fails safely', async (context) => {
  const data = rawStaticExport();
  data.files['../outside.txt'] = 'unsafe';
  const request = context.mock.method(globalThis, 'fetch', async () => ({ ok: true, json: async () => data }));
  assert.match((await checkExport('http://127.0.0.1:3000', '../escape')).err, /invalid app id/);
  assert.equal(request.mock.callCount(), 0);
  assert.match((await checkExport('http://127.0.0.1:3000', 'legacyID_1')).err, /unsafe file path/);
});
