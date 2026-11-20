import type { BrowserWindow, WebContents } from 'electron';
import log from 'electron-log';
import {
  agentBrowserTab,
  agentBrowserTabError,
  agentBrowserTabs,
  beginAgentInput,
  closeAgentBrowserTab,
  controlAgentBrowserTab,
  createAgentBrowserTab,
  endAgentInput,
  navigateAgentBrowserTab,
  reportAgentActivity,
  setAgentDownloadRoute,
  showAgentBrowserTab,
  type AgentDownloadReport,
  type AgentTabInfo,
} from '../../utils/embeddedBrowser';
import {
  CONTAINS_FUNCTION,
  DESCRIBE_COVER_FUNCTION,
  ELEMENT_FACTS_FUNCTION,
  PROTECTED_FIELD_REFUSAL,
  SET_VALUE_INPUT_TYPES,
  STABLE_BOX_FUNCTION,
  UNFILLABLE_INPUT_TYPES,
  boxOfQuads,
  checkElement,
  clickPoint,
  isPlainLinkClick,
  isProtectedInput,
  isStable,
  type Box,
  type ElementFacts,
} from './actionability';
import { TabCdp, type CdpEvent } from './cdp';
import { parseChord, strokeForCharacter, type KeyStroke } from './keys';
import { READ_AS_MARKDOWN_FUNCTION } from './readPage';
import { RefStore, fallbackMatch, type RefTarget } from './refs';
import { buildSnapshot, type AXNode, type DomFacts, type SnapshotFrame } from './snapshot';
import {
  BROWSER_AGENT_OPS,
  type BrowserAgentRequest,
  type BrowserAgentResult,
  type BrowserAgentTabInfo,
} from './types';

/**
 * The Biorouter Browser's agent driver (BB-11 to BB-15): what main does with one `browser`
 * operation the daemon sent through the chat's window.
 *
 * - **Only the chat's own agent tabs.** A tab the person opened has no `agentSession` and is
 *   never found here.
 * - **Only a visible tab.** Before every operation the tab is shown in the chat's panel; if the
 *   panel cannot show it within 2.5 s (a menu or a modal is over it, the panel is closed) the
 *   operation fails with a reason the model can retry on (BB-15).
 * - **One operation at a time per tab**, in a queue; the person's own input is never blocked.
 * - **Checked actions** (BB-13): references resolve through the stored `backendNodeId`, the
 *   target is scrolled into view, must be visible, enabled, stable and not covered, and is
 *   flashed with the inspector overlay before the input is dispatched.
 * - **Never the operating system focus.** Input goes through CDP into this web contents only;
 *   no operation focuses the view, moves the system cursor or raises the window.
 */

export type DriverDeps = {
  /** Write a PNG where the daemon will read it (the app's pasted image temp folder). */
  writeScreenshot: (png: Buffer) => Promise<string>;
  /** Make a PNG smaller (Electron's nativeImage in main). */
  resizePng?: (png: Buffer, maxWidth: number, maxHeight: number) => Buffer;
};

type PendingDialog = { type: string; message: string; defaultPrompt: string; sessionId?: string };

type TabState = {
  cdp: TabCdp;
  refs: RefStore;
  dialog: PendingDialog | null;
  dialogWaiters: Set<() => void>;
  mainNavigations: number;
  queue: Promise<unknown>;
  download: AgentDownloadReport | null;
  stopListening: () => void;
};

const tabStates = new WeakMap<WebContents, TabState>();
/** `${window id}:${session}` -> the view id of the chat's current agent tab. */
const currentTabs = new Map<string, string>();
const inFlight = new Map<string, { controller: AbortController; contents: WebContents | null }>();

const SHOW_WAIT_MS = 2_500;
const HIGHLIGHT_MS = 300;
const SETTLE_START_MS = 500;
const SNAPSHOT_BYTES_MAX = 96 * 1024;
const VIEWPORT_LONG_SIDE = 1_568;
const FULL_PAGE_MAX_CSS_HEIGHT = 8_000;
const ISOLATED_WORLD = 'biorouter-agent';

class OpFailure extends Error {
  constructor(
    message: string,
    readonly extra: BrowserAgentResult['data'] = {}
  ) {
    super(message);
  }
}

function sessionKey(window: BrowserWindow, sessionId: string): string {
  return `${window.webContents.id}:${sessionId}`;
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(abortError(signal));
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(abortError(signal));
    };
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

function abortError(signal: AbortSignal): Error {
  const reason = signal.reason;
  return reason instanceof Error ? reason : new Error(String(reason ?? 'cancelled'));
}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(abortError(signal));
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(abortError(signal));
    signal.addEventListener('abort', onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener('abort', onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener('abort', onAbort);
        reject(error);
      }
    );
  });
}

function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** Cut text to `maxChars` characters and to what the socket's 128 KiB frame cap leaves room for. */
function fitText(text: string, maxChars: number): { text: string; cut: boolean } {
  let out = text.length > maxChars ? text.slice(0, maxChars) : text;
  let cut = out.length < text.length;
  while (Buffer.byteLength(out, 'utf8') > SNAPSHOT_BYTES_MAX) {
    out = out.slice(0, Math.floor(out.length * 0.9));
    cut = true;
  }
  return { text: out, cut };
}

function stateFor(contents: WebContents): TabState {
  let state = tabStates.get(contents);
  if (state) return state;
  const cdp = new TabCdp(contents);
  const refs = new RefStore();
  const created: TabState = {
    cdp,
    refs,
    dialog: null,
    dialogWaiters: new Set(),
    mainNavigations: 0,
    queue: Promise.resolve(),
    download: null,
    stopListening: () => undefined,
  };
  created.stopListening = cdp.on((event: CdpEvent) => onCdpEvent(created, contents, event));
  contents.once('destroyed', () => {
    created.stopListening();
    cdp.detach();
    refs.invalidateAll('', 'discarded');
  });
  tabStates.set(contents, created);
  state = created;
  return state;
}

function onCdpEvent(state: TabState, contents: WebContents, event: CdpEvent): void {
  if (event.method === 'Page.frameNavigated') {
    const frame = event.params.frame as { id: string; parentId?: string; url?: string } | undefined;
    if (!frame) return;
    const url = contents.isDestroyed() ? '' : contents.getURL();
    if (!frame.parentId && !event.sessionId) {
      state.mainNavigations += 1;
      state.refs.invalidateAll(url);
    } else {
      state.refs.invalidateFrame(frame.id, url);
    }
  } else if (event.method === 'Page.javascriptDialogOpening') {
    state.dialog = {
      type: String(event.params.type ?? 'alert'),
      message: String(event.params.message ?? '').slice(0, 2_000),
      defaultPrompt: String(event.params.defaultPrompt ?? '').slice(0, 2_000),
      sessionId: event.sessionId,
    };
    for (const wake of state.dialogWaiters) wake();
  } else if (event.method === 'Page.javascriptDialogClosed') {
    state.dialog = null;
  }
}

/** Tests only: forget every chat's current tab. */
export function resetAgentDriverForTests(): void {
  currentTabs.clear();
  inFlight.clear();
}

/** Stop an operation the daemon no longer waits for (a cancel or its own timeout). */
export function cancelAgentBrowserOperation(requestId: string): boolean {
  const running = inFlight.get(requestId);
  if (!running) return false;
  running.controller.abort(new Error('cancelled'));
  if (running.contents && !running.contents.isDestroyed() && running.contents.isLoading()) {
    running.contents.stop();
  }
  return true;
}

