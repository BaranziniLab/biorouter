/**
 * The last mile of a macOS update: from "electron-updater has the zip" to
 * "Biorouter has quit and ShipIt is installing it".
 *
 * ⚠ electron-updater says `update-downloaded` BEFORE the update can be
 * installed. That event fires when the zip is on disk; Squirrel.Mac then has to
 * fetch it from electron-updater's local proxy, unpack it, check its code
 * signature and hand it to ShipIt, and only after its OWN `update-downloaded`
 * can `quitAndInstall` relaunch the app. Biorouter showed "Restart & update" at
 * the first event, so:
 *
 *   - a click while Squirrel was still working parked a listener and did
 *     nothing visible, and a person who then quit by hand got the update
 *     installed with no relaunch (ShipIt's request says
 *     `launchAfterInstallation: false` until `quitAndInstall` rewrites it);
 *   - a Squirrel failure (a read-only or translocated app, a bad signature)
 *     arrived as an `error` after the download, which the UI deliberately
 *     ignores once an update is "ready", so the button stayed and did nothing.
 *
 * This tracks Squirrel's own stage. The UI is told `update-preparing` while
 * Squirrel works, `update-downloaded` only once it is done, and
 * `install-failed` for anything that stops the install, so the person always
 * sees either progress or a reason. Pure apart from the injected dependencies.
 */

export type MacInstallStage = 'idle' | 'staging' | 'staged' | 'installing';

/** An opaque timer handle: whatever `setTimer` returns, `clearTimer` takes. */
type Timer = unknown;

export interface MacInstallDeps {
  /** electron-updater's `quitAndInstall`. */
  quitAndInstall: () => void;
  /** Send an updater event to every window and the persisted snapshot. */
  emit: (event: string, data?: unknown) => void;
  /**
   * Make sure a window exists. `quitAndInstall` closes every window before it
   * quits, so when the quit then stalls there is nowhere to show the failure,
   * and on macOS a tray app with no windows looks exactly like one that quit.
   */
  ensureWindow: () => void;
  log: (message: string) => void;
  setTimer?: (callback: () => void, ms: number) => Timer;
  clearTimer?: (timer: Timer) => void;
  /** How long Squirrel.Mac may take to stage an update. */
  stagingTimeoutMs?: number;
  /** How long the app may take to quit once the install starts. */
  quitTimeoutMs?: number;
}

export const STAGING_TIMEOUT_MS = 5 * 60_000;
export const QUIT_TIMEOUT_MS = 30_000;

export class MacInstallCoordinator {
  private stage: MacInstallStage = 'idle';
  private version: string | null = null;
  private installWhenStaged = false;
  private timer: Timer | null = null;

  constructor(private readonly deps: MacInstallDeps) {}

  get currentStage(): MacInstallStage {
    return this.stage;
  }

  /** The version being staged, ready, or installing; null when there is none. */
  get pendingVersion(): string | null {
    return this.stage === 'idle' ? null : this.version;
  }

  /** An error that arrives now belongs to Squirrel.Mac's staging or the install. */
  get ownsErrors(): boolean {
    return this.stage === 'staging' || this.stage === 'installing';
  }

  /** electron-updater has the zip; Squirrel.Mac starts staging it. */
  downloaded(version: string): void {
    if (this.version === version && this.stage !== 'idle') return;
    this.clearTimer();
    this.version = version;
    this.stage = 'staging';
    this.deps.log(`Update ${version} downloaded; waiting for Squirrel.Mac to stage it`);
    this.deps.emit('update-preparing', { version });
    this.timer = this.setTimer(
      () =>
        this.fail(
          `macOS did not finish preparing Biorouter ${version} for installation. Try again, ` +
            'or download the new version from the Biorouter website.'
        ),
      this.deps.stagingTimeoutMs ?? STAGING_TIMEOUT_MS
    );
  }

  /**
   * Squirrel.Mac's own `update-downloaded`: ShipIt holds the update. True when
   * this made an update ready and is not already installing it, which is when
   * the person should be told.
   */
  staged(): boolean {
    if (this.version === null) {
      this.deps.log('Squirrel.Mac staged an update this app did not download; ignoring it');
      return false;
    }
    // Accepted from `idle` too: a staging that outlived its timeout still worked.
    if (this.stage === 'staged' || this.stage === 'installing') return false;
    this.clearTimer();
    this.stage = 'staged';
    this.deps.log(`Update ${this.version} is staged and ready to install`);
    this.deps.emit('update-downloaded', { version: this.version });
    if (this.installWhenStaged) {
      this.installWhenStaged = false;
      this.install();
      return false;
    }
    return true;
  }

  /** Something stopped the staging or the install. */
  fail(message: string): void {
    const wasInstalling = this.stage === 'installing';
    this.clearTimer();
    this.stage = 'idle';
    this.installWhenStaged = false;
    this.deps.log(`Update could not be installed: ${message}`);
    this.deps.emit('install-failed', { version: this.version, message });
    if (wasInstalling) this.deps.ensureWindow();
  }

  /** "Restart & update". */
  requestInstall(): { ok: boolean; error?: string } {
    switch (this.stage) {
      case 'staged':
        this.install();
        return { ok: true };
      case 'staging':
        // Restart the moment Squirrel.Mac is done, and say so now.
        this.installWhenStaged = true;
        this.deps.emit('update-installing', { version: this.version });
        return { ok: true };
      case 'installing':
        return { ok: true };
      case 'idle':
        return { ok: false, error: 'No update is ready to install yet.' };
    }
  }

  private install(): void {
    this.clearTimer();
    this.stage = 'installing';
    this.deps.emit('update-installing', { version: this.version });
    try {
      this.deps.quitAndInstall();
    } catch (error) {
      this.fail(error instanceof Error ? error.message : String(error));
      return;
    }
    // Reaching this timer at all means the app is still running.
    this.timer = this.setTimer(
      () =>
        this.fail(
          'Biorouter could not close to install the update. Quit Biorouter and open it ' +
            'again: the update installs when it quits.'
        ),
      this.deps.quitTimeoutMs ?? QUIT_TIMEOUT_MS
    );
  }

  private setTimer(callback: () => void, ms: number): Timer {
    return this.deps.setTimer ? this.deps.setTimer(callback, ms) : setTimeout(callback, ms);
  }

  private clearTimer(): void {
    if (this.timer === null) return;
    if (this.deps.clearTimer) this.deps.clearTimer(this.timer);
    else clearTimeout(this.timer as ReturnType<typeof setTimeout>);
    this.timer = null;
  }
}
