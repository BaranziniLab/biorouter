import type { WebContents } from 'electron';
import log from 'electron-log';

/**
 * The in-process CDP client of one browser tab (BB-11): `webContents.debugger`, attached when the
 * agent first drives the tab and detached when the tab closes. No remote debugging port is ever
 * opened for this, so no other local process can drive the browser.
 *
 * Out-of-process frames (a cross-site iframe, such as a consent dialog) are reached through
 * `Target.setAutoAttach({ flatten: true })`: each gets a session id that `sendCommand` takes as
 * its third argument.
 */

export type CdpEvent = { method: string; params: Record<string, unknown>; sessionId?: string };

export class CdpError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CdpError';
  }
}

export class TabCdp {
  private attached = false;
  private listeners = new Set<(event: CdpEvent) => void>();
  /** Child frame id (an out-of-process frame's target id) -> its flattened session. */
  readonly frameSessions = new Map<string, string>();
  /** Session id -> the frame id it drives. */
  readonly sessionFrames = new Map<string, string>();
  private detachedReason: string | null = null;

  constructor(private readonly contents: WebContents) {}

  get isAttached(): boolean {
    return this.attached && !this.contents.isDestroyed();
  }

  /** Why the debugger went away, if it did (DevTools opened, the page crashed). */
  get detachReason(): string | null {
    return this.detachedReason;
  }

  async attach(): Promise<void> {
    if (this.isAttached) return;
    const dbg = this.contents.debugger;
    try {
      if (!dbg.isAttached()) dbg.attach('1.3');
    } catch (error) {
      throw new CdpError(
        `The browser could not take control of this tab (${String(error)}). If its developer tools are open, close them and try again.`
      );
    }
    this.attached = true;
    this.detachedReason = null;
    dbg.on('message', this.onMessage);
    dbg.on('detach', this.onDetach);
    await this.send('Page.enable');
    await this.send('DOM.enable');
    await this.send('Runtime.enable');
    await this.send('Accessibility.enable').catch(() => undefined);
    // While the agent drives, the page behaves as if focused, so typing works without taking the
    // operating system's focus from the composer or another app (BB-13).
    await this.send('Emulation.setFocusEmulationEnabled', { enabled: true }).catch(() => undefined);
    await this.send('Target.setAutoAttach', {
      autoAttach: true,
      waitForDebuggerOnStart: false,
      flatten: true,
    }).catch((error) => log.info('[BiorouterBrowser] no frame auto-attach:', String(error)));
  }

  detach(): void {
    if (!this.attached) return;
    this.attached = false;
    const dbg = this.contents.debugger;
    dbg.removeListener('message', this.onMessage);
    dbg.removeListener('detach', this.onDetach);
    try {
      if (!this.contents.isDestroyed() && dbg.isAttached()) dbg.detach();
    } catch {
      // Already gone with its page.
    }
    this.frameSessions.clear();
    this.sessionFrames.clear();
  }

  on(listener: (event: CdpEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async send<T = Record<string, unknown>>(
    method: string,
    params: Record<string, unknown> = {},
    sessionId?: string
  ): Promise<T> {
    if (!this.isAttached) {
      throw new CdpError(this.detachedReason ?? 'The browser tab is no longer controlled.');
    }
    try {
      return (await (sessionId
        ? this.contents.debugger.sendCommand(method, params, sessionId)
        : this.contents.debugger.sendCommand(method, params))) as T;
    } catch (error) {
      throw new CdpError(error instanceof Error ? error.message : String(error));
    }
  }

  private onMessage = (
    _event: unknown,
    method: string,
    params: Record<string, unknown>,
    sessionId?: string
  ) => {
    if (method === 'Target.attachedToTarget') {
      const child = params.sessionId as string;
      const info = params.targetInfo as { targetId?: string; type?: string } | undefined;
      if (child && info?.targetId && info.type === 'iframe') {
        this.frameSessions.set(info.targetId, child);
        this.sessionFrames.set(child, info.targetId);
        void this.prepareChild(child);
      } else if (child && info?.type !== 'iframe') {
        // Workers and the like: let them run, never drive them.
        void this.send('Runtime.runIfWaitingForDebugger', {}, child).catch(() => undefined);
      }
    } else if (method === 'Target.detachedFromTarget') {
      const child = params.sessionId as string;
      const frame = this.sessionFrames.get(child);
      if (frame) this.frameSessions.delete(frame);
      this.sessionFrames.delete(child);
    }
    for (const listener of this.listeners) {
      try {
        listener({ method, params, sessionId: sessionId || undefined });
      } catch (error) {
        log.warn('[BiorouterBrowser] CDP listener failed:', String(error));
      }
    }
  };

  private async prepareChild(sessionId: string): Promise<void> {
    await Promise.all([
      this.send('Page.enable', {}, sessionId).catch(() => undefined),
      this.send('DOM.enable', {}, sessionId).catch(() => undefined),
      this.send('Runtime.enable', {}, sessionId).catch(() => undefined),
      this.send('Accessibility.enable', {}, sessionId).catch(() => undefined),
      this.send(
        'Target.setAutoAttach',
        { autoAttach: true, waitForDebuggerOnStart: false, flatten: true },
        sessionId
      ).catch(() => undefined),
    ]);
    await this.send('Runtime.runIfWaitingForDebugger', {}, sessionId).catch(() => undefined);
  }

  private onDetach = (_event: unknown, reason: string) => {
    this.attached = false;
    this.detachedReason =
      reason === 'replaced_with_devtools'
        ? 'Developer tools were opened on this tab, which takes it away from the agent. Close them and try again.'
        : `The browser lost control of this tab (${reason}).`;
    this.frameSessions.clear();
    this.sessionFrames.clear();
  };
}