async function waitShown(
  window: BrowserWindow,
  viewId: string,
  signal: AbortSignal
): Promise<AgentTabInfo> {
  let info = agentBrowserTab(window, viewId);
  if (!info) throw new OpFailure('The browser tab was closed. Use browser_open to open a page.');
  if (info.visible) return info;
  showAgentBrowserTab(window, viewId);
  const until = Date.now() + SHOW_WAIT_MS;
  while (Date.now() < until) {
    await delay(50, signal);
    info = agentBrowserTab(window, viewId);
    if (!info) throw new OpFailure('The browser tab was closed. Use browser_open to open a page.');
    if (info.visible) return info;
  }
  throw new OpFailure(
    "The browser panel is not visible right now (it is closed, or a menu or dialog covers it). Try again in a moment; if it keeps failing, ask the person to show the chat's side panel.",
    { error_kind: 'not_visible' }
  );
}

/**
 * The main document's ready state through CDP. Not `executeJavaScript`, which waits for a load
 * in progress to finish, exactly when this is asked.
 */
async function readyState(state: TabState): Promise<string> {
  const result = await state.cdp
    .send<{ result: { value?: string } }>('Runtime.evaluate', {
      expression: 'document.readyState',
      returnByValue: true,
    })
    .catch(() => ({ result: { value: 'loading' } }));
  return result.result.value ?? 'loading';
}

/** Wait for a main frame load to start (up to `startMs`) and then to settle. */
async function settleNavigation(
  contents: WebContents,
  state: TabState,
  before: number,
  startMs: number,
  untilMs: number,
  signal: AbortSignal
): Promise<boolean> {
  const startBy = Date.now() + startMs;
  while (Date.now() < startBy && state.mainNavigations === before && !contents.isLoading()) {
    await delay(50, signal);
  }
  if (state.mainNavigations === before && !contents.isLoading()) return false;
  while (Date.now() < untilMs && contents.isLoading() && !state.dialog) {
    // DOMContentLoaded is enough: a page whose last image never arrives is still usable.
    const ready = await readyState(state);
    if (ready !== 'loading' && state.mainNavigations !== before) break;
    await delay(100, signal);
  }
  return state.mainNavigations !== before;
}

type Context = {
  window: BrowserWindow;
  request: BrowserAgentRequest;
  contents: WebContents;
  info: AgentTabInfo;
  state: TabState;
  signal: AbortSignal;
  deadline: number;
  /** When the operation started: input the person made after it is reported. */
  startedAt: number;
  deps: DriverDeps;
  /** Isolated worlds made during this operation, per frame key. */
  worlds: Map<string, number>;
};

type Resolved = {
  target: RefTarget;
  sessionId: string | undefined;
  objectId: string;
  /** Offset of the frame's viewport inside the main viewport (out-of-process frames only). */
  offset: { x: number; y: number };
  label: string;
};

function describeTarget(target: RefTarget): string {
  return target.name ? `[${target.role}] "${target.name}"` : `[${target.role}] ${target.ref}`;
}

async function mainFrameId(ctx: Context, sessionId?: string): Promise<string> {
  const tree = await ctx.state.cdp.send<{ frameTree: { frame: { id: string } } }>(
    'Page.getFrameTree',
    {},
    sessionId
  );
  return tree.frameTree.frame.id;
}

/** An isolated world in the frame: a page cannot replace the DOM methods run there. */
async function worldFor(ctx: Context, frameKey: string): Promise<number> {
  const cached = ctx.worlds.get(frameKey);
  if (cached !== undefined) return cached;
  const sessionId = sessionForFrame(ctx, frameKey);
  const frameId = frameKey === 'main' || sessionId ? await mainFrameId(ctx, sessionId) : frameKey;
  const world = await ctx.state.cdp.send<{ executionContextId: number }>(
    'Page.createIsolatedWorld',
    { frameId, worldName: ISOLATED_WORLD, grantUniveralAccess: false },
    sessionId
  );
  ctx.worlds.set(frameKey, world.executionContextId);
  return world.executionContextId;
}

function sessionForFrame(ctx: Context, frameKey: string): string | undefined {
  if (frameKey === 'main') return undefined;
  return ctx.state.cdp.frameSessions.get(frameKey);
}

async function frameOffset(ctx: Context, frameKey: string): Promise<{ x: number; y: number }> {
  const sessionId = sessionForFrame(ctx, frameKey);
  if (!sessionId) return { x: 0, y: 0 };
  const owner = await ctx.state.cdp.send<{ backendNodeId: number }>('DOM.getFrameOwner', {
    frameId: frameKey,
  });
  const box = await ctx.state.cdp.send<{ model: { content: number[] } }>('DOM.getBoxModel', {
    backendNodeId: owner.backendNodeId,
  });
  return { x: box.model.content[0], y: box.model.content[1] };
}

async function callOn<T>(
  ctx: Context,
  objectId: string,
  sessionId: string | undefined,
  functionDeclaration: string,
  args: unknown[] = [],
  awaitPromise = false
): Promise<T> {
  const result = await ctx.state.cdp.send<{
    result: { value?: T };
    exceptionDetails?: { text?: string };
  }>(
    'Runtime.callFunctionOn',
    {
      functionDeclaration,
      objectId,
      arguments: args.map((value) =>
        typeof value === 'object' && value && 'objectId' in (value as object) ? value : { value }
      ),
      returnByValue: true,
      awaitPromise,
    },
    sessionId
  );
  if (result.exceptionDetails) {
    throw new OpFailure(
      `The page refused the action (${result.exceptionDetails.text ?? 'error'}).`
    );
  }
  return result.result.value as T;
}

async function resolveNodeObject(
  ctx: Context,
  frameKey: string,
  backendNodeId: number
): Promise<string | null> {
  const sessionId = sessionForFrame(ctx, frameKey);
  const executionContextId = await worldFor(ctx, frameKey);
  try {
    const resolved = await ctx.state.cdp.send<{ object: { objectId?: string } }>(
      'DOM.resolveNode',
      { backendNodeId, executionContextId },
      sessionId
    );
    return resolved.object.objectId ?? null;
  } catch {
    return null;
  }
}

/** Resolve a reference (BB-12), with the unique role and name fallback. */
async function resolveRef(ctx: Context, ref: unknown): Promise<Resolved> {
  if (typeof ref !== 'string') throw new OpFailure('Pass an @e reference from browser_snapshot.');
  const url = ctx.contents.getURL();
  const target = ctx.state.refs.get(ref);
  if (!target)
    throw new OpFailure(ctx.state.refs.staleReason(ref, url), { refs_invalidated: true });
  const sessionId = sessionForFrame(ctx, target.frameKey);
  if (target.frameKey !== 'main' && !sessionId) {
    // A same-process child frame: its nodes are reached through the main session.
  }
  let objectId = await resolveNodeObject(ctx, target.frameKey, target.backendNodeId);
  let connected = false;
  if (objectId) {
    connected = await callOn<boolean>(
      ctx,
      objectId,
      sessionId,
      'function () { return this.isConnected === true; }'
    ).catch(() => false);
  }
  if (!objectId || !connected) {
    const tree = await ctx.state.cdp
      .send<{
        nodes: AXNode[];
      }>(
        'Accessibility.getFullAXTree',
        target.frameKey === 'main' || sessionId ? {} : { frameId: target.frameKey },
        sessionId
      )
      .catch(() => ({ nodes: [] as AXNode[] }));
    const candidates = tree.nodes
      .filter((node) => !node.ignored && node.backendDOMNodeId !== undefined)
      .map((node) => ({
        backendNodeId: node.backendDOMNodeId as number,
        role: String(node.role?.value ?? ''),
        name: String(node.name?.value ?? '')
          .replace(/\s+/g, ' ')
          .trim(),
      }));
    const match = fallbackMatch(target, candidates);
    if (!match.ok) {
      ctx.state.refs.forget(target.ref, url);
      throw new OpFailure(match.reason, { refs_invalidated: true });
    }
    ctx.state.refs.assign({ ...target, backendNodeId: match.backendNodeId });
    objectId = await resolveNodeObject(ctx, target.frameKey, match.backendNodeId);
    if (!objectId)
      throw new OpFailure(`${target.ref} no longer exists. Take a new browser_snapshot.`);
    target.backendNodeId = match.backendNodeId;
  }
  const offset = await frameOffset(ctx, target.frameKey);
  return { target, sessionId, objectId, offset, label: describeTarget(target) };
}

