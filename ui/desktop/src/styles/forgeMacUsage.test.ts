// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';

/**
 * The microphone on macOS (DD-65 in docs/dictation/implementation-plan.md).
 *
 * Two things in `forge.config.ts` decide whether dictation can ever hear
 * anything in a signed, hardened app, and neither shows up anywhere until a
 * person clicks the microphone:
 *
 * - `extendInfo.NSMicrophoneUsageDescription`, the sentence macOS shows when it
 *   asks. Without it the prompt says Electron's stock "This app needs access to
 *   the microphone".
 * - `osxSign.optionsForFile`, which signs the four files that record with
 *   `com.apple.security.device.audio-input`. Under the hardened runtime a file
 *   without it gets no prompt and every request is denied. Before it existed
 *   the four carried the entitlement only because @electron/osx-sign's default
 *   set happens to include it.
 *
 * These tests load the real config rather than matching its text, so they see
 * what the packager is handed. `osxSign` exists only when `APPLE_ID` is set.
 */

const desktop = resolve(__dirname, '../..');
const repo = resolve(desktop, '../..');
const configPath = resolve(desktop, 'forge.config.ts');
const nodeRequire = createRequire(configPath);

const USAGE =
  'Biorouter uses the microphone only while you dictate, and does not save the recording.';
const AUDIO_INPUT = 'com.apple.security.device.audio-input';

type SignOptions = { entitlements?: string | string[] };
type ForgeConfig = {
  packagerConfig: {
    extendInfo: Record<string, unknown>;
    osxSign?: { optionsForFile?: (filePath: string) => SignOptions };
  };
};

/** Loads `forge.config.ts` afresh, with `APPLE_ID` set to `appleId` or unset. */
function loadForgeConfig(appleId: string | undefined): ForgeConfig {
  const saved = process.env.APPLE_ID;
  const key = nodeRequire.resolve(configPath);
  if (appleId === undefined) delete process.env.APPLE_ID;
  else process.env.APPLE_ID = appleId;
  try {
    delete nodeRequire.cache[key];
    return nodeRequire(configPath) as ForgeConfig;
  } finally {
    delete nodeRequire.cache[key];
    if (saved === undefined) delete process.env.APPLE_ID;
    else process.env.APPLE_ID = saved;
  }
}

function optionsForFile(): (filePath: string) => SignOptions {
  const sign = loadForgeConfig('signing@example.test').packagerConfig.osxSign;
  expect(sign, 'osxSign is configured when APPLE_ID is set').toBeDefined();
  expect(typeof sign?.optionsForFile, 'osxSign.optionsForFile is a function').toBe('function');
  return sign!.optionsForFile!;
}

/** Where @electron/packager signs from: a temporary copy, so only the tail is fixed. */
const app =
  '/private/var/folders/xy/T/electron-packager/darwin-arm64/Biorouter-darwin-arm64/Biorouter.app';

const MICROPHONE_FILES = [
  app,
  `${app}/Contents/Frameworks/Biorouter Helper.app`,
  `${app}/Contents/Resources/bin/biorouter`,
  `${app}/Contents/Resources/bin/biorouterd`,
];

const OTHER_FILES = [
  `${app}/Contents/Frameworks/Biorouter Helper (GPU).app`,
  `${app}/Contents/Frameworks/Biorouter Helper (Renderer).app`,
  `${app}/Contents/Frameworks/Biorouter Helper (Plugin).app`,
  `${app}/Contents/Frameworks/Biorouter Helper (Renderer).app/Contents/MacOS/Biorouter Helper (Renderer)`,
  `${app}/Contents/Frameworks/Electron Framework.framework`,
  `${app}/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/libffmpeg.dylib`,
  `${app}/Contents/Resources/bin/Biorouter Authentication.app`,
  `${app}/Contents/Resources/bin/llamacpp/llama-server`,
  `${app}/Contents/Resources/bin/esbuild`,
  `${app}/Contents/Resources/bin/biorouter-extra`,
  `${app}/Contents/Resources/app.asar.unpacked/node_modules/node-pty/build/Release/pty.node`,
  '/somewhere/else/Biorouter.app.bak',
];

