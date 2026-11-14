/**
 * MSG-CPU-ROSETTA (S2 in `docs/dictation/surfaces.md`): the Intel build of the
 * desktop app running under Rosetta on a macOS before 15, where Rosetta cannot
 * translate the AVX2 the worker needs, so the daemon reports `unsupported_cpu`
 * for a reason a person can fix by installing the Apple Silicon build.
 *
 * The main process knows both facts (`app.runningUnderARM64Translation` and
 * `process.getSystemVersion()`); it computes the answer once with
 * {@link rosettaBeforeMacOS15} and hands it to each chat window under
 * {@link ROSETTA_BEFORE_MACOS15_KEY} in the window's start up configuration
 * (the JSON in `additionalArguments`, which preload serves through
 * `window.appConfig.get`, so preload needs no method of its own), and
 * `dictationStore.ts` reads it when it chooses the sentence. Free of Electron,
 * so the main process and the renderer share it. `rosetta.test.ts` pins each hop.
 */

/** The key `main.ts` sets in each window's configuration. */
export const ROSETTA_BEFORE_MACOS15_KEY = 'BIOROUTER_DICTATION_ROSETTA_BEFORE_MACOS15';

/**
 * Whether this is the Intel build translated by Rosetta on macOS 14 or earlier.
 * An unreadable version counts as 15 or later, so the ordinary processor sentence
 * shows rather than one that tells the person to reinstall for no reason.
 */
export function rosettaBeforeMacOS15(
  platform: string,
  runningUnderTranslation: boolean,
  systemVersion: string
): boolean {
  if (platform !== 'darwin' || !runningUnderTranslation) return false;
  const major = Number.parseInt(systemVersion.split('.')[0] ?? '', 10);
  return Number.isFinite(major) && major > 0 && major < 15;
}
