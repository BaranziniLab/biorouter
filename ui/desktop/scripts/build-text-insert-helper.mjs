#!/usr/bin/env node
/**
 * Stage the `biorouter-textinsert` helper (crates/biorouter-textinsert) into `src/bin/`, beside
 * `biorouterd`, so `extraResource` carries it to `Biorouter.app/Contents/Resources/bin/`, where
 * `textInsertHelperPath` (src/utils/textInsertHelper.ts) looks for it in a packaged app.
 *
 * It is a plain executable, not an app bundle: the app starts it as a child process, so macOS
 * attributes its Accessibility permission to Biorouter itself, which is the name a person sees in
 * System Settings. Forge signs it with the rest of `Resources/bin` (hardened runtime, Developer
 * ID) and notarization covers it; it needs no entitlement, because posting events is granted by
 * Accessibility, not by the signature.
 *
 * macOS only, and a no-op elsewhere: Windows and Linux copy a finished dictation to the clipboard
 * instead of typing it.
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
const NAME = 'biorouter-textinsert';

if (process.platform !== 'darwin') {
  console.log('[text-insert-helper] not macOS, nothing to stage');
  process.exit(0);
}

const arch = process.env.ELECTRON_ARCH === 'x64' ? 'x64' : 'arm64';
const target = arch === 'x64' ? 'x86_64-apple-darwin' : null;
const built = target
  ? path.join(repo, 'target', target, 'release', NAME)
  : path.join(repo, 'target', 'release', NAME);

if (!fs.existsSync(built)) {
  // Loud, not silent: a build without it can never type into another app, and an arm64 helper
  // in an Intel app would not start at all, so every dictation there would only be copied.
  console.error(`[text-insert-helper] MISSING: ${built}`);
  const cmd = target
    ? `cargo build --release --target ${target} -p ${NAME}`
    : `cargo build --release -p ${NAME}`;
  console.error(`[text-insert-helper] build it first:  ${cmd}`);
  process.exit(1);
}

const bin = path.join(desktop, 'src', 'bin');
fs.mkdirSync(bin, { recursive: true });
const staged = path.join(bin, NAME);
fs.copyFileSync(built, staged);
fs.chmodSync(staged, 0o755);
console.log(`[text-insert-helper] staged ${path.relative(desktop, staged)} (${arch})`);
