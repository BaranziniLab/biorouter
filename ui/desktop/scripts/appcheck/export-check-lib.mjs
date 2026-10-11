import { mkdtemp, mkdir, writeFile, chmod, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, isAbsolute } from 'node:path';
import { createRequire } from 'node:module';

export function exportHeaders() {
  const secret = process.env.BIOROUTER_SERVER__SECRET_KEY;
  return secret ? { 'X-Secret-Key': secret } : {};
}

export async function checkExport(base, id) {
  const result = { id, files: 0, hasRun: false, hasDist: false, rawEntry: false, rebuildOk: false, skippedBinary: [], ok: false, err: '' };
  let out;
  try {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(id || '')) throw new Error('invalid app id');
    const response = await fetch(new URL(`/apps/${id}/export`, base), { headers: exportHeaders() });
    if (!response.ok) throw new Error(`export http ${response.status}`);
    const data = await response.json();
    const files = data.files || {};
    const manifest = JSON.parse(files['manifest.json'] || '{}');
    const entry = manifest.entry || 'index.html';
    const needed = ['manifest.json', entry, 'package.json', 'serve.mjs', 'biorouter-launch.sh', 'run.sh', 'run.command', 'run.ps1', 'run.bat'];
    const missing = needed.filter((name) => typeof files[name] !== 'string');
    if (missing.length) throw new Error(`missing: ${missing.join(', ')}`);
    result.files = Object.keys(files).length;
    result.hasRun = true;
    result.hasDist = typeof files['dist/app.js'] === 'string';
    result.rawEntry = !files[entry].includes('biorouter-app-config');
    if (!result.rawEntry) throw new Error('entry contains a stale runtime config island');
    if (manifest.session_id) throw new Error('export carries a build chat id');
    result.skippedBinary = data.skipped_binary || [];
    out = await mkdtemp(join(tmpdir(), `biorouter-export-${id}-`));
    for (const [rel, contents] of Object.entries(files)) {
      if (isAbsolute(rel) || rel.includes('\\') || rel.split('/').some((part) => !part || part === '.' || part === '..')) {
        throw new Error('export contains an unsafe file path');
      }
      const path = join(out, rel);
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, contents);
      if (['run.sh', 'run.command', 'biorouter-launch.sh'].includes(rel)) await chmod(path, 0o755);
    }
    if ('src/main.ts' in files) {
      const require = createRequire(import.meta.url);
      const esbuild = require('esbuild');
      esbuild.buildSync({ entryPoints: [join(out, 'src/main.ts')], bundle: true, format: 'iife', target: 'es2018', outfile: join(out, 'dist/app.js'), logLevel: 'silent' });
    }
    result.rebuildOk = true;
    result.ok = true;
  } catch (error) {
    result.err = String(error.message || error).slice(0, 500);
  } finally {
    if (out) await rm(out, { recursive: true, force: true });
  }
  return result;
}