async function factsOf(ctx: Context, node: Resolved): Promise<ElementFacts> {
  return callOn<ElementFacts>(ctx, node.objectId, node.sessionId, ELEMENT_FACTS_FUNCTION);
}

async function viewportOf(
  ctx: Context,
  sessionId?: string
): Promise<{ width: number; height: number; scrollX: number; scrollY: number }> {
  const metrics = await ctx.state.cdp.send<{
    cssLayoutViewport?: { clientWidth: number; clientHeight: number; pageX: number; pageY: number };
    layoutViewport?: { clientWidth: number; clientHeight: number; pageX: number; pageY: number };
  }>('Page.getLayoutMetrics', {}, sessionId);
  const viewport = metrics.cssLayoutViewport ?? metrics.layoutViewport;
  return {
    width: viewport?.clientWidth ?? 0,
    height: viewport?.clientHeight ?? 0,
    scrollX: viewport?.pageX ?? 0,
    scrollY: viewport?.pageY ?? 0,
  };
}

/** BB-13 steps 4 to 6: scroll into view, visible and enabled, stable, not covered. */
async function actionablePoint(
  ctx: Context,
  node: Resolved,
  facts: ElementFacts,
  needsEnabled: boolean
): Promise<{ x: number; y: number }> {
  await ctx.state.cdp
    .send(
      'DOM.scrollIntoViewIfNeeded',
      { backendNodeId: node.target.backendNodeId },
      node.sessionId
    )
    .catch(() => undefined);
  let quads: number[][] = [];
  try {
    quads = (
      await ctx.state.cdp.send<{ quads: number[][] }>(
        'DOM.getContentQuads',
        { backendNodeId: node.target.backendNodeId },
        node.sessionId
      )
    ).quads;
  } catch {
    quads = [];
  }
  const check = checkElement(
    facts,
    boxOfQuads(quads),
    needsEnabled,
    `${node.target.ref} ${node.label}`
  );
  if (!check.ok) throw new OpFailure(check.reason);
  for (let attempt = 0; ; attempt += 1) {
    const boxes = await callOn<{ first: Box; second: Box } | null>(
      ctx,
      node.objectId,
      node.sessionId,
      STABLE_BOX_FUNCTION,
      [],
      true
    );
    if (boxes && isStable(boxes.first, boxes.second)) break;
    if (attempt >= 3) {
      throw new OpFailure(
        `${node.target.ref} ${node.label} is still moving (an animation?). Wait and try again.`
      );
    }
    await delay(120, ctx.signal);
    quads = (
      await ctx.state.cdp.send<{ quads: number[][] }>(
        'DOM.getContentQuads',
        { backendNodeId: node.target.backendNodeId },
        node.sessionId
      )
    ).quads;
  }
  const viewport = await viewportOf(ctx, node.sessionId);
  const point = clickPoint(quads, viewport);
  if (!point) {
    throw new OpFailure(
      `${node.target.ref} ${node.label} is outside the visible page and could not be scrolled into view.`
    );
  }
  // Hit test at the point, in the element's own frame. `DOM.getNodeForLocation` takes document
  // coordinates, so the frame's scroll offset is added to the viewport point the quads give.
  const hit = await ctx.state.cdp
    .send<{ backendNodeId: number }>(
      'DOM.getNodeForLocation',
      {
        x: Math.round(point.x + viewport.scrollX),
        y: Math.round(point.y + viewport.scrollY),
        includeUserAgentShadowDOM: false,
        ignorePointerEventsNone: true,
      },
      node.sessionId
    )
    .catch(() => null);
  if (hit) {
    const hitObject = await resolveNodeObject(ctx, node.target.frameKey, hit.backendNodeId);
    if (hitObject) {
      const inside = await callOn<boolean>(ctx, node.objectId, node.sessionId, CONTAINS_FUNCTION, [
        { objectId: hitObject },
      ]).catch(() => true);
      if (!inside) {
        const cover = await callOn<string>(
          ctx,
          hitObject,
          node.sessionId,
          DESCRIBE_COVER_FUNCTION
        ).catch(() => 'another element');
        throw new OpFailure(
          `${node.target.ref} ${node.label} is covered by ${cover}. Close or dismiss it first.`
        );
      }
    }
  }
  return { x: point.x + node.offset.x, y: point.y + node.offset.y };
}

/** BB-13 step 7: flash the target, drawn by the inspector overlay without touching the DOM. */
async function highlight(ctx: Context, node: Resolved): Promise<() => Promise<void>> {
  try {
    await ctx.state.cdp.send('Overlay.enable', {}, node.sessionId);
    await ctx.state.cdp.send(
      'Overlay.highlightNode',
      {
        backendNodeId: node.target.backendNodeId,
        highlightConfig: {
          contentColor: { r: 232, g: 112, b: 80, a: 0.22 },
          borderColor: { r: 232, g: 112, b: 80, a: 0.95 },
        },
      },
      node.sessionId
    );
    await delay(HIGHLIGHT_MS, ctx.signal);
  } catch (error) {
    if (ctx.signal.aborted) throw error;
  }
  return async () => {
    await ctx.state.cdp.send('Overlay.hideHighlight', {}, node.sessionId).catch(() => undefined);
  };
}

async function dispatchMouse(
  ctx: Context,
  type: 'mouseMoved' | 'mousePressed' | 'mouseReleased' | 'mouseWheel',
  point: { x: number; y: number },
  extra: Record<string, unknown> = {}
): Promise<void> {
  await ctx.state.cdp.send('Input.dispatchMouseEvent', {
    type,
    x: point.x,
    y: point.y,
    button: type === 'mouseMoved' || type === 'mouseWheel' ? 'none' : 'left',
    clickCount: type === 'mousePressed' || type === 'mouseReleased' ? 1 : 0,
    ...extra,
  });
}

/** Wait for `work`, or for a dialog the work opened (a page blocks in `alert()`). */
async function untilDialog(ctx: Context, work: Promise<unknown>): Promise<void> {
  let wake: () => void = () => undefined;
  const opened = new Promise<void>((resolve) => {
    wake = resolve;
  });
  ctx.state.dialogWaiters.add(wake);
  try {
    await raceAbort(Promise.race([work.then(() => undefined), opened]), ctx.signal);
  } finally {
    ctx.state.dialogWaiters.delete(wake);
  }
}

async function pressStroke(ctx: Context, stroke: KeyStroke): Promise<void> {
  const base = {
    key: stroke.key,
    code: stroke.code,
    windowsVirtualKeyCode: stroke.keyCode,
    nativeVirtualKeyCode: stroke.keyCode,
    modifiers: stroke.modifiers,
  };
  await ctx.state.cdp.send('Input.dispatchKeyEvent', {
    ...base,
    type: stroke.text ? 'keyDown' : 'rawKeyDown',
    text: stroke.text,
    unmodifiedText: stroke.text,
    ...(stroke.commands ? { commands: stroke.commands } : {}),
  });
  await ctx.state.cdp.send('Input.dispatchKeyEvent', { ...base, type: 'keyUp' });
}

