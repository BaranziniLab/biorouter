#!/usr/bin/env node
/**
 * Stage the `biorouter-fnkey` helper (crates/biorouter-fnkey) into `src/bin/`, beside
 * `biorouterd`, so `extraResource` carries it to `Biorouter.app/Contents/Resources/bin/`, where
 * `fnKeyHelperPath` (src/utils/fnKeyHelper.ts) looks for it in a packaged app.
 *
 * It is a plain executable, not an app bundle: the app starts it as a child process, so macOS
 * attributes its Input Monitoring permission to Biorouter itself, which is the name a person sees
 * in System Settings. Forge signs it with the rest of `Resources/bin` (hardened runtime, Developer
 * ID) and notarization covers it; it needs no entitlement.
 *
 * macOS only, and a no-op elsewhere: no other system lets an app see the fn key.
 *
 * `cargo build --release` (and `--target x86_64-apple-darwin`) already builds it, because it is a
 * workspace member; `scripts/release.sh` builds the whole workspace for both Mac targets.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktop = path.resolve(here, '..');
const repo = path.resolve(desktop, '../..');
const NAME = 'biorouter-fnkey';

if (process.platform !== 'darwin') {
  console.log('[fn-key-helper] not macOS — nothing to stage');
  process.exit(0);
}

const arch = process.env.ELECTRON_ARCH === 'x64' ? 'x64' : 'arm64';
const target = arch === 'x64' ? 'x86_64-apple-darwin' : null;
const built = target
  ? path.join(repo, 'target', target, 'release', NAME)
  : path.join(repo, 'target', 'release', NAME);

if (!fs.existsSync(built)) {
  // Loud, not silent: a build without it ships a fn key switch that can only say
  // MSG-FN-MISSING, and an arm64 helper in an Intel app would not start at all.
  console.error(`[fn-key-helper] MISSING: ${built}`);
  const cmd = target
    ? `cargo build --release --target ${target} -p ${NAME}`
    : `cargo build --release -p ${NAME}`;
  console.error(`[fn-key-helper] build it first:  ${cmd}`);
  process.exit(1);
}

const bin = path.join(desktop, 'src', 'bin');
fs.mkdirSync(bin, { recursive: true });
const staged = path.join(bin, NAME);
fs.copyFileSync(built, staged);
fs.chmodSync(staged, 0o755);
console.log(`[fn-key-helper] staged ${path.relative(desktop, staged)} (${arch})`);