/** The keys of osx-sign's default entitlements for an app and its main helper. */
function osxSignDefaultDarwinEntitlements(): string[] {
  // Resolved from @electron/packager, which is what loads osx-sign when signing.
  const fromPackager = createRequire(nodeRequire.resolve('@electron/packager/package.json'));
  const osxSign = dirname(fromPackager.resolve('@electron/osx-sign/package.json'));
  const plist = readFileSync(resolve(osxSign, 'entitlements/default.darwin.plist'), 'utf8');
  return [...plist.matchAll(/<key>([^<]+)<\/key>\s*<true\/>/g)].map((match) => match[1]).sort();
}

describe('the macOS microphone prompt', () => {
  it('names what Biorouter does with the microphone', () => {
    for (const appleId of [undefined, 'signing@example.test']) {
      const { extendInfo } = loadForgeConfig(appleId).packagerConfig;
      expect(extendInfo.NSMicrophoneUsageDescription).toBe(USAGE);
    }
  });

  it('keeps the drag-and-drop document types beside it', () => {
    const { extendInfo } = loadForgeConfig(undefined).packagerConfig;
    expect(Array.isArray(extendInfo.CFBundleDocumentTypes)).toBe(true);
  });
});

describe('the macOS microphone entitlement', () => {
  it('signs the four files that record with audio input', () => {
    const forFile = optionsForFile();
    for (const file of MICROPHONE_FILES) {
      const { entitlements } = forFile(file);
      expect(Array.isArray(entitlements), file).toBe(true);
      expect(entitlements, file).toContain(AUDIO_INPUT);
    }
  });

  it("gives those four exactly osx-sign's default set, so nothing else about them changes", () => {
    // allow-jit in particular: V8 in the main process needs it under the
    // hardened runtime. If an osx-sign upgrade changes its default, decide
    // whether the four follow it, then update the list in forge.config.ts.
    const expected = osxSignDefaultDarwinEntitlements();
    expect(expected).toContain(AUDIO_INPUT);
    expect(expected).toContain('com.apple.security.cs.allow-jit');
    const forFile = optionsForFile();
    for (const file of MICROPHONE_FILES) {
      expect([...(forFile(file).entitlements as string[])].sort(), file).toEqual(expected);
    }
  });

  it("leaves every other file on osx-sign's own default", () => {
    const forFile = optionsForFile();
    for (const file of OTHER_FILES) {
      expect(forFile(file), file).toEqual({});
    }
  });

  it('hands osx-sign a new array each time, since it overwrites the one it gets', () => {
    const forFile = optionsForFile();
    const first = forFile(MICROPHONE_FILES[0]);
    first.entitlements = '/tmp/tmp-entitlements-x/entitlements.plist';
    expect(forFile(MICROPHONE_FILES[0]).entitlements).toContain(AUDIO_INPUT);
  });
});

describe('the release checks read back the same values', () => {
  // `cmd_verify` and `smoke_mac` open the built app. They are the only checks
  // that see a signed build, so they must test for the same sentence and the
  // same entitlement this config sets.
  for (const script of ['scripts/release.sh', 'scripts/smoke-test-release-artifacts.sh']) {
    it(`${script} checks the usage sentence and the entitlement`, () => {
      const text = readFileSync(resolve(repo, script), 'utf8');
      for (const needle of [
        `MAC_MICROPHONE_USAGE="${USAGE}"`,
        AUDIO_INPUT,
        'codesign -d --entitlements - --xml',
        'plutil -extract NSMicrophoneUsageDescription raw',
        '"$app/Contents/Frameworks/Biorouter Helper.app"',
        '"$app/Contents/Resources/bin/biorouter"',
        '"$app/Contents/Resources/bin/biorouterd"',
      ]) {
        expect(text.includes(needle), `${script} should contain ${needle}`).toBe(true);
      }
    });
  }
});