function withDownloads(ctx: Context): () => void {
  const { window, request, info, state } = ctx;
  state.download = null;
  setAgentDownloadRoute(window, info.viewId, {
    dir: request.grant === 'act' && request.downloadDir ? request.downloadDir : null,
    onDone: (download) => {
      state.download = download;
    },
  });
  return () => setAgentDownloadRoute(window, info.viewId, null);
}

/** After an action: let a navigation start and settle, and a download finish (BB-05, BB-13). */
async function afterAction(ctx: Context, navigationsBefore: number): Promise<boolean> {
  const navigated = await settleNavigation(
    ctx.contents,
    ctx.state,
    navigationsBefore,
    SETTLE_START_MS,
    ctx.deadline - 1_000,
    ctx.signal
  ).catch((error) => {
    if (ctx.signal.aborted) throw error;
    return false;
  });
  // A download the action started: wait for it while the deadline allows.
  while (ctx.state.download?.state === 'progressing' && Date.now() < ctx.deadline - 1_000) {
    await delay(100, ctx.signal);
  }
  return navigated;
}

// --- Operations ---------------------------------------------------------------------------

async function opOpen(ctx: Context): Promise<BrowserAgentResult> {
  const url = String(ctx.request.args.url ?? '');
  const before = ctx.state.mainNavigations;
  const navigation = await navigateAgentBrowserTab(
    ctx.window,
    ctx.info.viewId,
    url,
    ctx.request.tier
  );
  if (!navigation.ok) throw new OpFailure(navigation.reason);
  await settleNavigation(ctx.contents, ctx.state, before, 3_000, ctx.deadline - 1_000, ctx.signal);
  const error = agentBrowserTabError(ctx.window, ctx.info.viewId);
  if (error && ctx.contents.getURL() !== url && !ctx.contents.getURL().startsWith(url)) {
    throw new OpFailure(`${hostOf(url)} did not open: ${error}`);
  }
  return {
    ok: true,
    detail: `opened ${hostOf(ctx.contents.getURL() || url)}`,
    data: { navigated: true, refs_invalidated: true },
  };
}

async function opNavigate(ctx: Context): Promise<BrowserAgentResult> {
  const action = String(ctx.request.args.action) as 'back' | 'forward' | 'reload' | 'stop';
  const before = ctx.state.mainNavigations;
  if (!controlAgentBrowserTab(ctx.window, ctx.info.viewId, action)) {
    throw new OpFailure(
      action === 'back'
        ? 'There is no earlier page in this tab.'
        : action === 'forward'
          ? 'There is no later page in this tab.'
          : 'The tab could not do that.'
    );
  }
  if (action === 'stop') return { ok: true, detail: 'stopped loading' };
  const navigated = await settleNavigation(
    ctx.contents,
    ctx.state,
    before,
    2_000,
    ctx.deadline - 1_000,
    ctx.signal
  );
  return {
    ok: true,
    detail: `went ${action === 'reload' ? 'and reloaded' : action}`,
    data: { navigated, refs_invalidated: navigated },
  };
}

function tabInfos(window: BrowserWindow, sessionId: string): BrowserAgentTabInfo[] {
  const current = currentTabs.get(sessionKey(window, sessionId));
  return agentBrowserTabs(window, sessionId).map((tab) => ({
    tab: tab.label,
    title: tab.title,
    url: tab.url,
    active: tab.viewId === current,
    opened_by: 'agent',
  }));
}

/** Collect every frame's accessibility tree and DOM facts. */
async function collectFrames(ctx: Context): Promise<Map<string, SnapshotFrame>> {
  const frames = new Map<string, SnapshotFrame>();
  const cdp = ctx.state.cdp;
  const mainDom = await domFacts(ctx, undefined);
  const tree = await cdp.send<{ frameTree: FrameTreeNode }>('Page.getFrameTree');
  const mainId = tree.frameTree.frame.id;
  const visit = async (node: FrameTreeNode) => {
    const isMain = node.frame.id === mainId;
    const key = isMain ? 'main' : node.frame.id;
    if (!isMain && cdp.frameSessions.has(node.frame.id)) return;
    const ax = await cdp
      .send<{
        nodes: AXNode[];
      }>('Accessibility.getFullAXTree', isMain ? {} : { frameId: node.frame.id })
      .catch(() => null);
    if (ax) {
      frames.set(key, {
        frameKey: key,
        url: node.frame.url,
        title: isMain ? ctx.contents.getTitle() : (node.frame.name ?? ''),
        nodes: ax.nodes,
        dom: mainDom,
      });
    }
    for (const child of node.childFrames ?? []) await visit(child);
  };
  await visit(tree.frameTree);
  // Out-of-process frames (cross-site iframes) through their own sessions, one level deep.
  for (const [frameId, sessionId] of cdp.frameSessions) {
    const ax = await cdp
      .send<{ nodes: AXNode[] }>('Accessibility.getFullAXTree', {}, sessionId)
      .catch(() => null);
    if (!ax) continue;
    const childTree = await cdp
      .send<{ frameTree: FrameTreeNode }>('Page.getFrameTree', {}, sessionId)
      .catch(() => null);
    frames.set(frameId, {
      frameKey: frameId,
      url: childTree?.frameTree.frame.url ?? '',
      title: '',
      nodes: ax.nodes,
      dom: await domFacts(ctx, sessionId),
    });
  }
  return frames;
}

type FrameTreeNode = {
  frame: { id: string; url: string; name?: string };
  childFrames?: FrameTreeNode[];
};

type DomNode = {
  backendNodeId: number;
  nodeType: number;
  nodeName: string;
  localName?: string;
  nodeValue?: string;
  attributes?: string[];
  children?: DomNode[];
  shadowRoots?: DomNode[];
  contentDocument?: DomNode;
  frameId?: string;
};

const ICON_CLASS =
  /^(?:fa[srlbd]?|bi|icon|glyphicon|mdi|ti|ri|lucide|octicon|material-icons)[-_](.+)$/i;
const HINTED_TAGS = new Set(['a', 'button', 'summary', 'select', 'input']);

function attrs(node: DomNode): Record<string, string> {
  const out: Record<string, string> = {};
  const list = node.attributes ?? [];
  for (let i = 0; i + 1 < list.length; i += 2) out[list[i].toLowerCase()] = list[i + 1];
  return out;
}

/** Short hints from an element's subtree: an image's alt, an icon's name, a select's options. */
function subtreeHints(node: DomNode): Pick<DomFacts, 'imgAlt' | 'iconHint' | 'options'> {
  const out: Pick<DomFacts, 'imgAlt' | 'iconHint' | 'options'> = {};
  const options: string[] = [];
  const stack: Array<{ node: DomNode; depth: number }> = [{ node, depth: 0 }];
  while (stack.length > 0) {
    const { node: current, depth } = stack.pop()!;
    if (current.nodeType === 1) {
      const tag = (current.localName ?? current.nodeName).toLowerCase();
      const a = attrs(current);
      if (!out.imgAlt && tag === 'img' && a.alt) out.imgAlt = a.alt;
      if (!out.iconHint) {
        if (a['data-icon']) out.iconHint = a['data-icon'];
        else if ((tag === 'svg' || tag === 'i' || tag === 'span') && a['aria-label'])
          out.iconHint = a['aria-label'];
        else {
          for (const token of (a.class ?? '').split(/\s+/)) {
            const match = ICON_CLASS.exec(token);
            if (match && !/^(solid|regular|light|brands|icon|lg|sm|xs|fw|\dx)$/i.test(match[1])) {
              out.iconHint = match[1];
              break;
            }
          }
        }
        if (!out.iconHint && tag === 'title' && depth > 0) {
          const text = (current.children ?? [])
            .map((child) => child.nodeValue ?? '')
            .join('')
            .trim();
          if (text) out.iconHint = text;
        }
      }
      if (tag === 'option' && options.length < 60) {
        const text = (current.children ?? [])
          .map((child) => child.nodeValue ?? '')
          .join('')
          .trim();
        options.push(text || a.value || '');
      }
    }
    if (depth < 5) {
      // Reversed, so the stack visits children in document order (option order matters).
      const children = current.children ?? [];
      for (let i = children.length - 1; i >= 0; i -= 1)
        stack.push({ node: children[i], depth: depth + 1 });
    }
  }
  if (options.length > 0) out.options = options;
  return out;
}

async function domFacts(
  ctx: Context,
  sessionId: string | undefined
): Promise<Map<number, DomFacts>> {
  const facts = new Map<number, DomFacts>();
  const document = await ctx.state.cdp
    .send<{ root: DomNode }>('DOM.getDocument', { depth: -1, pierce: true }, sessionId)
    .catch(() => null);
  if (!document) return facts;
  const stack: DomNode[] = [document.root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.nodeType === 1) {
      const tag = (node.localName ?? node.nodeName).toLowerCase();
      const attributes = attrs(node);
      const entry: DomFacts = { tag, attributes };
      if (node.frameId) entry.frameId = node.frameId;
      if (HINTED_TAGS.has(tag) || attributes.role || attributes.tabindex !== undefined) {
        Object.assign(entry, subtreeHints(node));
      }
      facts.set(node.backendNodeId, entry);
    }
    for (const child of node.children ?? []) stack.push(child);
    for (const root of node.shadowRoots ?? []) stack.push(root);
    if (node.contentDocument) stack.push(node.contentDocument);
  }
  return facts;
}

async function opSnapshot(ctx: Context): Promise<BrowserAgentResult> {
  const args = ctx.request.args as {
    interactive_only?: boolean;
    max_depth?: number | null;
    scope?: { ref?: string; selector?: string } | null;
    include_urls?: boolean;
    max_chars?: number;
  };
  const frames = await collectFrames(ctx);
  let scope: { frameKey: string; backendNodeId: number } | undefined;
  if (args.scope?.ref) {
    const node = await resolveRef(ctx, args.scope.ref);
    scope = { frameKey: node.target.frameKey, backendNodeId: node.target.backendNodeId };
  } else if (args.scope?.selector) {
    const doc = await ctx.state.cdp.send<{ root: { nodeId: number } }>('DOM.getDocument', {
      depth: 0,
    });
    const found = await ctx.state.cdp
      .send<{
        nodeId: number;
      }>('DOM.querySelector', { nodeId: doc.root.nodeId, selector: args.scope.selector })
      .catch(() => ({ nodeId: 0 }));
    if (!found.nodeId)
      throw new OpFailure(`Nothing on the page matches ${JSON.stringify(args.scope.selector)}.`);
    const described = await ctx.state.cdp.send<{ node: { backendNodeId: number } }>(
      'DOM.describeNode',
      {
        nodeId: found.nodeId,
      }
    );
    scope = { frameKey: 'main', backendNodeId: described.node.backendNodeId };
  }
  const maxChars = Math.min(40_000, Math.max(1_000, Number(args.max_chars) || 12_000));
  const result = buildSnapshot(
    frames,
    {
      interactiveOnly: args.interactive_only !== false,
      maxDepth: typeof args.max_depth === 'number' ? args.max_depth : undefined,
      includeUrls: args.include_urls === true,
      maxChars,
      scope,
    },
    (target) => ctx.state.refs.assign(target)
  );
  if (result.scopeMissing)
    throw new OpFailure('That element is not in the accessibility tree (it may be hidden).');
  const title = ctx.contents.getTitle();
  const header = [
    `Page: ${title.replace(/\s+/g, ' ').slice(0, 300)}  ${ctx.contents.getURL()}`,
    `Tab ${ctx.info.label}, opened by Biorouter. References last until this page navigates.`,
  ].join('\n');
  const fitted = fitText(`${header}\n${result.text}`, maxChars + header.length + 1);
  return {
    ok: true,
    detail: `took a snapshot with ${result.refCount} references`,
    data: { snapshot: fitted.text, truncated: result.truncated || fitted.cut },
  };
}

async function opRead(ctx: Context): Promise<BrowserAgentResult> {
  const maxChars = Math.min(40_000, Math.max(500, Number(ctx.request.args.max_chars) || 20_000));
  let value: { text: string; truncated: boolean } | undefined;
  if (typeof ctx.request.args.ref === 'string') {
    const node = await resolveRef(ctx, ctx.request.args.ref);
    value = await callOn(ctx, node.objectId, node.sessionId, READ_AS_MARKDOWN_FUNCTION, [maxChars]);
  } else {
    const executionContextId = await worldFor(ctx, 'main');
    const result = await ctx.state.cdp.send<{
      result: { value?: { text: string; truncated: boolean } };
      exceptionDetails?: { text?: string };
    }>('Runtime.callFunctionOn', {
      functionDeclaration: READ_AS_MARKDOWN_FUNCTION,
      executionContextId,
      arguments: [{ value: maxChars }],
      returnByValue: true,
    });
    if (result.exceptionDetails) throw new OpFailure('The page text could not be read.');
    value = result.result.value;
  }
  const fitted = fitText(value?.text ?? '', maxChars);
  return {
    ok: true,
    detail: `read ${fitted.text.length} characters`,
    data: { content: fitted.text, truncated: Boolean(value?.truncated) || fitted.cut },
  };
}

async function opScreenshot(ctx: Context): Promise<BrowserAgentResult> {
  const cdp = ctx.state.cdp;
  let params: Record<string, unknown> = { format: 'png' };
  let what = 'the visible page';
  if (typeof ctx.request.args.ref === 'string') {
    const node = await resolveRef(ctx, ctx.request.args.ref);
    await cdp
      .send(
        'DOM.scrollIntoViewIfNeeded',
        { backendNodeId: node.target.backendNodeId },
        node.sessionId
      )
      .catch(() => undefined);
    const quads = await cdp.send<{ quads: number[][] }>(
      'DOM.getContentQuads',
      { backendNodeId: node.target.backendNodeId },
      node.sessionId
    );
    const box = boxOfQuads(quads.quads);
    if (!box || box.width < 1 || box.height < 1)
      throw new OpFailure(`${node.target.ref} has no size on the page.`);
    const metrics = await cdp.send<{ cssVisualViewport: { pageX: number; pageY: number } }>(
      'Page.getLayoutMetrics'
    );
    params = {
      format: 'png',
      captureBeyondViewport: true,
      clip: {
        x: box.x + node.offset.x + metrics.cssVisualViewport.pageX,
        y: box.y + node.offset.y + metrics.cssVisualViewport.pageY,
        width: Math.min(box.width, 4_000),
        height: Math.min(box.height, FULL_PAGE_MAX_CSS_HEIGHT),
        scale: 1,
      },
    };
    what = node.label;
  } else if (ctx.request.args.full_page === true) {
    const metrics = await cdp.send<{ cssContentSize: { width: number; height: number } }>(
      'Page.getLayoutMetrics'
    );
    params = {
      format: 'png',
      captureBeyondViewport: true,
      clip: {
        x: 0,
        y: 0,
        width: Math.min(metrics.cssContentSize.width, 4_000),
        height: Math.min(metrics.cssContentSize.height, FULL_PAGE_MAX_CSS_HEIGHT),
        scale: 1,
      },
    };
    what =
      metrics.cssContentSize.height > FULL_PAGE_MAX_CSS_HEIGHT
        ? `the page's first ${FULL_PAGE_MAX_CSS_HEIGHT} CSS pixels`
        : 'the whole page';
  }
  const shot = await cdp.send<{ data: string }>('Page.captureScreenshot', params);
  let png: Buffer = Buffer.from(shot.data, 'base64');
  if (ctx.deps.resizePng) {
    png =
      ctx.request.args.full_page === true
        ? ctx.deps.resizePng(png, 1_200, 16_000)
        : ctx.deps.resizePng(png, VIEWPORT_LONG_SIDE, VIEWPORT_LONG_SIDE);
  }
  const path = await ctx.deps.writeScreenshot(png);
  return { ok: true, detail: `captured ${what}`, data: { screenshot_path: path } };
}

async function opScroll(ctx: Context): Promise<BrowserAgentResult> {
  const direction = ctx.request.args.direction as 'up' | 'down' | 'left' | 'right' | null;
  const ref = ctx.request.args.ref as string | null;
  let node: Resolved | null = null;
  if (ref) {
    node = await resolveRef(ctx, ref);
    await ctx.state.cdp
      .send(
        'DOM.scrollIntoViewIfNeeded',
        { backendNodeId: node.target.backendNodeId },
        node.sessionId
      )
      .catch(() => undefined);
    if (!direction)
      return { ok: true, detail: `scrolled ${node.target.ref} ${node.label} into view` };
  }
  const viewport = await viewportOf(ctx);
  const amount =
    Number(ctx.request.args.amount) ||
    Math.round(
      (direction === 'left' || direction === 'right' ? viewport.width : viewport.height) * 0.8
    );
  let point = { x: viewport.width / 2, y: viewport.height / 2 };
  if (node) {
    const quads = await ctx.state.cdp.send<{ quads: number[][] }>(
      'DOM.getContentQuads',
      { backendNodeId: node.target.backendNodeId },
      node.sessionId
    );
    const inner = clickPoint(quads.quads, await viewportOf(ctx, node.sessionId));
    if (inner) point = { x: inner.x + node.offset.x, y: inner.y + node.offset.y };
  }
  const deltaX = direction === 'left' ? -amount : direction === 'right' ? amount : 0;
  const deltaY = direction === 'up' ? -amount : direction === 'down' ? amount : 0;
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    await dispatchMouse(ctx, 'mouseWheel', point, { deltaX, deltaY });
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
  }
  await delay(250, ctx.signal);
  return {
    ok: true,
    detail: `scrolled ${direction} ${amount} px${node ? ` inside ${node.label}` : ''}`,
  };
}

async function opHover(ctx: Context): Promise<BrowserAgentResult> {
  const node = await resolveRef(ctx, ctx.request.args.ref);
  const facts = await factsOf(ctx, node);
  const point = await actionablePoint(ctx, node, facts, false);
  const hide = await highlight(ctx, node);
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    await dispatchMouse(ctx, 'mouseMoved', point);
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
    await hide();
  }
  await delay(200, ctx.signal);
  return { ok: true, detail: `hovered ${node.label}` };
}

function globToRegExp(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
  return new RegExp(escaped);
}

async function opWait(ctx: Context): Promise<BrowserAgentResult> {
  const args = ctx.request.args as {
    text?: string | null;
    ref?: string | null;
    state?: string | null;
    url?: string | null;
    load?: string | null;
    time_ms?: number | null;
    timeout_ms?: number;
  };
  if (args.time_ms && !args.text && !args.ref && !args.url && !args.load) {
    await delay(Math.min(60_000, args.time_ms), ctx.signal);
    return { ok: true, detail: `waited ${args.time_ms} ms` };
  }
  const timeout = Math.min(60_000, Math.max(100, Number(args.timeout_ms) || 10_000));
  const until = Math.min(Date.now() + timeout, ctx.deadline - 500);
  const want = args.state ?? 'visible';
  const condition = async (): Promise<boolean> => {
    if (args.url) {
      const url = ctx.contents.getURL();
      if (!(url.includes(args.url) || globToRegExp(args.url).test(url))) return false;
    }
    if (args.load) {
      if (args.load === 'load' && ctx.contents.isLoading()) return false;
      if (args.load === 'domcontentloaded') {
        const ready = await readyState(ctx.state);
        if (ready === 'loading') return false;
      }
    }
    if (args.text) {
      const world = await worldFor(ctx, 'main');
      const found = await ctx.state.cdp
        .send<{ result: { value?: boolean } }>('Runtime.evaluate', {
          expression: `(() => { const b = document.body; return !!b && b.innerText.includes(${JSON.stringify(args.text)}); })()`,
          contextId: world,
          returnByValue: true,
        })
        .catch(() => ({ result: { value: false } }));
      if (found.result.value !== true) return false;
    }
    if (args.ref) {
      let node: Resolved | null = null;
      try {
        node = await resolveRef(ctx, args.ref);
      } catch {
        node = null;
      }
      if (want === 'detached') return node === null;
      if (!node) return false;
      const facts = await factsOf(ctx, node).catch(() => null);
      if (!facts) return want === 'hidden';
      const quads = await ctx.state.cdp
        .send<{
          quads: number[][];
        }>('DOM.getContentQuads', { backendNodeId: node.target.backendNodeId }, node.sessionId)
        .catch(() => ({ quads: [] as number[][] }));
      const visible = checkElement(facts, boxOfQuads(quads.quads), false, node.target.ref).ok;
      if (want === 'visible' && !visible) return false;
      if (want === 'hidden' && visible) return false;
      if (want === 'enabled' && facts.disabled) return false;
      if (want === 'disabled' && !facts.disabled) return false;
    }
    return true;
  };
  while (Date.now() < until) {
    // Each pass gets fresh isolated worlds: a navigation ends the old ones.
    ctx.worlds.clear();
    if (await condition().catch(() => false)) {
      return { ok: true, detail: 'the condition was met' };
    }
    await delay(250, ctx.signal);
  }
  throw new OpFailure(`Waited ${timeout} ms and the condition was not met.`);
}

async function opClick(ctx: Context): Promise<BrowserAgentResult> {
  const node = await resolveRef(ctx, ctx.request.args.ref);
  const facts = await factsOf(ctx, node);
  if (ctx.request.grant !== 'act' && !isPlainLinkClick(facts)) {
    // Nothing has happened: the daemon asks the person for the act grant and sends this again.
    return {
      ok: false,
      detail: `clicking ${node.label} needs the permission to act on websites`,
      data: { needs_act_grant: true },
    };
  }
  const point = await actionablePoint(ctx, node, facts, true);
  const hide = await highlight(ctx, node);
  const before = ctx.state.mainNavigations;
  const stopDownloads = withDownloads(ctx);
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    await dispatchMouse(ctx, 'mouseMoved', point);
    await dispatchMouse(ctx, 'mousePressed', point);
    await untilDialog(ctx, dispatchMouse(ctx, 'mouseReleased', point));
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
    await hide();
  }
  try {
    const navigated = ctx.state.dialog ? false : await afterAction(ctx, before);
    return {
      ok: true,
      detail: `clicked ${node.label}`,
      data: { navigated, refs_invalidated: navigated },
    };
  } finally {
    stopDownloads();
  }
}

async function focusNode(ctx: Context, node: Resolved): Promise<void> {
  await callOn(
    ctx,
    node.objectId,
    node.sessionId,
    'function () { if (typeof this.focus === "function") this.focus(); }'
  );
}

function refuseProtected(facts: ElementFacts): void {
  if (isProtectedInput(facts))
    throw new OpFailure(PROTECTED_FIELD_REFUSAL, { error_kind: 'protected_field' });
}

async function opFill(ctx: Context): Promise<BrowserAgentResult> {
  const node = await resolveRef(ctx, ctx.request.args.ref);
  const facts = await factsOf(ctx, node);
  refuseProtected(facts);
  const value = String(ctx.request.args.value ?? '');
  if (facts.tag === 'select') throw new OpFailure(`${node.label} is a select; use browser_select.`);
  const fillable =
    facts.tag === 'textarea' ||
    facts.editable ||
    (facts.tag === 'input' && !UNFILLABLE_INPUT_TYPES.has(facts.type));
  if (!fillable) {
    throw new OpFailure(
      `${node.label} is not a text field. Use browser_click for buttons, checkboxes and radio buttons.`
    );
  }
  if (facts.readOnly) throw new OpFailure(`${node.label} is read-only.`);
  await actionablePoint(ctx, node, facts, true);
  const hide = await highlight(ctx, node);
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    await focusNode(ctx, node);
    if (facts.tag === 'input' && SET_VALUE_INPUT_TYPES.has(facts.type)) {
      await callOn(
        ctx,
        node.objectId,
        node.sessionId,
        `function (value) {
          const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
          setter.call(this, value);
          this.dispatchEvent(new Event('input', { bubbles: true }));
          this.dispatchEvent(new Event('change', { bubbles: true }));
        }`,
        [value]
      );
    } else {
      await callOn(
        ctx,
        node.objectId,
        node.sessionId,
        `function () {
          if (typeof this.select === 'function' && (this.tagName === 'INPUT' || this.tagName === 'TEXTAREA')) { this.select(); return; }
          const range = document.createRange(); range.selectNodeContents(this);
          const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
        }`
      );
      if (value) {
        await ctx.state.cdp.send('Input.insertText', { text: value });
      } else {
        await pressStroke(ctx, { key: 'Backspace', code: 'Backspace', keyCode: 8, modifiers: 0 });
      }
    }
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
    await hide();
  }
  return { ok: true, detail: `filled ${node.label}` };
}

async function focusedIsProtected(ctx: Context): Promise<'protected' | 'frame' | 'ok'> {
  const world = await worldFor(ctx, 'main');
  const result = await ctx.state.cdp
    .send<{ result: { value?: string } }>('Runtime.evaluate', {
      expression: `(() => {
        let el = document.activeElement;
        while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
        if (!el) return 'ok';
        if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') return 'frame';
        const type = (el.getAttribute('type') || '').toLowerCase();
        const ac = (el.getAttribute('autocomplete') || '').toLowerCase();
        if (el.tagName === 'INPUT' && type === 'password') return 'protected';
        if (/(^|\\s)(current-password|new-password|one-time-code|cc-[a-z-]+)(\\s|$)/.test(ac)) return 'protected';
        return 'ok';
      })()`,
      contextId: world,
      returnByValue: true,
    })
    .catch(() => ({ result: { value: 'ok' } }));
  return (result.result.value as 'protected' | 'frame' | 'ok') ?? 'ok';
}

async function opType(ctx: Context): Promise<BrowserAgentResult> {
  const text = String(ctx.request.args.text ?? '');
  let label = 'the focused element';
  if (typeof ctx.request.args.ref === 'string') {
    const node = await resolveRef(ctx, ctx.request.args.ref);
    const facts = await factsOf(ctx, node);
    refuseProtected(facts);
    await actionablePoint(ctx, node, facts, true);
    await focusNode(ctx, node);
    label = node.label;
  } else {
    const focused = await focusedIsProtected(ctx);
    if (focused === 'protected')
      throw new OpFailure(PROTECTED_FIELD_REFUSAL, { error_kind: 'protected_field' });
    if (focused === 'frame') {
      throw new OpFailure(
        'The focused element is inside a frame. Pass ref so the field can be checked before typing.'
      );
    }
  }
  const before = ctx.state.mainNavigations;
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    for (const char of text) {
      if (ctx.signal.aborted) throw abortError(ctx.signal);
      const stroke = strokeForCharacter(char);
      if (stroke.keyCode === 0) await ctx.state.cdp.send('Input.insertText', { text: char });
      else await untilDialog(ctx, pressStroke(ctx, stroke));
      if (ctx.state.dialog) break;
    }
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
  }
  const navigated = ctx.state.dialog ? false : await afterAction(ctx, before);
  return {
    ok: true,
    detail: `typed ${[...text].length} characters into ${label}`,
    data: { navigated, refs_invalidated: navigated },
  };
}

async function opPress(ctx: Context): Promise<BrowserAgentResult> {
  const parsed = parseChord(String(ctx.request.args.key ?? ''), process.platform);
  if (!parsed.ok) throw new OpFailure(parsed.error);
  if (typeof ctx.request.args.ref === 'string') {
    const node = await resolveRef(ctx, ctx.request.args.ref);
    const facts = await factsOf(ctx, node);
    await actionablePoint(ctx, node, facts, false);
    await focusNode(ctx, node);
  }
  const before = ctx.state.mainNavigations;
  const stopDownloads = withDownloads(ctx);
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    await untilDialog(ctx, pressStroke(ctx, parsed.stroke));
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
  }
  try {
    const navigated = ctx.state.dialog ? false : await afterAction(ctx, before);
    return {
      ok: true,
      detail: `pressed ${String(ctx.request.args.key)}`,
      data: { navigated, refs_invalidated: navigated },
    };
  } finally {
    stopDownloads();
  }
}

async function opSelect(ctx: Context): Promise<BrowserAgentResult> {
  const node = await resolveRef(ctx, ctx.request.args.ref);
  const facts = await factsOf(ctx, node);
  if (facts.tag !== 'select') {
    throw new OpFailure(
      `${node.label} is not a select element. Click it, then click the option you want.`
    );
  }
  await actionablePoint(ctx, node, facts, true);
  const values = (ctx.request.args.values as unknown[]).map(String).slice(0, 100);
  const hide = await highlight(ctx, node);
  let outcome: { chosen?: string[]; missing?: string };
  beginAgentInput(ctx.window, ctx.info.viewId);
  try {
    outcome = await callOn(
      ctx,
      node.objectId,
      node.sessionId,
      `function (values) {
        const options = Array.from(this.options);
        const chosen = [];
        const picks = this.multiple ? values : values.slice(0, 1);
        const matches = [];
        for (const wanted of picks) {
          const option = options.find((o) => o.value === wanted) || options.find((o) => (o.label || o.textContent || '').trim() === wanted.trim());
          if (!option) return { missing: wanted };
          matches.push(option);
        }
        if (this.multiple) for (const o of options) o.selected = false;
        for (const option of matches) { option.selected = true; chosen.push((option.label || option.textContent || '').trim()); }
        this.dispatchEvent(new Event('input', { bubbles: true }));
        this.dispatchEvent(new Event('change', { bubbles: true }));
        return { chosen };
      }`,
      [values]
    );
  } finally {
    endAgentInput(ctx.window, ctx.info.viewId);
    await hide();
  }
  if (outcome.missing !== undefined) {
    throw new OpFailure(`${node.label} has no option ${JSON.stringify(outcome.missing)}.`);
  }
  return {
    ok: true,
    detail: `chose ${(outcome.chosen ?? []).map((v) => JSON.stringify(v)).join(', ')} in ${node.label}`,
  };
}

async function opDialog(ctx: Context): Promise<BrowserAgentResult> {
  const dialog = ctx.state.dialog;
  if (!dialog) throw new OpFailure('No dialog is open on this page.');
  const accept = ctx.request.args.accept === true;
  const promptText =
    typeof ctx.request.args.prompt_text === 'string' ? ctx.request.args.prompt_text : undefined;
  await ctx.state.cdp.send(
    'Page.handleJavaScriptDialog',
    { accept, ...(promptText !== undefined ? { promptText } : {}) },
    dialog.sessionId
  );
  ctx.state.dialog = null;
  const before = ctx.state.mainNavigations;
  const navigated = await afterAction(ctx, before);
  return {
    ok: true,
    detail: `${accept ? 'accepted' : 'dismissed'} the ${dialog.type} dialog`,
    data: { navigated, refs_invalidated: navigated },
  };
}

const OPERATIONS: Record<string, (ctx: Context) => Promise<BrowserAgentResult>> = {
  open: opOpen,
  navigate: opNavigate,
  snapshot: opSnapshot,
  read: opRead,
  screenshot: opScreenshot,
  scroll: opScroll,
  hover: opHover,
  wait: opWait,
  click: opClick,
  fill: opFill,
  type: opType,
  press: opPress,
  select: opSelect,
  dialog: opDialog,
};

/** What the operation's tab looks like now, for every result (BB-09). */
function tabFacts(ctx: Context, before: Set<string>): NonNullable<BrowserAgentResult['data']> {
  const info = agentBrowserTab(ctx.window, ctx.info.viewId);
  const opened = agentBrowserTabs(ctx.window, ctx.request.sessionId)
    .filter((tab) => !before.has(tab.viewId))
    .map((tab) => ({ tab: tab.label, url: tab.url }));
  const dialog = ctx.state.dialog;
  return {
    tab: ctx.info.label,
    url: info?.url ?? '',
    title: info?.title ?? '',
    opened_tabs: opened,
    dialog: dialog
      ? {
          type: dialog.type,
          message: dialog.message,
          default_prompt: dialog.defaultPrompt || undefined,
        }
      : null,
    download: ctx.state.download,
    person_interacted: (info?.personInputAt ?? 0) > ctx.startedAt,
  };
}

/** The tab an operation runs on: the chat's current agent tab, or a new one. */
function pickTab(window: BrowserWindow, request: BrowserAgentRequest): AgentTabInfo {
  const key = sessionKey(window, request.sessionId);
  const currentId = currentTabs.get(key);
  let current = currentId ? agentBrowserTab(window, currentId) : null;
  if (!current) {
    // The current tab closed: fall back to the chat's newest agent tab.
    const all = agentBrowserTabs(window, request.sessionId);
    current = all[all.length - 1] ?? null;
    if (current) currentTabs.set(key, current.viewId);
  }
  if (request.op === 'open' && (request.args.new_tab === true || !current)) {
    const created = createAgentBrowserTab(window, request.sessionId);
    if (!created) throw new OpFailure('The browser could not open a tab in this window.');
    currentTabs.set(key, created.viewId);
    return created;
  }
  if (request.op === 'tabs') {
    const action = String(request.args.action ?? 'list');
    if (action !== 'list') {
      const label = String(request.args.tab ?? '');
      const found = agentBrowserTabs(window, request.sessionId).find((tab) => tab.label === label);
      if (!found) throw new OpFailure(`There is no tab ${label}. Use browser_tabs to list them.`);
      return found;
    }
  }
  if (!current) {
    throw new OpFailure('No browser tab is open in this chat yet. Use browser_open first.', {
      error_kind: 'no_tab',
    });
  }
  return current;
}

/**
 * Run one operation from the daemon (BB-10). Never throws: every outcome is a result the model
 * can read.
 */
export async function runAgentBrowserOperation(
  window: BrowserWindow,
  request: BrowserAgentRequest,
  deps: DriverDeps
): Promise<BrowserAgentResult> {
  if (!BROWSER_AGENT_OPS.has(request.op)) {
    return { ok: false, detail: `${String(request.op)} is not a browser operation.` };
  }
  if (request.op === 'tabs' && String(request.args.action ?? 'list') === 'list') {
    const tabs = tabInfos(window, request.sessionId);
    return {
      ok: true,
      detail: tabs.length
        ? `${tabs.length} tab${tabs.length === 1 ? '' : 's'}`
        : 'no browser tabs are open in this chat',
      data: { tabs },
    };
  }
  const controller = new AbortController();
  const deadlineMs = Math.min(120_000, Math.max(1_000, request.deadlineMs || 30_000));
  const deadline = Date.now() + deadlineMs;
  const timer = setTimeout(() => controller.abort(new Error('timeout')), deadlineMs);
  inFlight.set(request.requestId, { controller, contents: null });
  let info: AgentTabInfo | null = null;
  try {
    info = pickTab(window, request);
    const contents = info.contents;
    inFlight.set(request.requestId, { controller, contents });
    if (request.op === 'tabs') {
      const action = String(request.args.action);
      if (action === 'close') {
        closeAgentBrowserTab(window, info.viewId);
        const key = sessionKey(window, request.sessionId);
        if (currentTabs.get(key) === info.viewId) currentTabs.delete(key);
        return {
          ok: true,
          detail: `closed tab ${info.label}`,
          data: { tabs: tabInfos(window, request.sessionId) },
        };
      }
      currentTabs.set(sessionKey(window, request.sessionId), info.viewId);
      await waitShown(window, info.viewId, controller.signal);
      return {
        ok: true,
        detail: `switched to tab ${info.label}`,
        data: {
          tabs: tabInfos(window, request.sessionId),
          tab: info.label,
          url: info.url,
          title: info.title,
        },
      };
    }
    const shownInfo = await waitShown(window, info.viewId, controller.signal);
    const state = stateFor(contents);
    if (state.dialog && request.op !== 'dialog') {
      throw new OpFailure(
        `A ${state.dialog.type} dialog is open and the page is waiting for it. Answer it with browser_dialog first.`,
        { dialog: { type: state.dialog.type, message: state.dialog.message } }
      );
    }
    const before = new Set(agentBrowserTabs(window, request.sessionId).map((tab) => tab.viewId));
    const run = async (): Promise<BrowserAgentResult> => {
      const ctx: Context = {
        window,
        request,
        contents,
        info: shownInfo,
        state,
        signal: controller.signal,
        deadline,
        startedAt: Date.now(),
        deps,
        worlds: new Map(),
      };
      reportAgentActivity(window, shownInfo.viewId, true);
      try {
        // Inside the deadline: attaching waits for the tab's renderer.
        await raceAbort(state.cdp.attach(), controller.signal);
        const result = await raceAbort(OPERATIONS[request.op](ctx), controller.signal);
        return { ...result, data: { ...tabFacts(ctx, before), ...result.data } };
      } catch (error) {
        const failure = error instanceof OpFailure ? error : null;
        const reason = controller.signal.aborted
          ? String(controller.signal.reason?.message ?? controller.signal.reason) === 'timeout'
            ? `The operation timed out after ${Math.round(deadlineMs / 1000)} s. The page may have changed: take a browser_snapshot before acting again, and never repeat a click, fill or submit blindly.`
            : 'The operation was cancelled.'
          : failure
            ? failure.message
            : `The browser could not do that: ${error instanceof Error ? error.message : String(error)}`;
        if (!failure && !controller.signal.aborted)
          log.warn('[BiorouterBrowser] agent operation failed:', String(error));
        return {
          ok: false,
          detail: reason,
          data: { ...tabFacts(ctx, before), ...(failure?.extra ?? {}) },
        };
      } finally {
        reportAgentActivity(window, shownInfo.viewId, false);
      }
    };
    const queued = state.queue.then(run, run);
    state.queue = queued.catch(() => undefined);
    return await queued;
  } catch (error) {
    const failure = error instanceof OpFailure ? error : null;
    return {
      ok: false,
      detail: failure
        ? failure.message
        : controller.signal.aborted
          ? 'The operation was cancelled.'
          : `The browser could not do that: ${error instanceof Error ? error.message : String(error)}`,
      data: {
        ...(info ? { tab: info.label, url: info.url, title: info.title } : {}),
        ...(failure?.extra ?? {}),
      },
    };
  } finally {
    clearTimeout(timer);
    inFlight.delete(request.requestId);
  }
}
