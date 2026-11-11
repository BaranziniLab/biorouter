/**
 * Biorouter App SDK
 * =================
 * The runtime every Agent-Drafter-built app uses to talk to a *real* Biorouter
 * agent. Unlike the old "bridge" mode (which only forwarded a prompt into the
 * Biorouter chat box), this SDK opens a per-app WebSocket to the Biorouter
 * backend, which runs the full agent loop — the app's own model, extensions,
 * skills and knowledge base — and streams the answer (text / markdown / tool
 * activity) straight back into the app.
 *
 * The app is authored in TypeScript and bundled with esbuild; this module is
 * `import`ed by the app's `main.ts`. It has no external dependencies (markdown
 * rendering is built in) so a built app is fully self-contained and works when
 * served by `biorouterd` or opened as a standalone export.
 */

export interface AppConfig {
  /** Stable app id (matches the on-disk project + the backend route). */
  appId: string;
  /**
   * Explicit agent WebSocket endpoint. When omitted, it is derived from the
   * page location: `ws[s]://<host>/apps/<appId>/agent`.
   */
  endpoint?: string;
  /**
   * Candidate endpoints, tried in order until one connects. Standalone exports
   * set this so the app works whether it is served by the daemon itself, by the
   * bundled `serve.mjs` proxy, or opened straight off disk while a daemon runs
   * on some other port.
   */
  endpoints?: string[];
  /** Greeting shown when the default chat panel mounts. */
  greeting?: string;
  /** Auto-mount a chat panel into `[data-br-chat]` if the app has no custom UI. */
  autoChat?: boolean;
  /** Mount the agent-driven UI runtime (panels, charts, highlights). Default on. */
  ui?: boolean;
  /** Initial mode: "light", "dark", or "auto" (follow the viewer's OS).
   *  Left unset → the manifest's theme pack when one is selected, otherwise
   *  deterministic light. The agent's `ui_theme` overrides. */
  theme?: "light" | "dark" | "auto";
  /**
   * Per-app WebSocket auth token minted into the served page (same-origin
   * readable only). When present, `connect()` appends `token=<wsToken>` to the
   * agent WebSocket URL query, alongside `client_id`. The server requires it on
   * upgrade (SDK v2 socket authority); v1 pages simply omit it.
   */
  wsToken?: string;
  /**
   * The app's declared initial shared-state document (`surface.state_initial`).
   *
   * Seeded into the runtime's doc at construction, so bindings paint correctly on
   * first load rather than blank-until-the-first-agent-turn. The server's snapshot
   * (which carries durable state from a previous session) overwrites it on connect.
   */
  stateInitial?: unknown;
}

export interface ImageInput {
  /** MIME type, e.g. "image/png". */
  mimeType: string;
  /** Base64-encoded image bytes (no data-URL prefix). */
  data: string;
}

/** Options for `br.dnd.catalog(...)`. */
export interface DndCatalogOptions {
  /** Container of the draggable items (each marked `data-br-item="<id>"`). */
  source: HTMLElement | string;
  /** One or more drop zones (each marked `data-br-zone="<id>"`). */
  target: HTMLElement | HTMLElement[] | string;
  /** Declared signal to emit on a drop. The primitive sends it for you. */
  signal?: string;
  /** Author callback, in addition to the signal. */
  onDrop?: (item: string, zone: string) => void;
}

export interface PromptOptions {
  images?: ImageInput[];
  // SDK v2 (`br.run` turn control): trailing-edge debounce per run target, and
  // supersede — cancel the in-flight superseding run (resolving it early with
  // its partial text) and start this one instead of queueing behind it.
  debounceMs?: number;
  supersede?: boolean;
  // SDK v2 (§3.8) multi-agent facade: the worker profile this prompt/run is
  // scoped to. Set by `br.agent(name)`; rides the outgoing frame as `agent`, and
  // settles / filters against server frames that carry the same `agent`.
  agent?: string;
  /**
   * Where to show tool-call progress for this run.
   *
   * A selector or element routes progress there; `false` suppresses it entirely.
   * Omit and the SDK uses the app's existing progress surface if it has one, and
   * only falls back to an inline strip inside the run target if it does not — so
   * progress never displaces the result by default.
   */
  progress?: HTMLElement | string | false;
  /**
   * How long to wait for the turn to finish before abandoning it (default
   * `RUN_STALL_MS`). A turn that never emits `done` used to leave the run queue
   * pending forever, so every later `br.run` sat behind it — unsent, unpainted,
   * and indistinguishable from a broken button.
   */
  timeoutMs?: number;
}

export interface TimelineOptions {
  maxItems?: number;
}

export interface TimelineSummary {
  label: string;
  detail: string;
  state: string;
}

/** Events emitted while the agent answers a prompt. */
export type AgentEvent =
  | { type: "ready"; protocol?: number; capabilities?: string[]; sessionId?: string; resumed?: boolean; messageCount?: number; surface?: ReadySurface; profiles?: string[] }
  | { type: "message"; delta: string }
  | { type: "thought"; delta: string }
  | { type: "tool"; name: string; status: string; id?: string }
  | { type: "done"; degraded?: boolean; missingProfiles?: string[] }
  | { type: "error"; message: string }
  // ── BRSDK protocol v2 (additive; gated by the ready frame's capabilities) ──
  | { type: "output"; callId?: string; schema?: unknown; value: unknown }
  // The server reprompts when a structured result misses its schema. Shown so a
  // retry reads as work in progress rather than as a stall.
  | { type: "output_retry"; attempt?: number; errors?: unknown }
  | { type: "usage"; inputTokens?: number; outputTokens?: number; totalTokens?: number; model?: string }
  | { type: "guardrail"; stage?: string; name?: string; blocked?: boolean; reason?: string }
  | { type: "approval"; requestId: string; tool: string; args?: unknown; prompt?: string | null }
  // A decision this surface may not grant: the approval needs proof that a
  // *person* decided and an app page cannot supply it, so the server denied the
  // tool and the turn moved on. With no consumer the page simply stopped saying
  // anything, which reads as a hang (issue #143).
  | { type: "approval_refused"; requestId: string; reason?: string }
  | { type: "tool_call"; id: string; name: string; args?: unknown }
  | { type: "handoff"; from?: string; to?: string }
  | { type: "compaction"; phase: string; trigger?: string; before?: number; after?: number }
  | { type: "trace"; span?: unknown; snapshot?: unknown }
  | { type: "context"; used?: number; limit?: number; ratio?: number }
  | { type: "history"; messages?: Array<{ role: string; text: string }> }
  | { type: "model"; ok: boolean; provider?: string; model?: string }
  // Sent once on socket open, and ONLY when the report is degraded — which is
  // broader than "not installed". The daemon's `CapabilityReport::degraded()` is
  // true when any named skill is absent, when the knowledge base is absent OR
  // present but refused to this agent by the privacy barrier (the report omits a
  // base it may not reach rather than confirming it exists), or when any
  // declared `requires` entry is unmet. `degraded` is therefore always `true` on
  // the wire; see `eventSummary`'s arm.
  | { type: "capability_report"; degraded?: boolean; report?: CapabilityReportBody }
  | { type: "widget"; id: string; tree: unknown }
  // ── SDK v2 Phase 4 (br.kb + model status; additive, gated by capabilities) ──
  | { type: "kb_result"; reqId?: string; result?: unknown; error?: string }
  | { type: "kb_progress"; reqId?: string; stage?: string; detail?: string; pct?: number }
  | { type: "model_status"; provider?: string; model?: string; ready?: boolean; detail?: string }
  // ── Agent-driven UI control (BRSDK v3) ──
  // The agent's `ui_*` tools push these; the UI runtime below applies them.
  | ({ type: "ui" } & UiCommand);

type EventKind = AgentEvent["type"];
type Listener = (ev: AgentEvent) => void;

// Named aliases so the no-esbuild fallback type-stripper can remove these
// annotations (it keys off an uppercase/primitive leading type token, which a
// bare `(() => void)` annotation lacks).
type ResolveFn = () => void;
type RejectFn = (e: Error) => void;

/** The `capability_report` frame's body (the server's `CapabilityReport`).
 *  Named so the field reads survive the no-esbuild fallback stripper. */
export interface CapabilityReportBody {
  configured_skills?: string[];
  granted_skills?: string[];
  missing_skills?: string[];
  configured_knowledge_base?: string | null;
  granted_knowledge_base?: string | null;
  missing_knowledge_base?: string | null;
  unmet_requirements?: unknown[];
}

/** Wildcard listener bucket (see `onAny`). Not a valid `EventKind`, so it can
 *  never collide with a real frame type. */
const ANY_EVENT = "*";

/**
 * Every frame `type` this SDK version understands.
 *
 * It exists because an unknown TOP-LEVEL frame used to reach `emit`, find an
 * empty listener bucket and vanish without a trace — which is exactly how
 * `approval_refused` shipped with no consumer and left the page looking hung
 * while the agent had already been denied and moved on (issue #143). A frame
 * whose type is not listed here is now warned about once and drawn as a visible
 * "Unrecognized event" row, so the NEXT frame the daemon grows degrades loudly.
 *
 * ⚠ It is **`Record<EventKind, true>`, not `string[]`**, and that is the whole
 * point: a hand-kept array can silently omit a real frame type, which turns a
 * genuine daemon frame into a red "Unrecognized event" row in EVERY app — the
 * exact inverse of the bug this list exists to fix. As a `Record` keyed by the
 * `AgentEvent` union, a missing key and a misspelt one are both type errors:
 * `Record` requires every member of the union, and the object literal's excess
 * property check rejects a name the union does not have.
 *
 * The `string[]` view is derived with `Object.keys`, never written out twice.
 * (The declaration shape is also what the no-esbuild fallback stripper can
 * handle: `: Record<…>` and `: string[]` both lead with a token it recognises —
 * see the note at the `ResolveFn`/`RejectFn` aliases above.)
 */
const KNOWN_EVENT_KINDS_MAP: Record<EventKind, true> = {
  ready: true,
  message: true,
  thought: true,
  tool: true,
  done: true,
  error: true,
  output: true,
  output_retry: true,
  usage: true,
  guardrail: true,
  approval: true,
  approval_refused: true,
  capability_report: true,
  tool_call: true,
  handoff: true,
  compaction: true,
  trace: true,
  context: true,
  history: true,
  model: true,
  widget: true,
  kb_result: true,
  kb_progress: true,
  model_status: true,
  ui: true,
};

const KNOWN_EVENT_KINDS: string[] = Object.keys(KNOWN_EVENT_KINDS_MAP);

/** Whether this SDK version knows the frame type at all. */
function isKnownEventKind(kind: string): boolean {
  return KNOWN_EVENT_KINDS.indexOf(kind) >= 0;
}

// ── SDK v2 §3.8: multi-agent client facade ───────────────────────────────────
/** The optional `agent` routing field any v2 frame (client→server prompt/call,
 *  or server→client worker stream) may carry. Named so `(ev as AgentTagged)`
 *  survives the no-esbuild fallback stripper (an inline object-type cast does
 *  not). */
interface AgentTagged {
  agent?: string;
}

/** A scoped worker handle from `br.agent(name)`: every outgoing prompt/call
 *  carries `agent:name`, and `on()` fires only for frames whose `agent === name`.
 *  Named so the annotations strip cleanly. */
interface AgentFacade {
  prompt(text: string, opts?: PromptOptions): Promise<void>;
  ask(text: string, opts?: PromptOptions): Promise<string>;
  call(nameOrOpts: string | CallOpts, args?: unknown, opts?: CallOpts): Promise<CallResult>;
  run(text: string, target: HTMLElement | string, opts?: PromptOptions): Promise<string>;
  on(kind: EventKind, fn: Listener): AgentFacade;
}

/** A prompt awaiting its terminal `done`/`error`, scoped to one worker profile. */
interface AgentInflight {
  resolve: ResolveFn;
  reject: RejectFn;
}

/** The routing `agent` of a frame, or `""` for a main-facade frame (no agent). */
function frameAgent(ev: unknown): string {
  const a = (ev as AgentTagged).agent;
  return typeof a === "string" ? a : "";
}

// ── SDK v2 Phase 3: app.actions / br.call / app.signals ──────────────────────
// Single-ident named aliases (see note above): a bare inline function/object
// annotation would survive the no-esbuild fallback stripper, so every callback
// and record shape used in *code* positions goes through a named type.
type ActionHandler = (args: unknown) => unknown;
type CallResolve = (r: CallResult) => void;
type RunEarly = () => void;
type RunResolve = (text: string) => void;

/** The `output` frame's fields, named so a `msg as OutputFrame` cast survives the
 *  fallback stripper (an inline object-type cast does not). */
interface OutputFrame {
  callId?: string;
  value?: unknown;
  schema?: unknown;
}

/** The `br-network-select` CustomEvent shape (named for a strip-safe cast). */
interface NetSelectDetail {
  id: string | null;
}
interface NetSelectEvent {
  detail?: NetSelectDetail;
  target?: ElementLike;
}
interface ElementLike {
  closest?: (sel: string) => ElementLike | null;
  getAttribute: (name: string) => string | null;
}

/** Normalised argument bag for `br.call` (and the run-control subset). */
interface CallOpts {
  name?: string;
  args?: unknown;
  text?: string;
  outputSchema?: unknown;
  debounceMs?: number;
  supersede?: boolean;
  // SDK v2 §3.8: the worker profile this call is scoped to (set by `br.agent`).
  agent?: string;
}

/** What `br.call` resolves to: a structured `value` (from an `output` frame), or
 *  the accumulated `text` of the turn, or `superseded` when a newer call replaced
 *  it before it finished. */
interface CallResult {
  value?: unknown;
  text?: string;
  superseded?: boolean;
}

/** A `br.call` turn awaiting its `output`/`done`/`error`. */
interface PendingCall {
  callId: string;
  key: string;
  resolve: CallResolve;
  reject: RejectFn;
  textBuf: string;
  settled: boolean;
  superseding: boolean;
  // SDK v2 §3.8: the worker profile this call is scoped to, if any.
  agent?: string;
}

/** A trailing-edge debounce slot for `br.call`, keyed by call-key. */
interface CallDebounceRec {
  timer: ReturnType<typeof setTimeout>;
  resolve: CallResolve;
}

/** A pending coalesced signal (trailing-edge, last-payload-wins). */
interface SignalRec {
  payload: unknown;
  timer: number | ReturnType<typeof setTimeout>;
}

/** One declared signal from the `ready` surface (`coalesceMs` defaults to 250). */
interface SignalDecl {
  name: string;
  coalesceMs?: number;
}

/** The `ready` frame's optional advertised surface. */
interface ReadySurface {
  signals?: SignalDecl[];
  actions?: string[];
}

/** Live handle for an in-flight `br.run`, so a superseding run can resolve the
 *  prior one early with its partial text. */
interface RunHandle {
  superseding: boolean;
  settled: boolean;
  partial: string;
  resolveEarly: RunEarly;
}

/** A trailing-edge debounce slot for `br.run`, keyed by run target. */
interface RunDebounceRec {
  timer: ReturnType<typeof setTimeout>;
  resolve: RunResolve;
  reject: RejectFn;
}

// ── SDK v2 Phase 4: br.kb (knowledge bases) + br.model.status ────────────────
// Single-ident named aliases (see the note near ResolveFn): every callback and
// record shape used in a *code* position goes through a named type so the
// no-esbuild fallback stripper can erase the annotation.
/** What a `br.kb` request resolves to — op-shaped and opaque to the SDK. */
type KbResult = unknown;
/** Resolve fn for a pending `br.kb` request. */
type KbResolve = (r: KbResult) => void;
/** `br.kb.ingest` progress callback. */
type KbProgressFn = (p: KbProgress) => void;
/** `br.model.status()` resolve fn. */
type ModelStatusResolve = (s: ModelStatus) => void;

/** A `kb_progress` frame streamed during an `ingest`. */
interface KbProgress {
  stage: string;
  detail?: string;
  pct?: number;
}
/** Options common to every `br.kb` call (per-call timeout override). */
interface KbCallOpts {
  timeoutMs?: number;
}
/** Options for `br.kb.search`. */
interface KbSearchOpts {
  limit?: number;
  timeoutMs?: number;
}
/** The server-side `params` a `br.kb.search` sends. */
interface KbSearchParams {
  query: string;
  limit?: number;
}
/** Options for `br.kb.ingest`. */
interface KbIngestOpts {
  onProgress?: KbProgressFn;
  timeoutMs?: number;
}
/** A `br.kb` request awaiting its terminal `kb_result`. */
interface PendingKb {
  reqId: string;
  op: string;
  resolve: KbResolve;
  reject: RejectFn;
  onProgress?: KbProgressFn;
  timer: number | ReturnType<typeof setTimeout>;
  settled: boolean;
}
/** The fields of a `kb_result` frame (named for a strip-safe cast). */
interface KbResultFrame {
  reqId?: string;
  result?: unknown;
  error?: string;
}
/** The fields of a `kb_progress` frame (named for a strip-safe cast). */
interface KbProgressFrame {
  reqId?: string;
  stage?: string;
  detail?: string;
  pct?: number;
}
/** What `br.model.status()` resolves to. */
interface ModelStatus {
  provider?: string;
  model?: string;
  ready?: boolean;
  detail?: string;
}
/** The fields of a `model_status` reply frame (named for a strip-safe cast). */
interface ModelStatusFrame {
  provider?: string;
  model?: string;
  ready?: boolean;
  detail?: string;
}
/** A pending `br.model.status()` awaiting its reply (or timeout). */
interface PendingModelStatus {
  resolve: ModelStatusResolve;
  reject: RejectFn;
  timer: number | ReturnType<typeof setTimeout>;
  settled: boolean;
}
/** A KB `graph()` result's loose node shape (defensive field access). */
interface KbNodeLike {
  id?: unknown;
  name?: unknown;
  label?: unknown;
  title?: unknown;
  type?: unknown;
  kind?: unknown;
  group?: unknown;
}
/** A KB `graph()` result's loose edge shape (defensive field access). */
interface KbEdgeLike {
  source?: unknown;
  target?: unknown;
  from?: unknown;
  to?: unknown;
  kind?: unknown;
  relation?: unknown;
  predicate?: unknown;
  label?: unknown;
  type?: unknown;
}
/** A KB `graph()` result envelope (`{nodes, edges}`, both loose). */
interface KbGraphLike {
  nodes?: unknown[];
  edges?: unknown[];
}

declare global {
  interface Window {
    BIOROUTER_APP_CONFIG?: AppConfig;
    BioRouter?: BioRouterClient;
    Biorouter?: BioRouterClient;
    /** Not in older lib.dom typings; used by `cssEscape`'s feature check. */
    CSS?: { escape?: (value: string) => string };
  }
}

/** Stable per-app client id (persisted) so sessions resume across reloads. */
function getClientId(appId: string): string {
  const key = "br.client." + appId;
  try {
    let id = window.localStorage.getItem(key);
    if (!id) {
      id = "c-" + Math.random().toString(36).slice(2) + Date.now().toString(36);
      window.localStorage.setItem(key, id);
    }
    return id;
  } catch {
    // localStorage unavailable (e.g. private mode) → ephemeral id (no resume).
    return "c-" + Math.random().toString(36).slice(2);
  }
}

/**
 * The endpoint the page's own origin implies. `null` under `file://`, where
 * there is no host to derive one from.
 */
function sameOriginEndpoint(appId: string): string | null {
  const loc = window.location;
  if (loc.protocol !== "http:" && loc.protocol !== "https:") return null;
  const proto = loc.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${loc.host}/apps/${appId}/agent`;
}

/**
 * Candidate agent endpoints, most-likely first. Exports used to bake a single
 * absolute `ws://127.0.0.1:3000/...`, which fails whenever the daemon isn't on
 * that port (the desktop app starts it on an ephemeral one) — so we now try the
 * page's own origin first, then any configured fallbacks.
 */
function resolveEndpoints(cfg: AppConfig): string[] {
  const out: string[] = [];
  const add = (e: string | null) => {
    if (e && out.indexOf(e) < 0) out.push(e);
  };
  // An explicit single endpoint always wins (a remote daemon, a test harness).
  if (cfg.endpoint) add(cfg.endpoint);
  else add(sameOriginEndpoint(cfg.appId));
  for (const e of cfg.endpoints || []) add(e);
  if (!cfg.endpoint) add(sameOriginEndpoint(cfg.appId));
  return out;
}

function withClientId(base: string, appId: string, token?: string): string {
  const cid = encodeURIComponent(getClientId(appId));
  const sep = base.indexOf("?") >= 0 ? "&" : "?";
  let url = `${base}${sep}client_id=${cid}`;
  // SDK v2 socket authority: carry the per-app token when the served page
  // supplied one, so the daemon can authorize the upgrade (same-origin only).
  if (token) url += `&token=${encodeURIComponent(token)}`;
  return url;
}

/** Open one WebSocket, resolving on `open` and rejecting on the first error. */
function openSocket(url: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    let ws: WebSocket;
    try {
      ws = new WebSocket(url);
    } catch (e) {
      reject(e as Error);
      return;
    }
    const onOpen = () => {
      ws.removeEventListener("error", onErr);
      resolve(ws);
    };
    const onErr = () => {
      ws.removeEventListener("open", onOpen);
      reject(new Error(`could not connect to ${url}`));
    };
    ws.addEventListener("open", onOpen, { once: true });
    ws.addEventListener("error", onErr, { once: true });
  });
}

/** How many frames may wait for a closed socket before the oldest is dropped. */
const OUTBOX_MAX = 64;

/**
 * Every timeline currently mounted by the app author.
 *
 * `doRun` used to ALWAYS mount a timeline inside its run target, so
 * `br.run(prompt, "#synthesis")` rendered tool-call plumbing into the semantic
 * result region *by construction* — and if the app had also mounted a timeline at
 * `#progress`, the same events rendered twice, in two places. There was no way to
 * say "progress here, result there". Now: if the author mounted a sink, `run`
 * streams progress to it and leaves the result region for the result.
 */
const PROGRESS_SINKS = new Set<HTMLElement>();

/** Whether the app has somewhere of its own to show progress. */
function hasProgressSink(): boolean {
  for (const el of PROGRESS_SINKS) {
    if (el.isConnected) return true;
    PROGRESS_SINKS.delete(el);
  }
  return (
    !!document.querySelector("[data-br-progress]") ||
    !!document.querySelector("[data-br-chat]")
  );
}

/**
 * How long a queued/running `br.run` waits for the server's `done` before it is
 * abandoned and the run queue is drained.
 *
 * Generous: a legitimate multi-agent turn with several consults can take minutes.
 * The point is not to be tight, it is to be FINITE — the queue previously had no
 * liveness at all.
 */
const RUN_STALL_MS = 180_000;

/** Minimum WCAG contrast ratio a visible text element must keep after a restyle. */
const MIN_CONTRAST = 3.0;

/** An RGBA colour. A named object rather than a tuple: the no-esbuild fallback
 *  bundler strips simple type annotations, not tuple types. */
interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** Parse a computed `rgb()` / `rgba()` colour. Returns null for anything else. */
function parseRgb(value: string): Rgba | null {
  const m = /rgba?\(([^)]+)\)/.exec(value);
  if (!m) return null;
  const parts = m[1].split(",").map((p) => parseFloat(p.trim()));
  if (parts.length < 3 || parts.some((n) => Number.isNaN(n))) return null;
  return {
    r: parts[0],
    g: parts[1],
    b: parts[2],
    a: parts.length > 3 ? parts[3] : 1,
  };
}

/** Relative luminance, per WCAG 2.x. */
function luminance(r: number, g: number, b: number): number {
  const f = (c: number) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function contrastRatio(fg: Rgba, bg: Rgba): number {
  const l1 = luminance(fg.r, fg.g, fg.b);
  const l2 = luminance(bg.r, bg.g, bg.b);
  const hi = Math.max(l1, l2);
  const lo = Math.min(l1, l2);
  return (hi + 0.05) / (lo + 0.05);
}

/** The nearest ancestor with an opaque background — what the text actually sits on. */
function effectiveBackground(el: Element): Rgba | null {
  let node: Element | null = el;
  while (node) {
    const bg = parseRgb(getComputedStyle(node).backgroundColor);
    if (bg && bg.a > 0.5) return bg;
    node = node.parentElement;
  }
  const body = parseRgb(getComputedStyle(document.body).backgroundColor);
  return body && body.a > 0.5 ? body : null;
}

/**
 * Find text the current theme has made unreadable.
 *
 * `ui_theme` reported success no matter what it did to the page — and what it did
 * depended on CSS the agent itself had written in an earlier turn, so it could not
 * have known. An app whose page background is a hardcoded `#fff` while its cards
 * use `var(--br-surface)` produces exactly the reported artifact after a dark pack
 * lands: black blocks on a white page.
 *
 * Deliberately conservative. A false positive would revert a perfectly good theme
 * and teach the agent that theming does not work, which is worse than the bug: so
 * only VISIBLE elements with actual text and a resolvable opaque background are
 * checked, and the threshold is 3.0 (large-text AA), not 4.5.
 */
function auditContrast(): string[] {
  const offenders: string[] = [];
  const candidates = document.querySelectorAll<HTMLElement>(
    "[data-br-region], [data-br-region] *, .br-card, .br-card *, [data-br-bind]"
  );

  for (const el of candidates) {
    if (offenders.length >= 20) break;
    const text = (el.textContent || "").trim();
    if (!text) continue;
    // Only leaf-ish text: a container's textContent is its children's.
    if (el.children.length > 0) continue;

    const style = getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden") continue;
    if (parseFloat(style.opacity || "1") < 0.1) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) continue;

    const fg = parseRgb(style.color);
    const bg = effectiveBackground(el);
    if (!fg || !bg || fg.a < 0.5) continue;

    if (contrastRatio(fg, bg) < MIN_CONTRAST) {
      const label = el.tagName.toLowerCase() + (el.id ? `#${el.id}` : "");
      offenders.push(`${label}: "${text.slice(0, 24)}"`);
    }
  }

  return offenders;
}

/**
 * Render a run failure where the user can actually see it.
 *
 * A `br.run` whose target was missing rejected a promise that generated click
 * handlers never await — so the click did nothing, said nothing, and left no trace
 * in the session. This is the floor: whatever else fails, the user is told.
 */
function mountRunError(message: string): void {
  const host =
    document.querySelector<HTMLElement>("[data-br-chat]") ||
    document.querySelector<HTMLElement>(".br-output") ||
    document.body;
  const card = document.createElement("div");
  card.className = "br-run-failed";
  card.setAttribute("data-br-run-error", "1");
  card.setAttribute("role", "alert");
  card.textContent = message;
  host.appendChild(card);
}

export class BioRouterClient {
  readonly config: AppConfig;
  private ws: WebSocket | null = null;
  private readyPromise: Promise<void> | null = null;
  // Lazily-populated so new/unknown event kinds (BRSDK v2, future frames) work
  // without enumerating every key — `on()` seeds buckets on demand.
  private listeners: Record<string, Listener[]> = {};
  // Frame types we have already warned about (see `noteUnknownFrame`).
  private unknownKinds: Record<string, boolean> = {};
  // Capabilities advertised by the server in the `ready` frame (deny-by-default).
  private capabilities: string[] = [];
  // Durable-session info latched from the `ready` frame.
  sessionId: string | null = null;
  resumed = false;
  private activeResolve: ResolveFn | null = null;
  private activeReject: RejectFn | null = null;
  private tokensWaiters: Array<(u: { used: number; limit: number; ratio: number }) => void> = [];
  private historyWaiters: Array<(m: Array<{ role: string; text: string }>) => void> = [];
  // Last widget tree the server sent for each widget id.
  private widgetStore: Map<string, WidgetNode> = new Map();
  /** The agent-driven UI runtime: applies `ui` frames to the page. */
  readonly ui: UiRuntime;
  /** The endpoint that actually connected (useful in diagnostics). */
  activeEndpoint: string | null = null;
  // ── SDK v2 Phase 3 ──
  // Author-registered action handlers (`br.actions`), invoked by `app_call`.
  private actionHandlers: Map<string, ActionHandler> = new Map();
  // `br.call` turns awaiting resolution, keyed by callId, plus the currently
  // streaming turn (message deltas / done / error attach to it) and the
  // in-flight *superseding* call per call-key (so a newer one can cancel it).
  private pendingCalls: Map<string, PendingCall> = new Map();
  private callInFlight: Map<string, PendingCall> = new Map();
  private callDebounce: Map<string, CallDebounceRec> = new Map();
  private activeCall: PendingCall | null = null;
  private callSeq = 0;
  // `br.signals`: client-side trailing-edge coalescing + the `ready`-advertised
  // surface (signal names + coalesce windows, and the action names the agent
  // may `app_call`).
  private signalPending: Map<string, SignalRec> = new Map();
  /** Frames that must survive a closed socket (see `sendReliable`). */
  private outbox: unknown[] = [];
  private signalDeclMap: Map<string, SignalDecl> = new Map();
  private declaredSignals: SignalDecl[] = [];
  private declaredActions: string[] = [];
  // In-flight `br.run` (for supersede) + its trailing-edge debounce slots.
  private activeRun: RunHandle | null = null;
  private runDebounce: Map<string, RunDebounceRec> = new Map();
  // ── SDK v2 Phase 4 ──
  // In-flight `br.kb` requests, keyed by reqId, plus a monotonic reqId counter
  // and a latch for whether a `ready` frame has been seen (so the KB grant
  // pre-check only fires once the capability vocabulary is known).
  private pendingKb: Map<string, PendingKb> = new Map();
  private reqSeq = 0;
  private readySeen = false;
  // Callers parked on `br.model.status()` until the reply (or a 10s timeout).
  private modelStatusWaiters: PendingModelStatus[] = [];
  // ── SDK v2 Phase 6: ui_error feedback loop ──
  // Send-times of recent `ui_error` frames (rolling 30s window) + a count of
  // errors dropped since the last delivered frame (rides the next one).
  private uiErrorTimes: number[] = [];
  private uiErrorDropped = 0;
  // ── SDK v2 §3.8: multi-agent facade ──
  // Worker profiles the server advertised in `ready.profiles` (empty = none).
  private declaredAgents: string[] = [];
  // In-flight worker prompts + calls, keyed by profile, so a worker frame's
  // `done`/`error` settles the right facade's promise (the main, no-agent path
  // keeps using `activeResolve`/`activeReject`/`activeCall`).
  private agentInflight: Map<string, AgentInflight> = new Map();
  private agentActiveCall: Map<string, PendingCall> = new Map();
  // Facade-scoped listeners: agent name (or "" for the main facade) → per-kind
  // buckets. Global `on()` still fires for ALL frames (back-compat); this adds
  // the filtered routing so `br.agent(x).on()` sees only x's frames.
  private agentListeners: Map<string, Record<string, Listener[]>> = new Map();

  constructor(config: AppConfig) {
    this.config = config;
    this.ui = new UiRuntime(this);
  }

  /** Register a listener for an agent event. Returns `this` for chaining. */
  on(kind: EventKind, fn: Listener): this {
    (this.listeners[kind] ||= []).push(fn);
    return this;
  }

  private emit(ev: AgentEvent): void {
    this.fire(this.listeners[ev.type], ev);
    // Wildcard listeners see EVERY frame, including one whose `type` no per-kind
    // bucket can ever match because this SDK has never heard of it.
    this.fire(this.listeners[ANY_EVENT], ev);
  }

  private fire(bucket: Listener[] | undefined, ev: AgentEvent): void {
    for (const fn of bucket || []) {
      try {
        fn(ev);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  }

  /** Warn once per unrecognized frame type. Once, because a daemon that sends
   *  one such frame usually sends many, and a warning per frame is noise nobody
   *  reads. */
  private noteUnknownFrame(kind: string): void {
    if (this.unknownKinds[kind]) return;
    this.unknownKinds[kind] = true;
    try {
      // ⚠ The "rebuild" half applies to an EXPORTED standalone app only. An app
      // served by the daemon is rebuilt on SDK drift before its index is
      // returned (`routes/apps.rs::serve_index` → `bundle_is_stale` →
      // `rebuild_and_stamp`), so its vendored sdk.ts can only lag the daemon
      // when that build FAILED — which the daemon logs as "on-demand build
      // failed". An exported copy has no such rebuild and is the case that
      // really does need re-exporting.
      console.warn(
        "[Biorouter] unrecognized agent frame: " +
          kind +
          " — this app's vendored sdk.ts is older than the daemon. If this is an exported " +
          "standalone app, re-export it; if it is served by Biorouter, check the daemon log " +
          "for a failed on-demand build."
      );
    } catch {
      /* a console-less host must not break the socket */
    }
  }

  /**
   * Open (or reuse) the WebSocket to the Biorouter agent backend, trying each
   * candidate endpoint in turn. Rejects with an actionable message listing what
   * was tried, rather than a bare "could not reach the backend".
   */
  connect(): Promise<void> {
    if (this.readyPromise) return this.readyPromise;
    this.readyPromise = this.dial().catch((e: Error) => {
      // Let a later call retry (e.g. after the user starts biorouterd).
      this.readyPromise = null;
      throw e;
    });
    return this.readyPromise;
  }

  private async dial(): Promise<void> {
    const candidates = resolveEndpoints(this.config);
    if (!candidates.length) {
      throw new Error(
        "No Biorouter endpoint to connect to. This page was opened from a file:// URL " +
          "with no fallback configured — serve it with `npm start` instead."
      );
    }
    const tried: string[] = [];
    for (const base of candidates) {
      const url = withClientId(base, this.config.appId, this.config.wsToken);
      try {
        const ws = await openSocket(url);
        this.ws = ws;
        this.activeEndpoint = base;
        // Deliver anything the app emitted while we were disconnected — a signal
        // fired during page load used to be dropped on the floor here.
        this.flushOutbox();
        // We're connected — clear any stale "no backend" banner a prior failed
        // attempt (or an earlier reconnect) may have left on the page. A working
        // app must never show a backend-unreachable error.
        clearBackendError();
        ws.onerror = () => {
          this.emit({ type: "error", message: "connection error" });
          const err = new Error("connection error");
          this.settleActive(err);
          this.rejectAllAgents(err);
        };
        ws.onclose = () => {
          this.readyPromise = null;
          this.ws = null;
          const err = new Error("connection closed");
          this.settleActive(err);
          this.rejectAllAgents(err);
        };
        ws.onmessage = (ev) => this.handleFrame(ev.data);
        return;
      } catch {
        tried.push(base);
      }
    }
    throw new Error(
      "Could not reach the Biorouter backend. Tried: " +
        tried.join(", ") +
        ". Start it with `biorouterd agent`, or run this app's `run.sh` / `npm start`, " +
        "which starts one for you."
    );
  }

  private settleActive(err?: Error): void {
    if (err && this.activeReject) {
      const rej = this.activeReject;
      this.activeResolve = null;
      this.activeReject = null;
      rej(err);
    } else if (!err && this.activeResolve) {
      const res = this.activeResolve;
      this.activeResolve = null;
      this.activeReject = null;
      res();
    }
  }

  private handleFrame(raw: string): void {
    let msg: AgentEvent;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (msg == null || typeof msg.type !== "string") return;
    // Latch advertised capabilities + durable-session info from `ready`.
    if (msg.type === "ready") {
      this.readySeen = true;
      if (Array.isArray(msg.capabilities)) this.capabilities = msg.capabilities;
      if (typeof msg.sessionId === "string") this.sessionId = msg.sessionId;
      this.resumed = msg.resumed === true;
      // Latch the declared worker profiles (§3.8). Absent/empty = none.
      const profiles = msg.profiles;
      if (Array.isArray(profiles)) this.declaredAgents = profiles.slice();
      // Latch the advertised signal/action surface (absent on pre-Phase-3
      // servers — tolerated, falling back to the 250 ms default coalescing).
      this.latchSurface(msg.surface);
      // Tell the agent what this page offers it (regions, ids) so `ui_describe`
      // returns something real and `ui_render` can target the author's markup.
      if (this.has("ui")) this.ui.reportSurface();
    }
    // Agent-driven UI: apply the command to the page. An `app_call` rides the
    // `ui` frame but is not a DOM command — dispatch it to `br.actions` instead.
    if (msg.type === "ui") {
      if (msg.cmd === "app_call") this.dispatchAction(msg);
      else this.ui.apply(msg);
    }
    // Structured result for a pending `br.call` (matched by callId).
    if (msg.type === "output") this.resolveOutput(msg);
    // Accumulate a `br.call` turn's prose so a turn that ends without an
    // `output` frame still resolves with its `{text}`. A worker frame carries
    // `agent` and feeds that profile's in-flight call; a main frame feeds
    // `activeCall` (per-agent turn-text accumulation, §3.8).
    if (msg.type === "message" && typeof msg.delta === "string") {
      const agent = frameAgent(msg);
      const pc = agent ? this.agentActiveCall.get(agent) : this.activeCall;
      if (pc) pc.textBuf += msg.delta;
      // Presence attribution: a worker's message start shows "<profile> · …".
      if (agent && this.has("ui")) this.ui.presence(agent + " · responding…");
    }
    // Resolve any pending br.context.tokens() callers.
    if (msg.type === "context") {
      const waiters = this.tokensWaiters;
      this.tokensWaiters = [];
      const u = { used: msg.used || 0, limit: msg.limit || 0, ratio: msg.ratio || 0 };
      for (const w of waiters) {
        try {
          w(u);
        } catch {
          /* ignore */
        }
      }
    }
    // Cache server-emitted widget trees so apps can re-render / look them up.
    if (msg.type === "widget" && typeof msg.id === "string") {
      this.widgetStore.set(msg.id, msg.tree as WidgetNode);
    }
    // ── SDK v2 Phase 4 ── resolve pending br.kb / br.model.status callers.
    if (msg.type === "kb_result") this.resolveKb(msg);
    if (msg.type === "kb_progress") this.progressKb(msg);
    if (msg.type === "model_status") this.resolveModelStatus(msg);
    // Resolve any pending history() callers.
    if (msg.type === "history") {
      const waiters = this.historyWaiters;
      this.historyWaiters = [];
      const m = Array.isArray(msg.messages) ? msg.messages : [];
      for (const w of waiters) {
        try {
          w(m);
        } catch {
          /* ignore */
        }
      }
    }
    // ── A top-level frame this SDK version does not understand ──────────────
    // ⚠ NOT a default arm, and not a residue of the branches above. `handleFrame`
    // is a sequence of INDEPENDENT `if`s, and most known kinds — `approval`,
    // `approval_refused`, `capability_report`, `tool`, `usage`, `guardrail`, … —
    // have no branch here at all; they are delivered by `emit` to listeners and
    // to the timeline. So this is an independent lookup into a hand-maintained
    // list (`KNOWN_EVENT_KINDS_MAP`), and the list is what decides, not control
    // flow. Before it, an unknown frame fell through to `emit`, found an empty
    // listener bucket and vanished — the silence that made `approval_refused`
    // look like a hang (issue #143). Warn once here, and let the wildcard
    // listeners draw it (`eventSummary`'s own default arm turns it into a
    // visible "Unrecognized event" row).
    if (!isKnownEventKind(msg.type)) this.noteUnknownFrame(msg.type);
    // Global listeners fire for ALL frames (back-compat); the facade adds the
    // filtered routing so `br.agent(x).on()` sees only x's frames (§3.8).
    this.emit(msg);
    this.routeFacade(msg);
    // Settle the correct facade's in-flight promise by the frame's `agent`.
    if (msg.type === "done") {
      // A turn where the workers produced nothing must not look like a turn that
      // did the work. The main agent used to get a soft "did not answer" text,
      // ignore it, and complete normally — the page showed a finished turn with no
      // sign that half its reasoning never happened. The server marks the frame; we
      // show it, whether or not the model mentions it.
      if (msg.degraded) {
        showDegradedBanner(msg.missingProfiles || []);
      }
      const agent = frameAgent(msg);
      if (agent) {
        this.settleAgentCall(agent, null);
        this.settleAgentPrompt(agent);
      } else {
        this.settleActiveCall(null);
        this.settleActive();
      }
    } else if (msg.type === "error") {
      const err = new Error(msg.message);
      const agent = frameAgent(msg);
      if (agent) {
        this.settleAgentCall(agent, err);
        this.settleAgentPrompt(agent, err);
      } else {
        this.settleActiveCall(err);
        this.settleActive(err);
      }
    }
  }

  // ── SDK v2 §3.8: multi-agent facade ─────────────────────────────────────────

  /** The worker profiles the server declared in its `ready` frame (a copy). */
  agents(): string[] {
    return this.declaredAgents.slice();
  }

  /** A scoped handle for a declared worker profile: `{prompt, ask, call, run, on}`
   *  where every outgoing prompt/call carries `agent:name`, and `on()` fires only
   *  for frames whose `agent === name`. An unknown profile (not in
   *  `ready.profiles`) makes the async methods reject immediately with a clear
   *  error; `on()` is a harmless no-op (no frame will ever match). */
  agent(name: string): AgentFacade {
    const self = this;
    const facade: AgentFacade = {
      prompt: (text, opts) => self.agentReject(name) || self.prompt(text, self.withAgent(opts, name)),
      ask: (text, opts) => self.agentReject(name) || self.ask(text, self.withAgent(opts, name)),
      call: (nameOrOpts, args, opts) => self.agentReject(name) || self.callWithAgent(name, nameOrOpts, args, opts),
      run: (text, target, opts) => self.agentReject(name) || self.run(text, target, self.withAgent(opts, name)),
      on: (kind, fn) => {
        self.addAgentListener(name, kind, fn);
        return facade;
      },
    };
    return facade;
  }

  /** A rejected promise when `name` is not a declared profile, else `null` so the
   *  caller falls through to the real method. */
  private agentReject(name: string): Promise<never> | null {
    if (this.declaredAgents.indexOf(name) >= 0) return null;
    const known = this.declaredAgents.join(", ") || "(none)";
    return Promise.reject(
      new Error('Unknown agent profile "' + name + '". Declared profiles: ' + known + ".")
    );
  }

  /** Clone `opts` with `agent` stamped on, for a facade-scoped prompt/run. */
  private withAgent(opts: PromptOptions | undefined, name: string): PromptOptions {
    const base = opts || {};
    return { images: base.images, debounceMs: base.debounceMs, supersede: base.supersede, agent: name };
  }

  /** Facade-scoped `call`: normalise, stamp the agent, and schedule. */
  private callWithAgent(
    name: string,
    nameOrOpts: string | CallOpts,
    args: unknown,
    opts?: CallOpts
  ): Promise<CallResult> {
    const o = this.normalizeCall(nameOrOpts, args, opts);
    o.agent = name;
    return new Promise((resolve, reject) => {
      this.scheduleCall(o, resolve, reject);
    });
  }

  /** Register a facade-scoped listener (agent name, or "" for the main facade). */
  private addAgentListener(name: string, kind: EventKind, fn: Listener): void {
    let buckets = this.agentListeners.get(name);
    if (!buckets) {
      buckets = {};
      this.agentListeners.set(name, buckets);
    }
    (buckets[kind] ||= []).push(fn);
  }

  /** Dispatch a frame to the facade whose name matches its `agent` (or "" for a
   *  main frame). Errors in a facade listener are non-fatal. */
  private routeFacade(msg: AgentEvent): void {
    const buckets = this.agentListeners.get(frameAgent(msg));
    if (!buckets) return;
    const arr = buckets[msg.type];
    if (!arr) return;
    for (const fn of arr) {
      try {
        fn(msg);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  }

  /** Settle a worker profile's in-flight prompt (its `done`/`error`). */
  private settleAgentPrompt(name: string, err?: Error): void {
    const rec = this.agentInflight.get(name);
    if (!rec) return;
    this.agentInflight.delete(name);
    if (err) rec.reject(err);
    else rec.resolve();
  }

  /** Settle a worker profile's in-flight `br.call` (resolve `{text}` on done). */
  private settleAgentCall(name: string, err: Error | null): void {
    const pending = this.agentActiveCall.get(name);
    if (!pending) return;
    if (err) this.rejectCall(pending, err);
    else this.resolveCall(pending, { text: pending.textBuf });
  }

  /** Reject every in-flight worker prompt/call (on socket close/error). */
  private rejectAllAgents(err: Error): void {
    const prompts = Array.from(this.agentInflight.values());
    this.agentInflight.clear();
    for (const rec of prompts) {
      try {
        rec.reject(err);
      } catch {
        /* consumer errors are non-fatal */
      }
    }
    const calls = Array.from(this.agentActiveCall.values());
    for (const pc of calls) this.rejectCall(pc, err);
  }

  /**
   * Fetch the user-visible message backlog for the current (resumed) session so
   * a reloaded app can repaint its chat history. Returns `[]` when there is no
   * session yet or the request fails.
   */
  history(): Promise<Array<{ role: string; text: string }>> {
    return new Promise((resolve) => {
      this.historyWaiters.push(resolve);
      if (!this.send({ type: "history" })) {
        this.historyWaiters = this.historyWaiters.filter((w) => w !== resolve);
        resolve([]);
      }
    });
  }

  /** Current context-window usage (BRSDK context API). */
  tokens(): Promise<{ used: number; limit: number; ratio: number }> {
    return new Promise((resolve) => {
      this.tokensWaiters.push(resolve);
      if (!this.send({ type: "tokens" })) {
        // Socket not open — resolve with zeros rather than hang.
        this.tokensWaiters = this.tokensWaiters.filter((w) => w !== resolve);
        resolve({ used: 0, limit: 0, ratio: 0 });
      }
    });
  }

  /** Namespaced context API: `br.context.tokens()` / `br.context.history()`. */
  get context() {
    return {
      tokens: () => this.tokens(),
      history: () => this.history(),
    };
  }

  /**
   * The HTTP origin of the daemon we are actually talking to. Derived from the
   * connected WebSocket endpoint, NOT from `window.location` — an exported app
   * can be served from one origin while its agent lives on another, and using
   * the page origin silently 404s every REST call.
   */
  private httpBase(): string {
    const ep = this.activeEndpoint;
    const appPath = `/apps/${encodeURIComponent(this.config.appId)}`;
    if (ep) {
      const u = new URL(ep);
      const proto = u.protocol === "wss:" ? "https:" : "http:";
      return `${proto}//${u.host}${appPath}`;
    }
    const loc = window.location;
    return `${loc.protocol}//${loc.host}${appPath}`;
  }

  /** Provider/model catalog the user has available (the provider-agnostic
   *  headline). Returns `[]` on failure. */
  async listModels(): Promise<unknown[]> {
    try {
      await this.connect();
      const res = await fetch(`${this.httpBase()}/models`);
      if (!res.ok) return [];
      const data = await res.json();
      return Array.isArray(data.providers) ? data.providers : [];
    } catch {
      return [];
    }
  }

  /** Live-switch the session's provider/model. */
  selectModel(provider: string, model: string): void {
    this.send({ type: "modelselect", provider, model });
  }

  /** Namespaced model surface: `br.model.list()` / `br.model.select(p, m)` /
   *  `br.model.status()` (is the routed provider ready — e.g. is llamacpp still
   *  downloading, what context size). */
  get model() {
    return {
      list: () => this.listModels(),
      select: (provider: string, model: string) => this.selectModel(provider, model),
      status: (timeoutMs?: number) =>
        this.requestModelStatus(typeof timeoutMs === "number" && timeoutMs > 0 ? timeoutMs : 10000),
    };
  }

  /** Namespaced knowledge-base surface (`br.kb`). Every method sends a `kb`
   *  frame with a fresh reqId and resolves/rejects on the matching `kb_result`
   *  (`ingest` additionally streams `kb_progress` to `onProgress`). Access is
   *  gated by the app's `data.sources[kind:"knowledge"]` grant: when the `ready`
   *  frame advertises no KB-family capability token, the read/write methods
   *  reject immediately with a clear "no knowledge-base grant" error rather than
   *  round-tripping. `graphToNetwork` is a pure client-side convenience that maps
   *  a `graph()` result straight into the `network` component's spec shape. */
  get kb() {
    return {
      search: (query: string, opts?: KbSearchOpts) => this.kbSearch(query, opts),
      page: (path: string, opts?: KbCallOpts) => this.kbRequest("page", { path: path }, opts),
      graph: (opts?: KbCallOpts) => this.kbRequest("graph", {}, opts),
      history: (limit?: number, opts?: KbCallOpts) =>
        this.kbRequest("history", typeof limit === "number" ? { limit: limit } : {}, opts),
      ingest: (items: unknown, opts?: KbIngestOpts) => this.kbIngest(items, opts),
      graphToNetwork: (graph: unknown) => graphToNetwork(graph),
    };
  }

  // ── SDK v2 Phase 4: br.kb request plumbing ──────────────────────────────────

  private kbSearch(query: string, opts?: KbSearchOpts): Promise<KbResult> {
    const o = opts || {};
    const q = typeof query === "string" ? query : "";
    const params: KbSearchParams = { query: q };
    if (typeof o.limit === "number") params.limit = o.limit;
    return this.kbRequest("search", params, o);
  }

  private kbRequest(op: string, params: unknown, opts?: KbCallOpts): Promise<KbResult> {
    const o = opts || {};
    const timeoutMs = typeof o.timeoutMs === "number" && o.timeoutMs > 0 ? o.timeoutMs : 30000;
    if (this.kbGrantMissing()) return Promise.reject(this.kbGrantError());
    return new Promise((resolve, reject) => {
      this.startKb(op, params, timeoutMs, undefined, resolve, reject);
    });
  }

  private kbIngest(items: unknown, opts?: KbIngestOpts): Promise<KbResult> {
    const o = opts || {};
    const timeoutMs = typeof o.timeoutMs === "number" && o.timeoutMs > 0 ? o.timeoutMs : 600000;
    if (this.kbGrantMissing()) return Promise.reject(this.kbGrantError());
    return new Promise((resolve, reject) => {
      this.startKb("ingest", { items: items }, timeoutMs, o.onProgress, resolve, reject);
    });
  }

  /** True only when a `ready` frame has been latched AND none of the KB-family
   *  capability tokens is advertised — the one case where a graceful pre-reject
   *  beats a round-trip. Before `ready` (vocabulary unknown) we attempt the call
   *  and let the server answer, per the design's "tolerate absence" rule. */
  private kbGrantMissing(): boolean {
    if (!this.readySeen) return false;
    const kbTokens = ["data:knowledge", "data", "kb", "knowledge"];
    for (const t of kbTokens) {
      if (this.has(t)) return false;
    }
    return true;
  }

  private kbGrantError(): Error {
    return new Error(
      'br.kb: this app has no knowledge-base grant — declare a data.sources[kind:"knowledge"] ' +
        "capability in the manifest to read or ingest knowledge bases."
    );
  }

  private startKb(
    op: string,
    params: unknown,
    timeoutMs: number,
    onProgress: KbProgressFn | undefined,
    resolve: KbResolve,
    reject: RejectFn
  ): void {
    const reqId = this.freshReqId();
    const timer = setTimeout(() => {
      const p = this.pendingKb.get(reqId);
      if (p) this.rejectKb(p, new Error("br.kb " + op + " timed out after " + timeoutMs + "ms"));
    }, timeoutMs);
    const pending: PendingKb = {
      reqId: reqId,
      op: op,
      resolve: resolve,
      reject: reject,
      onProgress: onProgress,
      timer: timer,
      settled: false,
    };
    this.pendingKb.set(reqId, pending);
    const frame = { type: "kb", op: op, params: params, reqId: reqId };
    if (this.send(frame)) return;
    // Socket not open yet — connect, then send (or reject if still unreachable).
    this.connect().then(
      () => {
        if (!this.send(frame)) this.rejectKb(pending, new Error("Not connected to the Biorouter backend."));
      },
      (e: Error) => this.rejectKb(pending, e)
    );
  }

  private freshReqId(): string {
    this.reqSeq++;
    return "req-" + this.reqSeq + "-" + Date.now().toString(36);
  }

  private resolveKb(msg: AgentEvent): void {
    const frame = msg as KbResultFrame;
    const reqId = typeof frame.reqId === "string" ? frame.reqId : "";
    const pending = reqId ? this.pendingKb.get(reqId) : undefined;
    if (!pending) return;
    if (typeof frame.error === "string" && frame.error) this.rejectKb(pending, new Error(frame.error));
    else this.settleKb(pending, frame.result);
  }

  private progressKb(msg: AgentEvent): void {
    const frame = msg as KbProgressFrame;
    const reqId = typeof frame.reqId === "string" ? frame.reqId : "";
    const pending = reqId ? this.pendingKb.get(reqId) : undefined;
    if (!pending || !pending.onProgress) return;
    try {
      pending.onProgress({
        stage: typeof frame.stage === "string" ? frame.stage : "",
        detail: frame.detail,
        pct: frame.pct,
      });
    } catch {
      /* progress-callback errors are non-fatal */
    }
  }

  private settleKb(pending: PendingKb, result: unknown): void {
    if (pending.settled) return;
    pending.settled = true;
    clearTimeout(pending.timer);
    this.pendingKb.delete(pending.reqId);
    try {
      pending.resolve(result);
    } catch {
      /* consumer errors are non-fatal */
    }
  }

  private rejectKb(pending: PendingKb, err: Error): void {
    if (pending.settled) return;
    pending.settled = true;
    clearTimeout(pending.timer);
    this.pendingKb.delete(pending.reqId);
    try {
      pending.reject(err);
    } catch {
      /* consumer errors are non-fatal */
    }
  }

  // ── SDK v2 Phase 4: br.model.status ─────────────────────────────────────────

  private requestModelStatus(timeoutMs: number): Promise<ModelStatus> {
    return new Promise((resolve, reject) => {
      const rec: PendingModelStatus = { resolve: resolve, reject: reject, timer: 0, settled: false };
      rec.timer = setTimeout(
        () => this.failModelStatus(rec, new Error("br.model.status timed out after " + timeoutMs + "ms")),
        timeoutMs
      );
      this.modelStatusWaiters.push(rec);
      const frame = { type: "model_status" };
      if (this.send(frame)) return;
      this.connect().then(
        () => {
          if (!this.send(frame)) this.failModelStatus(rec, new Error("Not connected to the Biorouter backend."));
        },
        (e: Error) => this.failModelStatus(rec, e)
      );
    });
  }

  private resolveModelStatus(msg: AgentEvent): void {
    const frame = msg as ModelStatusFrame;
    const status: ModelStatus = {
      provider: frame.provider,
      model: frame.model,
      ready: frame.ready === true,
      detail: frame.detail,
    };
    const waiters = this.modelStatusWaiters;
    this.modelStatusWaiters = [];
    for (const w of waiters) {
      if (w.settled) continue;
      w.settled = true;
      clearTimeout(w.timer);
      try {
        w.resolve(status);
      } catch {
        /* consumer errors are non-fatal */
      }
    }
  }

  private failModelStatus(rec: PendingModelStatus, err: Error): void {
    if (rec.settled) return;
    rec.settled = true;
    clearTimeout(rec.timer);
    this.modelStatusWaiters = this.modelStatusWaiters.filter((w) => w !== rec);
    try {
      rec.reject(err);
    } catch {
      /* consumer errors are non-fatal */
    }
  }

  /**
   * Shared reactive state document (`br.state`). The doc is a single JSON value
   * both sides write: the agent via `ui_state`/`ui_patch_state`, author code via
   * these methods. Every mutation re-evaluates declarative `data-br-bind*`
   * bindings and notifies `subscribe`rs. Reads return deep clones so a caller
   * cannot mutate the live doc out from under the binding layer.
   *
   * - `get(path?)` — the value at a JSON Pointer, or the whole doc when omitted.
   * - `set(path, value)` — optimistically apply locally, then send a
   *   `state_write` carrying the pre-write `baseVersion` (server is the ordering
   *   authority; on conflict it replies with a fresh snapshot we simply apply).
   * - `remove(path)` — delete the value at a pointer, same version discipline.
   * - `update(fn)` — replace the whole doc with `fn(clone)`'s return value.
   * - `subscribe(path, fn)` — fire `fn(value)` whenever the value at `path`
   *   changes (compared by JSON.stringify); returns an unsubscribe function.
   */
  get state() {
    return {
      get: (path?: string) => this.ui.stateGet(path),
      set: (path: string, value: unknown) => this.ui.stateSet(path, value),
      remove: (path: string) => this.ui.stateRemove(path),
      update: (fn: DocUpdater) => this.ui.stateUpdate(fn),
      subscribe: (path: string, fn: ValueSub) => this.ui.stateSubscribe(path, fn),
    };
  }

  /** Author component registry (`br.components.register(name, def)`). A
   *  `component` node renders a container and calls the registered `mount`;
   *  `set_props`/`replace` call `update` if present, else remount. Props are
   *  agent-controlled — treat them as untrusted input. */
  get components() {
    return {
      register: (name: string, def: ComponentDef) => this.ui.registerComponent(name, def),
    };
  }

  /** HITL: approve a pending tool surfaced via an `approval` event.
   *  `action` is "allow_once" (default) or "always_allow". */
  approve(requestId: string, action: string = "allow_once"): void {
    this.send({ type: "approve", request: requestId, action });
  }

  /** HITL: reject a pending tool, with an optional human-readable reason. */
  reject(requestId: string, reason?: string): void {
    this.send({ type: "reject", request: requestId, reason });
  }

  /** Render a widget tree into `target` and wire its actions back to the agent
   *  (a submit button sends a `widget_action` frame scoped to this widget id). */
  private renderWidgetInto(
    id: string,
    tree: WidgetNode,
    target: HTMLElement | string
  ): HTMLElement {
    const host =
      typeof target === "string"
        ? (document.querySelector(target) as HTMLElement | null)
        : target;
    const ctx: WidgetContext = {
      fields: new Map(),
      onAction: (action, payload) =>
        this.send({ type: "widget_action", widgetId: id, action, payload }),
    };
    const dom = renderWidget(tree, ctx);
    if (host) {
      host.innerHTML = "";
      host.appendChild(dom);
    }
    return dom;
  }

  /** Interactive widgets API: render an agent-emitted tree, fire an action, or
   *  look up the last tree the server sent for an id. */
  get widgets() {
    return {
      render: (id: string, tree: WidgetNode, target: HTMLElement | string) =>
        this.renderWidgetInto(id, tree, target),
      action: (widgetId: string, action: string, payload?: unknown) =>
        this.send({ type: "widget_action", widgetId, action, payload }),
      // alias kept for symmetry with the agent-side naming
      submit: (widgetId: string, action: string, payload?: unknown) =>
        this.send({ type: "widget_action", widgetId, action, payload }),
      get: (id: string) => this.widgetStore.get(id),
    };
  }

  /** Whether the server advertised a given BRSDK capability in `ready`. */
  has(capability: string): boolean {
    return this.capabilities.indexOf(capability) >= 0;
  }

  /** Send a raw client frame. Public so the UI runtime can answer `ui_ask`. */
  sendRaw(frame: unknown): boolean {
    return this.send(frame);
  }

  /** Report a catalog render / component / action-handler error to the agent as
   *  a structured `ui_error` frame, so it can repair its own UI. Rate-limited to
   *  3 frames per rolling 30 s; beyond that, errors are dropped and counted, and
   *  the count rides the next delivered frame as `droppedCount`. Never throws —
   *  an error in the error path must never break the socket or the runtime.
   *
   *  `where` is `widget:<kind>` | `component:<name>` | `action:<name>`; `message`
   *  is `String(err)` capped at 500 chars; `instance` (optional) is the id. */
  reportUiError(where: string, message: string, instance?: string): void {
    try {
      const now = Date.now();
      const cutoff = now - 30000;
      const kept: number[] = [];
      for (const t of this.uiErrorTimes) {
        if (t > cutoff) kept.push(t);
      }
      this.uiErrorTimes = kept;
      if (this.uiErrorTimes.length >= 3) {
        this.uiErrorDropped++;
        return;
      }
      this.uiErrorTimes.push(now);
      const dropped = this.uiErrorDropped;
      this.uiErrorDropped = 0;
      // JSON.stringify drops the `undefined` fields, so absent instance /
      // droppedCount never ride the wire.
      const frame = {
        type: "ui_error",
        where: where,
        message: message,
        instance: instance,
        droppedCount: dropped > 0 ? dropped : undefined,
      };
      this.send(frame);
    } catch {
      /* reporting must never throw */
    }
  }

  /** Send a raw client frame if the socket is open; returns whether it went. */
  private send(frame: unknown): boolean {
    if (this.ws && this.ws.readyState === 1 /* WebSocket.OPEN */) {
      this.ws.send(JSON.stringify(frame));
      return true;
    }
    return false;
  }

  /**
   * Send a frame that must NOT be lost if the socket happens to be closed.
   *
   * `send()` returns false and drops the frame when the socket isn't OPEN, which
   * is exactly what used to happen to the user's first gesture: a signal emitted
   * during page load, or across a reconnect, was fire-and-forgotten into a closed
   * socket and never reached the agent. (Half of why app→agent signals
   * round-tripped 1 time in 12; the server dropped the other half.)
   *
   * Frames queued here are flushed in order on `open`. The queue is bounded — a
   * disconnected page that keeps emitting must not grow without limit — and
   * dropping the oldest is reported rather than silent.
   */
  private sendReliable(frame: unknown): void {
    if (this.send(frame)) return;
    if (this.outbox.length >= OUTBOX_MAX) {
      const dropped = this.outbox.shift();
      this.reportUiError(
        "send",
        `outbound queue is full (${OUTBOX_MAX}); dropped the oldest frame: ` +
          JSON.stringify(dropped).slice(0, 120)
      );
    }
    this.outbox.push(frame);
    // Opportunistically (re)connect so the queue actually drains.
    void this.connect().catch(() => undefined);
  }

  /** Flush anything queued while the socket was closed. Called on `open`. */
  private flushOutbox(): void {
    if (!this.outbox.length) return;
    const pending = this.outbox.slice();
    this.outbox.length = 0;
    for (const frame of pending) {
      if (!this.send(frame)) {
        // Still not open — put the remainder back, preserving order.
        this.outbox.unshift(frame);
        break;
      }
    }
  }

  /**
   * Send a prompt to the agent. Resolves when the agent finishes its turn
   * (`done`); reject on error. Streamed output arrives via `on("message", …)`.
   */
  async prompt(text: string, opts: PromptOptions = {}): Promise<void> {
    await this.connect();
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error("Not connected to the Biorouter backend.");
    }
    const agent = opts.agent;
    return new Promise<void>((resolve, reject) => {
      // A worker prompt settles against its own profile's `done`/`error`; the
      // main (no-agent) prompt keeps using the shared active slots (§3.8).
      if (agent) this.agentInflight.set(agent, { resolve: resolve, reject: reject });
      else {
        this.activeResolve = resolve;
        this.activeReject = reject;
      }
      this.ws!.send(
        JSON.stringify({ type: "prompt", text, images: opts.images || [], agent: agent })
      );
    });
  }

  /** Ask the agent to stop the current turn. */
  cancel(): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: "cancel" }));
    }
  }

  /** Remove a previously-registered listener. */
  off(kind: EventKind, fn: Listener): this {
    const arr = this.listeners[kind];
    if (arr) {
      const i = arr.indexOf(fn);
      if (i >= 0) arr.splice(i, 1);
    }
    return this;
  }

  /** Register a listener that fires for EVERY frame — including one whose type
   *  this SDK version does not know.
   *
   *  ⚠ `on()` is not structurally blind to an unknown kind — it seeds buckets on
   *  demand, so `on("quantum_frame" as EventKind, fn)` really would receive one.
   *  What is impossible is subscribing to a name you do not know YET: a caller
   *  cannot enumerate the frame types a future daemon will grow, so a per-kind
   *  subscription list can never cover them and the frame is dropped in silence.
   *  That is why `mountTimeline` takes ONE wildcard rather than a list of kinds.
   *  Returns `this` for chaining. */
  onAny(fn: Listener): this {
    (this.listeners[ANY_EVENT] ||= []).push(fn);
    return this;
  }

  /** Remove a wildcard listener registered with `onAny`. */
  offAny(fn: Listener): this {
    const arr = this.listeners[ANY_EVENT];
    if (arr) {
      const i = arr.indexOf(fn);
      if (i >= 0) arr.splice(i, 1);
    }
    return this;
  }

  /** Convenience: collect the full reply for one prompt as a single string. */
  async ask(text: string, opts: PromptOptions = {}): Promise<string> {
    let buf = "";
    // A worker ask (opts.agent set) accumulates only that profile's deltas; the
    // main ask accumulates only un-tagged (no-agent) deltas — a no-op filter for
    // single-agent apps, where no frame carries an `agent` at all (§3.8).
    const want = opts.agent || "";
    const onMsg: Listener = (ev) => {
      if (ev.type === "message" && frameAgent(ev) === want) buf += ev.delta;
    };
    this.on("message", onMsg);
    try {
      await this.prompt(text, opts);
    } finally {
      this.off("message", onMsg);
    }
    return buf;
  }

  /**
   * The primary helper for custom UIs: send `text` and stream the agent's
   * markdown (and ```chart blocks / tables) into `target` (an element or CSS
   * selector), showing a spinner and tool activity. Returns the full reply.
   * Wire any control to this — e.g. slider/select `change`, button `click`.
   *
   * Runs are serialized: if a call is still streaming when another starts, the
   * new one waits for it, so rapid control changes never overlap or garble the
   * output (and no request is orphaned).
   */
  run(
    text: string,
    target: HTMLElement | string,
    opts: PromptOptions = {}
  ): Promise<string> {
    const key = typeof target === "string" ? target : "@el";
    // Trailing-edge debounce: within the window, only the last run on this
    // target survives; earlier ones resolve early with "" (no partial yet).
    if (opts.debounceMs && opts.debounceMs > 0) {
      return this.debounceRun(key, text, target, opts);
    }
    return this.startRun(text, target, opts);
  }

  private debounceRun(
    key: string,
    text: string,
    target: HTMLElement | string,
    opts: PromptOptions
  ): Promise<string> {
    return new Promise((resolve, reject) => {
      const prev = this.runDebounce.get(key);
      if (prev) {
        clearTimeout(prev.timer);
        prev.resolve("");
      }
      const timer = setTimeout(() => {
        this.runDebounce.delete(key);
        this.startRun(text, target, opts).then(resolve, reject);
      }, opts.debounceMs);
      this.runDebounce.set(key, { timer: timer, resolve: resolve, reject: reject });
    });
  }

  /** Start a run. Plain runs serialize behind `runChain`. A `supersede` run
   *  cancels the in-flight superseding turn (resolving its promise early with
   *  the partial text streamed so far) so it drains fast, then queues after it —
   *  the single server turn is never doubled up. */
  private startRun(
    text: string,
    target: HTMLElement | string,
    opts: PromptOptions
  ): Promise<string> {
    const superseding = !!opts.supersede;
    if (superseding && this.activeRun && this.activeRun.superseding && !this.activeRun.settled) {
      this.cancel();
      this.activeRun.resolveEarly();
    }
    let userResolve: RunResolve = () => {};
    let userReject: RejectFn = () => {};
    const userPromise = new Promise<string>((res, rej) => {
      userResolve = res;
      userReject = rej;
    });
    const handle: RunHandle = {
      superseding: superseding,
      settled: false,
      partial: "",
      resolveEarly: () => {
        if (handle.settled) return;
        handle.settled = true;
        userResolve(handle.partial);
      },
    };

    // Resolve the target NOW, on the click — not inside the queued closure.
    //
    // `doRun` used to look the target up only when its turn came round, and throw
    // if it was missing. Two silent failures followed: a generated
    // `br.run(prompt, "#log")` whose `#log` had been swallowed by an SDK re-render
    // rejected a promise no click handler awaits (nothing visible, nothing sent);
    // and if the queue was wedged, `doRun` was never entered at all, so the target
    // was never even looked up. Either way the control looked alive and was dead.
    const el =
      typeof target === "string"
        ? document.querySelector<HTMLElement>(target)
        : target;
    if (!el) {
      const where = typeof target === "string" ? target : "(element)";
      const message = `run(): target not found: ${where}`;
      console.error(message);
      this.reportUiError("run", message);
      mountRunError(message);
      const err = new Error(message);
      userReject(err);
      return userPromise;
    }

    // Paint immediately, so a QUEUED run is visible. A control that fires and shows
    // nothing is indistinguishable from a control that is broken.
    if (this.activeRun && !this.activeRun.settled) {
      el.innerHTML =
        '<div class="br-run-status"></div><div class="br-run-answer">' +
        '<span class="br-spinner"></span> Queued — waiting for the current agent run…</div>';
    }

    // The queue advances on the *internal* turn settling (server done), NOT on
    // the user promise — which may resolve early via resolveEarly.
    const internal = this.runChain.then(() =>
      this.launchRun(text, el, opts, handle, userResolve, userReject)
    );
    this.runChain = internal.then(
      () => undefined,
      () => undefined
    );

    // Watchdog. `runChain` is a promise chain with no liveness: a turn that never
    // emits `done` — a blocked `ui_ask`, a consult that outran its deadline, a
    // dropped socket, the runaway-tool loop — left it pending FOREVER, and every
    // subsequent `br.run` sat behind it, unsent and unpainted. Serialization is
    // worth keeping; the deadlock is not.
    const timeoutMs = opts.timeoutMs ?? RUN_STALL_MS;
    const watchdog = setTimeout(() => {
      if (handle.settled) return;
      handle.settled = true;
      const message =
        "The agent run did not finish and was abandoned. The app is responsive again — try once more.";
      this.reportUiError("run", "run-stalled: no `done` frame within " + timeoutMs + "ms");
      try {
        el.innerHTML = '<div class="br-run-answer br-run-failed"></div>';
        const answer = el.querySelector<HTMLElement>(".br-run-answer");
        if (answer) answer.textContent = message;
      } catch {
        /* the target may have been removed; the rejection below still lands */
      }
      // Drain the chain so the NEXT run is not stuck behind a turn that never ends.
      this.runChain = Promise.resolve();
      if (this.activeRun === handle) this.activeRun = null;
      userReject(new Error("run-stalled"));
    }, timeoutMs);
    void internal.finally(() => clearTimeout(watchdog));

    return userPromise;
  }

  private async launchRun(
    text: string,
    target: HTMLElement | string,
    opts: PromptOptions,
    handle: RunHandle,
    userResolve: RunResolve,
    userReject: RejectFn
  ): Promise<void> {
    this.activeRun = handle;
    try {
      const full = await this.doRun(text, target, opts, handle);
      if (!handle.settled) {
        handle.settled = true;
        userResolve(full);
      }
    } catch (e) {
      if (!handle.settled) {
        handle.settled = true;
        userReject(e as Error);
      }
    } finally {
      if (this.activeRun === handle) this.activeRun = null;
    }
  }

  private runChain: Promise<void> = Promise.resolve();

  private async doRun(
    text: string,
    target: HTMLElement | string,
    opts: PromptOptions = {},
    handle?: RunHandle
  ): Promise<string> {
    // `startRun` already resolved the target on the click and reported a miss
    // visibly; this re-resolve only covers a direct internal caller.
    const el =
      typeof target === "string"
        ? document.querySelector<HTMLElement>(target)
        : target;
    if (!el) {
      const message = "run(): target element not found";
      this.reportUiError("run", message);
      mountRunError(message);
      throw new Error(message);
    }
    let buf = "";

    // Where does progress go?
    //
    // `run` used to mount a timeline INSIDE the run target unconditionally, so
    // `br.run(prompt, "#synthesis")` wrote tool-call plumbing into the semantic
    // result region by construction — and an app that also mounted its own
    // timeline rendered the same events twice, displacing the science it was
    // supposed to be showing. If the author has a progress surface (an explicit
    // `progress` option, a `mountTimeline`, a `[data-br-progress]`, or a chat
    // panel), the target gets the ANSWER and nothing else.
    const explicitProgress =
      opts.progress === false
        ? null
        : typeof opts.progress === "string"
          ? document.querySelector<HTMLElement>(opts.progress)
          : opts.progress || null;
    const routeAway =
      opts.progress === false || !!explicitProgress || hasProgressSink();

    el.innerHTML = routeAway
      ? '<div class="br-run-answer"><span class="br-spinner"></span> Starting agent run…</div>'
      : '<div class="br-run-status"></div><div class="br-run-answer"><span class="br-spinner"></span> Starting agent run…</div>';
    const statusEl = el.querySelector<HTMLElement>(".br-run-status");
    const answerEl = el.querySelector<HTMLElement>(".br-run-answer")!;

    // Only mount a timeline of our own when the app has nowhere else to show one.
    const timelineHost = explicitProgress || statusEl;
    const stopTimeline =
      opts.progress === false || !timelineHost
        ? () => undefined
        : mountTimeline(this, timelineHost, { maxItems: 18 });
    // A worker run streams only its profile's frames; a main run streams only
    // un-tagged frames (a no-op filter for single-agent apps) (§3.8).
    const want = opts.agent || "";
    const onMsg: Listener = (ev) => {
      if (ev.type === "message" && frameAgent(ev) === want) {
        buf += ev.delta;
        if (handle) handle.partial = buf;
        answerEl.innerHTML = this.renderMarkdown(buf);
      }
    };
    const onTool: Listener = (ev) => {
      if (ev.type === "tool" && frameAgent(ev) === want && !buf) {
        answerEl.innerHTML = `<span class="br-spinner"></span> <span class="br-msg--tool">${ev.name}…</span>`;
      }
    };
    this.on("message", onMsg);
    this.on("tool", onTool);
    try {
      await this.prompt(text, opts);
      if (!buf) answerEl.innerHTML = "";
    } catch (e) {
      // Hoisted out of the template literal so the no-esbuild fallback stripper
      // (which treats `${…}` as part of the string) can strip the `as` cast.
      const emsg = (e as Error).message || "request failed";
      answerEl.innerHTML = `<div class="br-msg br-msg--agent">Failed: ${emsg}</div>`;
      throw e;
    } finally {
      this.off("message", onMsg);
      this.off("tool", onTool);
      stopTimeline();
    }
    return buf;
  }

  /** Minimal, dependency-free markdown → HTML (safe-escaped). */
  renderMarkdown(md: string): string {
    return renderMarkdown(md);
  }

  // ── SDK v2 Phase 3: app.actions ────────────────────────────────────────────

  /** Author-callable action registry. The agent invokes a registered handler by
   *  name via an `app_call` frame; the handler's (awaited) return value is sent
   *  back as `app_result`. A missing handler / a throw becomes an `app_result`
   *  error string, so the agent branches on the failure inside its turn. Results
   *  that serialize to more than 64 KB are truncated inside a
   *  `{truncated:true, text:<prefix>}` wrapper.
   *
   *  - `register(name, handler)` — `handler: async (args) => result`.
   *  - `list()` — the registered action names. */
  get actions() {
    return {
      register: (name: string, handler: ActionHandler) => this.registerAction(name, handler),
      list: () => this.listActions(),
    };
  }

  private registerAction(name: string, handler: ActionHandler): void {
    if (name && typeof handler === "function") this.actionHandlers.set(String(name), handler);
  }

  private listActions(): string[] {
    const out: string[] = [];
    this.actionHandlers.forEach((_v, k) => out.push(k));
    return out;
  }

  /** Dispatch an `app_call` frame to a registered handler and reply with
   *  `app_result`. Never throws — every failure path answers the call. */
  private dispatchAction(cmd: UiCommand): void {
    const callId = typeof cmd.callId === "string" ? cmd.callId : "";
    const action = typeof cmd.action === "string" ? cmd.action : "";
    const handler = this.actionHandlers.get(action);
    if (!handler) {
      this.send({ type: "app_result", callId: callId, error: "no handler registered for " + action });
      return;
    }
    Promise.resolve()
      .then(() => handler(cmd.args))
      .then(
        (result) => {
          const payload = this.serializeActionResult(result);
          this.send({ type: "app_result", callId: callId, result: payload });
        },
        (err) => {
          const emsg =
            err && (err as Error).message ? String((err as Error).message) : "action handler failed";
          this.send({ type: "app_result", callId: callId, error: emsg });
          // Also surface it on the ui_error feedback loop (rate-limited).
          this.reportUiError("action:" + action, errText(err));
        }
      );
  }

  /** Cap an action result at 64 KB of serialized JSON. Over the cap, the JSON is
   *  truncated (with a `…[truncated]` marker) inside a wrapper object so the
   *  agent still receives a well-formed, bounded value. */
  private serializeActionResult(result: unknown): unknown {
    let json: string;
    try {
      json = JSON.stringify(result);
    } catch {
      json = String(result);
    }
    if (json == null) json = "null";
    const limit = 64 * 1024;
    if (json.length > limit) {
      const prefix = json.slice(0, limit) + "…[truncated]";
      return { truncated: true, text: prefix };
    }
    return result;
  }

  // ── SDK v2 Phase 3: br.call (typed turns with structured results) ───────────

  /** Request a typed turn and resolve with a structured result. Pass either an
   *  action `name` + `args`, or free `text`; add `outputSchema` to ask the agent
   *  for a shaped `output`. Resolves `{value}` when an `output` frame with this
   *  call's id arrives, `{text}` when the turn ends without one, `{superseded:true}`
   *  when a newer superseding call replaced it; rejects on the turn's `error`.
   *
   *  Options: `debounceMs` (trailing-edge, per call-key = name||text) and
   *  `supersede` (cancel the in-flight superseding call on this key first). */
  call(nameOrOpts: string | CallOpts, args?: unknown, opts?: CallOpts): Promise<CallResult> {
    const o = this.normalizeCall(nameOrOpts, args, opts);
    return new Promise((resolve, reject) => {
      this.scheduleCall(o, resolve, reject);
    });
  }

  private normalizeCall(nameOrOpts: string | CallOpts, args?: unknown, opts?: CallOpts): CallOpts {
    const extra = opts || {};
    if (typeof nameOrOpts === "string") {
      return {
        name: nameOrOpts,
        args: args,
        text: extra.text,
        outputSchema: extra.outputSchema,
        debounceMs: extra.debounceMs,
        supersede: extra.supersede,
        agent: extra.agent,
      };
    }
    const base = nameOrOpts || {};
    return {
      name: base.name,
      args: base.args,
      text: base.text,
      outputSchema: base.outputSchema,
      debounceMs: base.debounceMs,
      supersede: base.supersede,
      agent: base.agent,
    };
  }

  private callKey(o: CallOpts): string {
    const base = o.name || o.text || "__call";
    // Namespace the debounce/supersede key by profile so a worker's calls never
    // collide with the main agent's on the same name/text (§3.8).
    return o.agent ? o.agent + "::" + base : base;
  }

  private scheduleCall(o: CallOpts, resolve: CallResolve, reject: RejectFn): void {
    const key = this.callKey(o);
    if (o.debounceMs && o.debounceMs > 0) {
      const prev = this.callDebounce.get(key);
      if (prev) {
        clearTimeout(prev.timer);
        prev.resolve({ superseded: true });
      }
      const timer = setTimeout(() => {
        this.callDebounce.delete(key);
        this.dispatchCall(o, key, resolve, reject);
      }, o.debounceMs);
      this.callDebounce.set(key, { timer: timer, resolve: resolve });
      return;
    }
    this.dispatchCall(o, key, resolve, reject);
  }

  private dispatchCall(o: CallOpts, key: string, resolve: CallResolve, reject: RejectFn): void {
    // Supersede: cancel the in-flight superseding call on this key, resolve its
    // promise `{superseded:true}`, then take over.
    if (o.supersede) {
      const inflight = this.callInFlight.get(key);
      if (inflight && inflight.superseding && !inflight.settled) {
        this.cancel();
        this.resolveCall(inflight, { superseded: true });
      }
    }
    const callId = this.freshCallId();
    const pending: PendingCall = {
      callId: callId,
      key: key,
      resolve: resolve,
      reject: reject,
      textBuf: "",
      settled: false,
      superseding: !!o.supersede,
      agent: o.agent,
    };
    this.pendingCalls.set(callId, pending);
    // A worker call streams into its profile's active slot; the main call keeps
    // the shared one (§3.8).
    if (o.agent) this.agentActiveCall.set(o.agent, pending);
    else this.activeCall = pending;
    if (o.supersede) this.callInFlight.set(key, pending);
    // JSON.stringify drops the `undefined` fields, so name/args or text (and the
    // optional `agent`) ride exactly as populated.
    const frame = {
      type: "call",
      callId: callId,
      name: o.name,
      args: o.args,
      text: o.text,
      outputSchema: o.outputSchema,
      agent: o.agent,
    };
    if (this.send(frame)) return;
    // Socket not open yet — connect, then send (or reject if still unreachable).
    this.connect().then(
      () => {
        if (!this.send(frame)) this.rejectCall(pending, new Error("Not connected to the Biorouter backend."));
      },
      (e: Error) => this.rejectCall(pending, e)
    );
  }

  private freshCallId(): string {
    this.callSeq++;
    return "call-" + this.callSeq + "-" + Date.now().toString(36);
  }

  private resolveOutput(msg: AgentEvent): void {
    const frame = msg as OutputFrame;
    const cid = typeof frame.callId === "string" ? frame.callId : "";
    // No callId → fall back to the active call for this frame's profile (§3.8):
    // a worker frame settles that worker's call, a main frame settles activeCall.
    const agent = frameAgent(msg);
    const fallback = agent ? this.agentActiveCall.get(agent) : this.activeCall;
    const pending = cid ? this.pendingCalls.get(cid) : fallback;
    if (!pending) return;
    this.resolveCall(pending, { value: frame.value });
  }

  private settleActiveCall(err: Error | null): void {
    const pending = this.activeCall;
    if (!pending) return;
    if (err) this.rejectCall(pending, err);
    else this.resolveCall(pending, { text: pending.textBuf });
  }

  private resolveCall(pending: PendingCall, result: CallResult): void {
    if (pending.settled) return;
    pending.settled = true;
    this.pendingCalls.delete(pending.callId);
    if (this.activeCall === pending) this.activeCall = null;
    if (pending.agent && this.agentActiveCall.get(pending.agent) === pending) {
      this.agentActiveCall.delete(pending.agent);
    }
    if (this.callInFlight.get(pending.key) === pending) this.callInFlight.delete(pending.key);
    try {
      pending.resolve(result);
    } catch {
      /* consumer errors are non-fatal */
    }
  }

  private rejectCall(pending: PendingCall, err: Error): void {
    if (pending.settled) return;
    pending.settled = true;
    this.pendingCalls.delete(pending.callId);
    if (this.activeCall === pending) this.activeCall = null;
    if (pending.agent && this.agentActiveCall.get(pending.agent) === pending) {
      this.agentActiveCall.delete(pending.agent);
    }
    if (this.callInFlight.get(pending.key) === pending) this.callInFlight.delete(pending.key);
    try {
      pending.reject(err);
    } catch {
      /* consumer errors are non-fatal */
    }
  }

  // ── SDK v2 Phase 3: app.signals ─────────────────────────────────────────────

  /** Client→server signals with trailing-edge coalescing: within a signal's
   *  declared `coalesceMs` window (default 250 ms) only the last payload per name
   *  is kept, then a single `signal` frame is sent.
   *
   *  - `emit(name, payload)` — coalesced send.
   *  - `declared()` — the `[{name, coalesceMs}]` the `ready` surface advertised. */
  get signals() {
    return {
      emit: (name: string, payload: unknown) => this.emitSignal(name, payload),
      declared: () => this.declaredSignals.slice(),
    };
  }

  /**
   * `br.dnd.catalog({source, target, onDrop, signal})` — a drag interaction that
   * actually works.
   *
   * The SDK had NO drag support at all, while `theme.css` shipped `.br-dropzone`
   * and "draggable list item" styling. That is starter gravity: the CSS told the
   * model this was the blessed pattern and gave it no runtime, so it hand-rolled
   * HTML5 `draggable="true"` + `dragstart`/`drop` with `DataTransfer`. HTML5 DnD is
   * not driven by synthetic pointer moves, so a coordinate drag from Playwright, a
   * screen reader, or any assistive pointer produced NO `dragstart` — the core
   * interaction of spec-009 was unreachable by anything but a human mouse.
   *
   * This primitive is reliable by construction:
   *   - **pointer events** (`pointerdown`/`move`/`up`), so real and synthetic
   *     coordinate drags both work;
   *   - **click parity** — click an item to pick it up, click a zone to drop;
   *   - **keyboard parity** — `Enter`/`Space` picks up, arrows move between zones,
   *     `Enter` drops, `Escape` cancels, with ARIA roles and live announcements;
   *   - and it **emits the declared signal itself**, so the app→agent path cannot
   *     be forgotten.
   *
   * Returns a teardown function.
   */
  get dnd() {
    return {
      catalog: (opts: DndCatalogOptions) => this.mountDndCatalog(opts),
    };
  }

  private mountDndCatalog(opts: DndCatalogOptions) {
    const source =
      typeof opts.source === "string"
        ? document.querySelector<HTMLElement>(opts.source)
        : opts.source;
    const zones: HTMLElement[] =
      typeof opts.target === "string"
        ? Array.from(document.querySelectorAll<HTMLElement>(opts.target))
        : Array.isArray(opts.target)
          ? opts.target
          : opts.target
            ? [opts.target]
            : [];

    if (!source || !zones.length) {
      const msg = `br.dnd.catalog(): source or target not found (${String(opts.source)} → ${String(opts.target)})`;
      console.error(msg);
      this.reportUiError("dnd", msg);
      return () => undefined;
    }

    const items = () =>
      Array.from(source.querySelectorAll<HTMLElement>("[data-br-item]"));
    let picked: HTMLElement | null = null;
    let zoneIdx = 0;

    const live = document.createElement("div");
    live.className = "br-sr-only";
    live.setAttribute("aria-live", "polite");
    source.appendChild(live);
    const announce = (m: string) => {
      live.textContent = m;
    };

    const itemId = (el: HTMLElement) => el.dataset.brItem || el.id || "";
    const zoneId = (el: HTMLElement) => el.dataset.brZone || el.id || "";

    const setPicked = (el: HTMLElement | null) => {
      if (picked) picked.classList.remove("is-picked");
      picked = el;
      if (picked) {
        picked.classList.add("is-picked");
        announce(
          `${itemId(picked)} picked up. Choose a zone, then press Enter to drop.`
        );
      }
    };

    const highlight = (zone: HTMLElement | null) => {
      for (const z of zones) z.classList.toggle("is-over", z === zone);
    };

    const drop = (item: HTMLElement, zone: HTMLElement) => {
      const i = itemId(item);
      const z = zoneId(zone);
      setPicked(null);
      highlight(null);
      announce(`${i} dropped on ${z}.`);
      // The primitive emits, so the app→agent path cannot be forgotten. This is
      // the whole point: a drag that the agent never hears about is decoration.
      if (opts.signal) this.emitSignal(opts.signal, { item: i, zone: z });
      if (opts.onDrop) opts.onDrop(i, z);
    };

    const zoneAt = (x: number, y: number): HTMLElement | null =>
      zones.find((z) => {
        const r = z.getBoundingClientRect();
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      }) || null;

    // ── Pointer path (works for a real mouse AND a synthetic coordinate drag) ──
    const onPointerDown = (e: PointerEvent) => {
      const item = (e.target as HTMLElement | null)?.closest<HTMLElement>(
        "[data-br-item]"
      );
      if (!item || !source.contains(item)) return;
      e.preventDefault();
      item.setPointerCapture?.(e.pointerId);
      setPicked(item);

      const onMove = (m: PointerEvent) => highlight(zoneAt(m.clientX, m.clientY));
      const onUp = (u: PointerEvent) => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        const z = zoneAt(u.clientX, u.clientY);
        if (z && picked) drop(picked, z);
        else {
          // A pointerdown with no movement is a CLICK: keep the item picked up so
          // the click-to-drop path works (mouse, touch, and automated pointers).
          highlight(null);
        }
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    // ── Click-to-drop parity ────────────────────────────────────────────────
    const onZoneClick = (e: Event) => {
      const zone = e.currentTarget as HTMLElement;
      if (picked) drop(picked, zone);
    };

    // ── Keyboard parity ─────────────────────────────────────────────────────
    const onKeyDown = (e: KeyboardEvent) => {
      const item = (e.target as HTMLElement | null)?.closest<HTMLElement>(
        "[data-br-item]"
      );
      if (e.key === "Escape") {
        setPicked(null);
        highlight(null);
        return;
      }
      if ((e.key === "Enter" || e.key === " ") && item && !picked) {
        e.preventDefault();
        setPicked(item);
        zoneIdx = 0;
        highlight(zones[0]);
        return;
      }
      if (!picked) return;
      if (e.key === "ArrowDown" || e.key === "ArrowRight") {
        e.preventDefault();
        zoneIdx = (zoneIdx + 1) % zones.length;
        highlight(zones[zoneIdx]);
        announce(`Zone ${zoneId(zones[zoneIdx])}.`);
      } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
        e.preventDefault();
        zoneIdx = (zoneIdx - 1 + zones.length) % zones.length;
        highlight(zones[zoneIdx]);
        announce(`Zone ${zoneId(zones[zoneIdx])}.`);
      } else if (e.key === "Enter") {
        e.preventDefault();
        drop(picked, zones[zoneIdx]);
      }
    };

    // Make every item reachable and announce-able.
    for (const it of items()) {
      it.setAttribute("role", "option");
      it.setAttribute("tabindex", "0");
      it.setAttribute("aria-grabbed", "false");
    }
    source.setAttribute("role", "listbox");
    source.setAttribute("data-br-dnd", "1");
    for (const z of zones) {
      z.setAttribute("data-br-dnd-zone", "1");
      z.addEventListener("click", onZoneClick);
    }
    source.addEventListener("pointerdown", onPointerDown);
    source.addEventListener("keydown", onKeyDown);

    return () => {
      source.removeEventListener("pointerdown", onPointerDown);
      source.removeEventListener("keydown", onKeyDown);
      for (const z of zones) z.removeEventListener("click", onZoneClick);
      live.remove();
    };
  }

  private emitSignal(name: string, payload: unknown): void {
    if (!name) return;
    const decl = this.signalDeclMap.get(name);
    const coalesceMs = decl && typeof decl.coalesceMs === "number" ? decl.coalesceMs : 250;
    if (coalesceMs <= 0) {
      this.sendReliable({ type: "signal", name: name, payload: payload });
      return;
    }
    const pending = this.signalPending.get(name);
    if (pending) {
      // Trailing edge: keep the last payload; the running timer still fires once.
      pending.payload = payload;
      return;
    }
    const rec: SignalRec = { payload: payload, timer: 0 };
    rec.timer = setTimeout(() => {
      this.signalPending.delete(name);
      this.sendReliable({ type: "signal", name: name, payload: rec.payload });
    }, coalesceMs);
    this.signalPending.set(name, rec);
  }

  /** Latch the `ready` frame's advertised surface. Absent/partial surfaces are
   *  tolerated (Phase-3 servers add it; older ones do not). */
  private latchSurface(surface?: ReadySurface): void {
    if (!surface || typeof surface !== "object") return;
    const sigs = Array.isArray(surface.signals) ? surface.signals : [];
    this.declaredSignals = sigs;
    this.signalDeclMap.clear();
    for (const s of sigs) {
      if (s && typeof s.name === "string") this.signalDeclMap.set(s.name, s);
    }
    this.declaredActions = Array.isArray(surface.actions) ? surface.actions : [];
  }

  /** Convenience wiring: a `network` instance's selection change auto-emits a
   *  `node_selected` signal `{id, instance}` — but only when the `ready` surface
   *  declared such a signal (so it stays silent unless the agent asked for it). */
  autoEmitNodeSelected(nodeId: string | null, instanceId: string | null): void {
    if (!this.signalDeclMap.has("node_selected")) return;
    this.emitSignal("node_selected", { id: nodeId, instance: instanceId });
  }
}

// ---------------------------------------------------------------------------
// SDK v2 Phase 4: br.kb.graphToNetwork — pure KB-graph → network-spec mapper.
// ---------------------------------------------------------------------------

/** First non-empty string in `vals`, else `undefined`. Used to pick a field
 *  across the several names a KB graph might use (id/name, label/title, …). */
function firstString(vals: unknown[]): string | undefined {
  for (const v of vals) {
    if (typeof v === "string" && v) return v;
  }
  return undefined;
}

/** Map one KB graph node (object or a bare id string) into a `network` NetNode,
 *  or `null` when it has no usable id. Type is optional — missing-type nodes are
 *  tolerated (they simply carry no `type`). */
function kbNode(n: unknown): NetNode | null {
  if (typeof n === "string") return n ? { id: n, label: n } : null;
  if (!n || typeof n !== "object") return null;
  const o = n as KbNodeLike;
  const id = firstString([o.id, o.name]);
  if (!id) return null;
  const label = firstString([o.label, o.title, o.name]) || id;
  const type = firstString([o.type, o.kind, o.group]);
  const node: NetNode = { id: id, label: label };
  if (type) node.type = type;
  return node;
}

/** Map one KB graph edge into a `network` NetEdge, or `null` when either
 *  endpoint is missing. Accepts `source`/`target` or `from`/`to`, and derives
 *  `kind` from any of kind/relation/predicate/label/type (all optional). */
function kbEdge(e: unknown): NetEdge | null {
  if (!e || typeof e !== "object") return null;
  const o = e as KbEdgeLike;
  const source = firstString([o.source, o.from]);
  const target = firstString([o.target, o.to]);
  if (!source || !target) return null;
  const kind = firstString([o.kind, o.relation, o.predicate, o.label, o.type]);
  const edge: NetEdge = { source: source, target: target };
  if (kind) edge.kind = kind;
  return edge;
}

/** Map a knowledge-base `graph()` result (`{nodes, edges}`) into the `network`
 *  component's spec shape (`{nodes:[{id,label,type}], edges:[{source,target,kind}]}`)
 *  so an `explorer` app can do `ui.network(...)` in one line. Pure and defensive:
 *  a missing/mis-shaped graph yields empty arrays, id-less nodes and dangling
 *  edges are dropped, and missing `type`/`kind` are simply omitted. */
export function graphToNetwork(graph: unknown): NetworkSpec {
  const g = (graph && typeof graph === "object" ? graph : {}) as KbGraphLike;
  const rawNodes = Array.isArray(g.nodes) ? g.nodes : [];
  const rawEdges = Array.isArray(g.edges) ? g.edges : [];
  const nodes: NetNode[] = [];
  for (const n of rawNodes) {
    const node = kbNode(n);
    if (node) nodes.push(node);
  }
  const edges: NetEdge[] = [];
  for (const e of rawEdges) {
    const edge = kbEdge(e);
    if (edge) edges.push(edge);
  }
  return { nodes: nodes, edges: edges };
}

// ---------------------------------------------------------------------------
// Markdown rendering (compact, no external deps, HTML-escaped first).
// ---------------------------------------------------------------------------

function escapeHtml(s: string): string {
  // Quotes are escaped too: the model's markdown is untrusted (prompt injection),
  // and escaped text is interpolated into attributes downstream (notably a link
  // href). Without escaping `"`, `[t](https://x" onmouseover="alert(1))` breaks
  // out of the href and injects an event handler — XSS in the app's own origin,
  // which is not a sandboxed iframe.
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function inline(s: string): string {
  // links, bold, italic, inline code — operating on already-escaped text.
  return s
    .replace(/`([^`]+)`/g, (_m, c) => `<code>${c}</code>`)
    .replace(
      /\[([^\]]+)\]\((https?:[^)\s]+)\)/g,
      // Belt-and-suspenders with escapeHtml: the URL is `http(s)` only, and any
      // residual quote/angle/space is stripped so it cannot escape the attribute.
      (_m, t, u) =>
        `<a href="${String(u).replace(/["'<>`\s]/g, "")}" target="_blank" rel="noopener">${t}</a>`
    )
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

interface ChartPoint {
  label: string;
  value: number;
}
interface ChartSeries {
  name?: string;
  data: ChartPoint[];
}
interface ChartSpec {
  type?: "bar" | "line" | "pie";
  title?: string;
  // Single series (unchanged) …
  data?: ChartPoint[];
  // … or several named series sharing an x-axis (loss curves, comparisons, …).
  series?: ChartSeries[];
}

interface GraphNode {
  id: string;
  label?: string;
  group?: string;
}
interface GraphEdge {
  source: string;
  target: string;
  label?: string;
}
interface GraphSpec {
  title?: string;
  nodes?: Array<GraphNode | string>;
  edges?: Array<GraphEdge | string>;
}
interface ParsedGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}
interface GraphPoint {
  x: number;
  y: number;
}

/**
 * Render an AI-generated chart from a ```chart JSON block into dependency-free
 * SVG, themed with Biorouter tokens. Supports "bar" (default) and "line".
 * Returns a quiet placeholder on malformed/partial JSON (streaming-safe).
 */
const CHART_PALETTE = [
  "var(--br-coral)",
  "var(--br-accent)",
  "var(--br-n400)",
  "var(--br-green)",
  "var(--br-n500)",
  "var(--br-n300)",
];

/** Normalize a chart spec into a list of series, each cleaned of non-finite
 *  points. Accepts single-series `data` OR multi-series `series`. */
function normalizeSeries(spec: ChartSpec): ChartSeries[] {
  const clean = (pts: ChartPoint[] | undefined): ChartPoint[] =>
    Array.isArray(pts) ? pts.filter((d) => d && isFinite(d.value)) : [];
  if (Array.isArray(spec.series) && spec.series.length) {
    return spec.series.map((s) => ({ name: s.name, data: clean(s.data) })).filter((s) => s.data.length);
  }
  const d = clean(spec.data);
  return d.length ? [{ data: d }] : [];
}

/**
 * Render an AI-generated chart from a ```chart JSON block into dependency-free
 * SVG, themed with Biorouter tokens. Supports bar / line / pie, single-series
 * (`data`) or multi-series (`series` — grouped bars or overlaid lines with a
 * legend). Returns a quiet placeholder on malformed/partial JSON (streaming-safe).
 */
export function renderChart(json: string): string {
  let spec: ChartSpec;
  try {
    spec = JSON.parse(json);
  } catch {
    return '<div class="br-chart br-chart--pending"></div>';
  }
  const series = normalizeSeries(spec);
  if (!series.length) return '<div class="br-chart br-chart--pending"></div>';
  const multi = series.length > 1;
  // Categories = the labels of the longest series (series are index-aligned).
  const cats = series.reduce((a, s) => (s.data.length > a.length ? s.data : a), series[0].data).map((d) => d.label);
  const n = cats.length;

  const legendW = multi ? 96 : 0;
  const W = 520;
  const H = 240;
  const padL = 44;
  const padB = 48;
  const padT = spec.title ? 28 : 12;
  const allVals = series.flatMap((s) => s.data.map((d) => d.value));
  const max = Math.max(...allVals, 0);
  const min = Math.min(...allVals, 0);
  const span = max - min || 1;
  const plotW = W - padL - 12 - legendW;
  const plotH = H - padT - padB;
  const x = (i: number) => padL + (plotW * (i + 0.5)) / Math.max(n, 1);
  const y = (v: number) => padT + plotH * (1 - (v - min) / span);
  const esc = (s: string) => escapeHtml(String(s));
  const palette = CHART_PALETTE;
  const color = (i: number) => palette[i % palette.length];
  const title = spec.title
    ? `<text x="${W / 2}" y="18" font-size="12" font-weight="600" text-anchor="middle" fill="var(--br-text)">${esc(
        spec.title
      )}</text>`
    : "";
  const svg = (inner: string) =>
    `<div class="br-chart"><svg viewBox="0 0 ${W} ${H}" width="100%" preserveAspectRatio="xMidYMid meet" role="img">${title}${inner}</svg></div>`;

  // Legend (multi-series line/bar): named swatches on the right.
  const seriesLegend = multi
    ? series
        .map((s, si) => {
          const name = s.name || `series ${si + 1}`;
          return `<g transform="translate(${W - legendW + 4},${padT + 6 + si * 18})"><rect width="11" height="11" rx="2" fill="${color(
            si
          )}"/><text x="16" y="10" font-size="11" fill="var(--br-text)">${esc(name).slice(0, 14)}</text></g>`;
        })
        .join("")
    : "";

  // Pie: single series (the first), slices + percentage legend, no axes.
  if (spec.type === "pie") {
    const data = series[0].data;
    const total = data.reduce((s, d) => s + Math.max(d.value, 0), 0) || 1;
    const cx = 130;
    const cy = padT + (H - padT - 12) / 2;
    const r = Math.min(cy - padT, 84);
    let a0 = -Math.PI / 2;
    let slices = "";
    data.forEach((d, i) => {
      const frac = Math.max(d.value, 0) / total;
      const a1 = a0 + frac * 2 * Math.PI;
      const large = frac > 0.5 ? 1 : 0;
      const x0 = cx + r * Math.cos(a0);
      const y0 = cy + r * Math.sin(a0);
      const x1 = cx + r * Math.cos(a1);
      const y1 = cy + r * Math.sin(a1);
      slices +=
        frac >= 0.999
          ? `<circle cx="${cx}" cy="${cy}" r="${r}" fill="${color(i)}"/>`
          : `<path d="M${cx},${cy} L${x0.toFixed(1)},${y0.toFixed(1)} A${r},${r} 0 ${large} 1 ${x1.toFixed(
              1
            )},${y1.toFixed(1)} Z" fill="${color(i)}" stroke="var(--br-surface)" stroke-width="1"/>`;
      a0 = a1;
    });
    const legend = data
      .map((d, i) => {
        const pct = Math.round((Math.max(d.value, 0) / total) * 100);
        return `<g transform="translate(${cx + r + 28},${padT + 6 + i * 18})"><rect width="11" height="11" rx="2" fill="${color(
          i
        )}"/><text x="16" y="10" font-size="11" fill="var(--br-text)">${esc(d.label).slice(0, 20)} (${pct}%)</text></g>`;
      })
      .join("");
    return svg(slices + legend);
  }

  let body = "";
  if (spec.type === "line") {
    // One overlaid polyline (+ markers) per series.
    series.forEach((s, si) => {
      const pts = s.data.map((d, i) => `${x(i).toFixed(1)},${y(d.value).toFixed(1)}`).join(" ");
      const stroke = color(si);
      body += `<polyline fill="none" stroke="${stroke}" stroke-width="2.5" points="${pts}"/>`;
      body += s.data
        .map((d, i) => `<circle cx="${x(i).toFixed(1)}" cy="${y(d.value).toFixed(1)}" r="3" fill="${stroke}"/>`)
        .join("");
    });
  } else {
    // Grouped bars: within each category, one bar per series side by side.
    const groupW = Math.min((plotW / Math.max(n, 1)) * 0.72, 64);
    const bw = groupW / series.length;
    series.forEach((s, si) => {
      body += s.data
        .map((d, i) => {
          const bx = x(i) - groupW / 2 + si * bw;
          const by = y(Math.max(d.value, 0));
          const bh = Math.abs(y(d.value) - y(0));
          const fill = multi ? color(si) : color(i);
          return `<rect x="${bx.toFixed(1)}" y="${by.toFixed(1)}" width="${bw.toFixed(1)}" height="${bh.toFixed(
            1
          )}" rx="2" fill="${fill}"/>`;
        })
        .join("");
    });
  }
  // x labels (truncated, centered) + baseline + y-max tick.
  const labels = cats
    .map(
      (lab, i) =>
        `<text x="${x(i).toFixed(1)}" y="${H - padB + 16}" font-size="10" text-anchor="middle" fill="var(--br-text-muted)">${esc(
          lab
        ).slice(0, 10)}</text>`
    )
    .join("");
  const axis = `<line x1="${padL}" y1="${y(0).toFixed(1)}" x2="${W - 12 - legendW}" y2="${y(0).toFixed(
    1
  )}" stroke="var(--br-border)"/>`;
  const yMax = `<text x="${padL - 6}" y="${(y(max) + 4).toFixed(
    1
  )}" font-size="10" text-anchor="end" fill="var(--br-text-muted)">${esc(String(max))}</text>`;
  return svg(axis + body + labels + yMax + seriesLegend);
}

function parseGraphEdges(text: string): ParsedGraph {
  const nodeMap = new Map<string, GraphNode>();
  const edges: GraphEdge[] = [];
  const ensure = (id: string) => {
    const clean = id.trim().replace(/^["'`[]+|["'`\]]+$/g, "");
    if (!clean) return "";
    if (!nodeMap.has(clean)) nodeMap.set(clean, { id: clean, label: clean });
    return clean;
  };
  for (const raw of text.split("\n")) {
    const line = raw
      .trim()
      .replace(/;$/, "")
      .replace(/^\s*(graph|flowchart)\s+(TD|TB|BT|LR|RL)\s*/i, "");
    if (!line || line.startsWith("#")) continue;
    const match = line.match(/^(.+?)\s*(?:-->|---|--|->)\s*(.+?)(?:\s*[:|]\s*(.+))?$/);
    if (!match) continue;
    const source = ensure(match[1].replace(/\[.*?\]|\(.*?\)/g, ""));
    const target = ensure(match[2].replace(/\[.*?\]|\(.*?\)/g, ""));
    if (source && target) edges.push({ source, target, label: match[3]?.trim() });
  }
  return { nodes: Array.from(nodeMap.values()), edges };
}

function normalizeGraph(input: string): GraphSpec | null {
  try {
    const spec = JSON.parse(input) as GraphSpec;
    const nodes = Array.isArray(spec.nodes) ? spec.nodes : [];
    const edges = Array.isArray(spec.edges) ? spec.edges : [];
    if (nodes.length || edges.length) return spec;
  } catch {
    // Edge-list and Mermaid-ish blocks are parsed below.
  }
  const parsed = parseGraphEdges(input);
  return parsed.nodes.length || parsed.edges.length ? parsed : null;
}

/**
 * Render a graph/diagram block as dependency-free SVG. Accepts either:
 * ```graph JSON ({ nodes, edges, title }) or a simple edge list:
 * A -> B : relationship
 */
export function renderGraph(input: string): string {
  const spec = normalizeGraph(input);
  if (!spec) return '<div class="br-visual br-visual--pending"></div>';
  const rawNodes = Array.isArray(spec.nodes) ? spec.nodes : [];
  const rawEdges = Array.isArray(spec.edges) ? spec.edges : [];
  const nodes = new Map<string, GraphNode>();
  const ensureNode = (id: string, label?: string) => {
    const clean = String(id || "").trim();
    if (!clean) return;
    if (!nodes.has(clean)) nodes.set(clean, { id: clean, label: label || clean });
  };
  for (const n of rawNodes) {
    if (typeof n === "string") ensureNode(n);
    else ensureNode(n.id, n.label);
  }
  const edges: GraphEdge[] = [];
  for (const e of rawEdges) {
    if (typeof e === "string") {
      const parsed = parseGraphEdges(e);
      for (const n of parsed.nodes) ensureNode(n.id, n.label);
      edges.push(...parsed.edges);
    } else if (e?.source && e?.target) {
      ensureNode(e.source);
      ensureNode(e.target);
      edges.push({ source: e.source, target: e.target, label: e.label });
    }
  }
  if (!nodes.size) {
    for (const e of edges) {
      ensureNode(e.source);
      ensureNode(e.target);
    }
  }
  const list = Array.from(nodes.values()).slice(0, 18);
  const visible = new Set(list.map((n) => n.id));
  const visibleEdges = edges.filter((e) => visible.has(e.source) && visible.has(e.target)).slice(0, 28);
  if (!list.length) return '<div class="br-visual br-visual--pending"></div>';

  const W = 640;
  const H = 360;
  const cx = W / 2;
  const cy = H / 2 + 8;
  const radius = Math.min(132, 52 + list.length * 9);
  const esc = (s: string) => escapeHtml(String(s));
  const pos = new Map<string, GraphPoint>();
  list.forEach((n, i) => {
    const angle = -Math.PI / 2 + (2 * Math.PI * i) / Math.max(list.length, 1);
    pos.set(n.id, {
      x: cx + Math.cos(angle) * radius,
      y: cy + Math.sin(angle) * radius,
    });
  });
  const title = spec.title
    ? `<text x="${cx}" y="24" font-size="13" font-weight="650" text-anchor="middle" fill="var(--br-text)">${esc(spec.title)}</text>`
    : "";
  const edgeEls = visibleEdges
    .map((e) => {
      const a = pos.get(e.source);
      const b = pos.get(e.target);
      if (!a || !b) return "";
      const mx = (a.x + b.x) / 2;
      const my = (a.y + b.y) / 2;
      const label = e.label
        ? `<text x="${mx.toFixed(1)}" y="${(my - 5).toFixed(1)}" font-size="10" text-anchor="middle" fill="var(--br-text-muted)">${esc(
            e.label
          ).slice(0, 26)}</text>`
        : "";
      return `<line x1="${a.x.toFixed(1)}" y1="${a.y.toFixed(1)}" x2="${b.x.toFixed(1)}" y2="${b.y.toFixed(
        1
      )}" stroke="var(--br-visual-line)" stroke-width="1.4"/>${label}`;
    })
    .join("");
  const nodeEls = list
    .map((n, i) => {
      const p = pos.get(n.id)!;
      const fill = i % 3 === 0 ? "var(--br-coral)" : i % 3 === 1 ? "var(--br-accent)" : "var(--br-n500)";
      return `<g><circle cx="${p.x.toFixed(1)}" cy="${p.y.toFixed(1)}" r="18" fill="${fill}"/><text x="${p.x.toFixed(
        1
      )}" y="${(p.y + 4).toFixed(1)}" font-size="10" text-anchor="middle" fill="var(--br-on-accent)">${esc(
        n.label || n.id
      ).slice(0, 10)}</text></g>`;
    })
    .join("");
  return `<div class="br-visual br-graph"><svg viewBox="0 0 ${W} ${H}" width="100%" preserveAspectRatio="xMidYMid meet" role="img">${title}${edgeEls}${nodeEls}</svg></div>`;
}

// Sequence for auto-ids assigned to lowered markdown fences (chart#n / graph#n),
// so `ui_patch replace` can address a chart/graph rendered inside prose.
let fenceSeq = 0;
function withFenceIid(html: string, kind: string): string {
  fenceSeq++;
  const id = kind + "#" + fenceSeq;
  return html.replace(/^<div /, '<div data-br-iid="' + id + '" ');
}

export function renderMarkdown(md: string): string {
  const src = escapeHtml(md || "");
  const lines = src.split("\n");
  const out: string[] = [];
  let i = 0;
  let inList = false;
  let inOrdered = false;
  const closeList = () => {
    if (inList) {
      out.push(inOrdered ? "</ol>" : "</ul>");
      inList = false;
      inOrdered = false;
    }
  };
  while (i < lines.length) {
    const line = lines[i];
    // fenced code block
    const fence = line.match(/^```([A-Za-z0-9_-]*)(?:\s+.*)?\s*$/);
    if (fence) {
      closeList();
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      i++; // skip closing fence
      const fenceKind = (fence[1] || "").toLowerCase();
      // AI-generated visualization blocks render as themed SVG. Fences are
      // lowered to addressable instances (data-br-iid) so `ui_patch replace`
      // can target them — kept on the lightweight renderers, not the heavy
      // network engine.
      if (fenceKind === "chart") {
        out.push(withFenceIid(renderChart(code.join("\n")), "chart"));
      } else if (["graph", "diagram", "network", "map", "mermaid"].includes(fenceKind)) {
        out.push(withFenceIid(renderGraph(code.join("\n")), "graph"));
      } else {
        out.push(`<pre><code>${code.join("\n")}</code></pre>`);
      }
      continue;
    }
    const heading = line.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      closeList();
      const level = heading[1].length;
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      i++;
      continue;
    }
    // GFM table: a header row, a separator row (---|:--:), then body rows.
    if (
      /^\s*\|.*\|\s*$/.test(line) &&
      i + 1 < lines.length &&
      /^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(lines[i + 1]) &&
      lines[i + 1].includes("-")
    ) {
      closeList();
      const cells = (row: string) =>
        row
          .trim()
          .replace(/^\||\|$/g, "")
          .split("|")
          .map((c) => c.trim());
      const header = cells(line);
      i += 2; // skip header + separator
      const body: string[][] = [];
      while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) {
        body.push(cells(lines[i]));
        i++;
      }
      let t = "<table><thead><tr>";
      t += header.map((h) => `<th>${inline(h)}</th>`).join("");
      t += "</tr></thead><tbody>";
      for (const r of body) {
        t += "<tr>" + r.map((c) => `<td>${inline(c)}</td>`).join("") + "</tr>";
      }
      t += "</tbody></table>";
      out.push(t);
      continue;
    }
    const ul = line.match(/^\s*[-*]\s+(.*)$/);
    const ol = line.match(/^\s*\d+\.\s+(.*)$/);
    if (ul || ol) {
      const ordered = !!ol;
      if (!inList || inOrdered !== ordered) {
        closeList();
        out.push(ordered ? "<ol>" : "<ul>");
        inList = true;
        inOrdered = ordered;
      }
      // Hoisted out of the template literal so the fallback stripper can drop
      // the non-null assertion (`!`) — it skips `${…}` interpolations as string.
      const liText = (ul || ol)![1];
      out.push(`<li>${inline(liText)}</li>`);
      i++;
      continue;
    }
    if (line.trim() === "") {
      closeList();
      i++;
      continue;
    }
    closeList();
    // accumulate a paragraph
    const para: string[] = [line];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,4})\s|^\s*[-*]\s|^\s*\d+\.\s|^```|^\s*\|.*\|\s*$/.test(lines[i])
    ) {
      para.push(lines[i]);
      i++;
    }
    out.push(`<p>${inline(para.join(" "))}</p>`);
  }
  closeList();
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Default chat panel (used when the app doesn't build its own UI).
// ---------------------------------------------------------------------------

/** File → base64 ImageInput (strips the data-URL prefix). */
export function fileToImageInput(file: File): Promise<ImageInput> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result || "");
      const comma = result.indexOf(",");
      resolve({ mimeType: file.type || "image/png", data: result.slice(comma + 1) });
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function eventSummary(ev: AgentEvent): TimelineSummary | null {
  switch (ev.type) {
    case "ready":
      return {
        label: ev.resumed ? "Session resumed" : "Session ready",
        detail: ev.capabilities?.length ? ev.capabilities.join(", ") : "Agent connection open",
        state: "done",
      };
    case "tool":
      return {
        label: ev.status === "completed" ? "Tool completed" : ev.status === "failed" ? "Tool failed" : "Tool running",
        detail: ev.name || "tool",
        state: ev.status === "completed" ? "done" : ev.status === "failed" ? "error" : "active",
      };
    case "tool_call":
      return { label: "Tool call queued", detail: ev.name, state: "active" };
    case "guardrail":
      return {
        label: ev.blocked ? "Guardrail blocked" : "Guardrail checked",
        detail: [ev.name, ev.stage, ev.reason].filter(Boolean).join(" · "),
        state: ev.blocked ? "error" : "done",
      };
    case "approval":
      return { label: "Approval needed", detail: ev.tool, state: "active" };
    case "approval_refused": {
      // Say what happened AND what it means for the run: the tool was denied, so
      // the turn is continuing without it. A bare "refused" would leave the user
      // waiting for something that is never coming.
      // `"timeout"` is the daemon's bounded wait (`APP_APPROVAL_WAIT`) giving
      // up: nobody answered, so it denied on the page's behalf rather than
      // holding the socket's only reader open forever. Naming that is the whole
      // point — a bare "timeout" reads as a network fault, not as a decision.
      const why =
        ev.reason === "unproven"
          ? "this app page may not grant tool approvals"
          : ev.reason === "timeout"
            ? "nobody answered in time, so Biorouter answered for you"
            : ev.reason || "the approval was not applied";
      return { label: "Approval refused", detail: why + " — the tool was denied", state: "error" };
    }
    case "capability_report": {
      // Typed, not `{}`: an untyped empty-object fallback made every `rep.*`
      // read below a type error that no `tsc` in this repo is watching for.
      const rep: CapabilityReportBody = ev.report || {};
      const missing = (rep.missing_skills || []).slice();
      if (rep.missing_knowledge_base) missing.push("knowledge base " + rep.missing_knowledge_base);
      const unmet = (rep.unmet_requirements || []).length;
      if (unmet) missing.push(unmet + " unmet requirement" + (unmet === 1 ? "" : "s"));
      // There is no healthy report to draw. The daemon sends this frame only
      // inside `if capability_report.degraded()` and hardcodes
      // `"degraded": true` in the payload, so an "everything this app asked for
      // is available" row was unreachable — a branch that reads as a tested
      // state and is not one. If the daemon ever starts sending a non-degraded
      // report, this arm has to grow the branch back rather than mislabel it.
      return {
        label: "Capabilities missing",
        detail: missing.length
          ? missing.join(", ")
          : "some of what this app asked for is unavailable",
        state: "error",
      };
    }
    case "output_retry": {
      const n = typeof ev.attempt === "number" ? ev.attempt : 0;
      return {
        label: "Retrying structured output",
        detail: n ? "attempt " + n : "the last result did not match the schema",
        state: "active",
      };
    }
    case "handoff":
      return {
        label: "Agent handoff",
        detail: [ev.from, ev.to].filter(Boolean).join(" → "),
        state: "active",
      };
    case "compaction":
      return {
        label: "Context compaction",
        detail: [ev.phase, ev.trigger].filter(Boolean).join(" · "),
        state: ev.phase === "done" ? "done" : "active",
      };
    case "context":
      return {
        label: "Context checked",
        detail: ev.limit ? `${ev.used || 0}/${ev.limit} tokens (${Math.round((ev.ratio || 0) * 100)}%)` : "Token usage unavailable",
        state: "done",
      };
    case "model":
      return {
        label: ev.ok ? "Model selected" : "Model switch failed",
        detail: [ev.provider, ev.model].filter(Boolean).join(" / "),
        state: ev.ok ? "done" : "error",
      };
    case "widget":
      return { label: "Interface updated", detail: ev.id, state: "done" };
    case "done":
      return { label: "Run complete", detail: "", state: "done" };
    case "error":
      return { label: "Run error", detail: ev.message, state: "error" };
    default:
      // Two very different things land here, and collapsing them is the bug this
      // arm exists to prevent. A KNOWN kind that simply has no timeline row
      // (`message`, `thought`, `usage`, `ui`, …) draws nothing — otherwise every
      // streamed delta would push a row. A kind this SDK has never heard of is
      // drawn, loudly: silence is what made `approval_refused` look like a hang.
      return isKnownEventKind(ev.type)
        ? null
        : { label: "Unrecognized event", detail: String(ev.type), state: "error" };
  }
}

/**
 * Approve / reject buttons for one pending tool, appended to its timeline row.
 *
 * ⚠ Without these, NOTHING in this SDK ever called `br.approve` / `br.reject`.
 * The `approval` row said "Approval needed" and offered no way to answer it,
 * while on the daemon side `handle_action_required` parked the app socket's ONLY
 * reader on the decision — so an ordinary tool approval froze the whole
 * connection: the run, every later prompt, `ui_ask` replies and even cancel.
 * `approval_refused` (issue #143) covered the case the daemon *denied*; this is
 * the far commoner one where nobody answered at all. The daemon now also bounds
 * that wait (`APP_APPROVAL_WAIT`, 120 s → an `approval_refused` with reason
 * `"timeout"`), which is the floor under the freeze; these buttons are what make
 * answering it possible in the first place.
 *
 * An app that builds its own approval interface simply does not mount the
 * timeline — `br.approve`/`br.reject` are public and take the same `requestId`.
 */
function mountApprovalControls(
  client: BioRouterClient,
  row: HTMLElement,
  requestId: string
): void {
  if (!requestId) return;
  const actions = document.createElement("span");
  actions.className = "br-run-step__actions";
  actions.setAttribute("data-br-approval", requestId);
  // ⚠ Sized inline, deliberately. `.br-run-step` is a TWO-column grid and a
  // third child lands in a new row, and `theme.css` — which this file does not
  // own — has no `.br-run-step__actions` rule and sizes `.br-btn` at 36px, which
  // is oversized inside a 0.75rem timeline row. The colours, radius and press
  // still come from the theme's `.br-btn` / `.br-btn--ghost`; only the geometry
  // is here. Moving these five lines into `theme.css` is a clean follow-up.
  actions.style.gridColumn = "1 / -1";
  actions.style.display = "flex";
  actions.style.gap = "6px";
  actions.style.marginTop = "2px";
  const settle = (verdict: string) => {
    // Replacing the children removes both buttons, so a second click cannot
    // send a second decision for a request the daemon has already resolved.
    actions.textContent = verdict;
    actions.setAttribute("data-br-approval-settled", verdict);
  };
  const button = (label: string, kind: string, verdict: string, pick: ResolveFn) => {
    const b = document.createElement("button");
    b.type = "button";
    // Reject is the quiet one: the theme's ghost variant, so the accented
    // button is the one that grants a tool rather than the one that refuses.
    b.className = "br-btn br-btn--" + kind + (kind === "reject" ? " br-btn--ghost" : "");
    b.textContent = label;
    b.setAttribute("data-br-approval-action", kind);
    b.style.height = "24px";
    b.style.padding = "0 10px";
    b.style.fontSize = "0.75rem";
    b.addEventListener("click", () => {
      settle(verdict);
      pick();
    });
    return b;
  };
  actions.appendChild(button("Approve", "approve", "Approved", () => client.approve(requestId)));
  actions.appendChild(button("Reject", "reject", "Rejected", () => client.reject(requestId)));
  row.appendChild(actions);
}

/** Retire the buttons for a request the daemon has already decided — a refusal,
 *  including the bounded-wait timeout. Leaving them live invites a click that
 *  sends a decision for a request that no longer exists. */
function settleApprovalControls(host: HTMLElement, requestId: string, verdict: string): void {
  if (!requestId) return;
  const actions = host.querySelector('[data-br-approval="' + cssEscape(requestId) + '"]');
  if (actions && !actions.getAttribute("data-br-approval-settled")) {
    actions.textContent = verdict;
    actions.setAttribute("data-br-approval-settled", verdict);
  }
}

/** Mount a visible execution timeline for long-running agent work. Generated
 * apps should include this when they call `prompt()` / `ask()` directly; `run()`
 * and the default chat mount it automatically. */
export function mountTimeline(
  client: BioRouterClient,
  target: HTMLElement | string,
  options: TimelineOptions = {}
) {
  const host =
    typeof target === "string"
      ? (document.querySelector(target) as HTMLElement | null)
      : target;
  if (!host) {
    // A silent no-op here is how an app ends up with no progress display at all
    // and nobody knowing why.
    console.error(`mountTimeline(): target not found: ${String(target)}`);
    return () => undefined;
  }
  host.classList.add("br-run-status");
  PROGRESS_SINKS.add(host);
  const maxItems = options.maxItems || 16;
  const entries: HTMLElement[] = [];
  const add = (ev: AgentEvent) => {
    const s = eventSummary(ev);
    if (!s) return;
    const row = document.createElement("div");
    row.className = "br-run-step br-run-step--" + s.state;
    const label = document.createElement("span");
    label.className = "br-run-step__label";
    label.textContent = s.label;
    row.appendChild(label);
    if (s.detail) {
      const detail = document.createElement("span");
      detail.className = "br-run-step__detail";
      detail.textContent = s.detail;
      row.appendChild(detail);
    }
    // An `approval` is the one frame that needs an ACTION and not just a row.
    if (ev.type === "approval") mountApprovalControls(client, row, ev.requestId);
    // …and the one the daemon can answer on the user's behalf: once it has, the
    // buttons on the earlier row are stale.
    if (ev.type === "approval_refused") {
      settleApprovalControls(host, ev.requestId, "Refused");
    }
    host.appendChild(row);
    entries.push(row);
    while (entries.length > maxItems) {
      const old = entries.shift();
      if (old) old.remove();
    }
  };
  // ONE wildcard subscription, not a hand-kept list of kinds, so `eventSummary`
  // is the single place that decides what a frame looks like. The parallel
  // `kinds` array this replaces could fall out of step with `eventSummary` — and
  // a subscription list is structurally incapable of naming a frame type the SDK
  // does not know yet, so an unrecognized frame could never reach the timeline
  // at all. That is the `approval_refused` silence, one layer down.
  client.onAny(add);
  return () => {
    client.offAny(add);
    PROGRESS_SINKS.delete(host);
  };
}

// ── Interactive widgets ─────────────────────────────────────────────────────
// Agent-emitted UI (cards/forms/tables/charts) that can call back into the loop:
// the agent emits a `widget` frame, the app renders the tree, and a Button with
// `submit` collects the named form fields and sends a `widget_action` frame the
// server feeds back into the agent as the next turn. Dependency-free DOM,
// built only from the Biorouter theme classes so it stays on-brand and passes
// the build lint.

export type WidgetNode =
  | { t: "card"; title?: string; children: WidgetNode[] }
  | { t: "row"; children: WidgetNode[] }
  | { t: "col"; children: WidgetNode[] }
  | { t: "text"; value: string; markdown?: boolean; muted?: boolean }
  | { t: "badge"; value: string }
  | { t: "table"; columns: string[]; rows: Array<Array<string | number>> }
  | { t: "chart"; spec: unknown }
  | { t: "graph"; spec: unknown }
  | { t: "stat"; label?: string; value: string | number; unit?: string; delta?: string }
  | { t: "divider" }
  | { t: "progress"; value: number; label?: string }
  | { t: "input"; name: string; label?: string; value?: string; placeholder?: string; inputType?: string }
  | { t: "select"; name: string; label?: string; value?: string; options: Array<string | { value: string; label?: string }> }
  | { t: "checkbox"; name: string; label?: string; checked?: boolean }
  | { t: "button"; label: string; action: string; variant?: string; submit?: boolean }
  | { t: "form"; children: WidgetNode[] }
  // ── SDK v2 science pack (added to bodies/patches; server-validated) ──
  | { t: "markdown"; md: string; id?: string }
  | { t: "image"; src: string; alt?: string; caption?: string; id?: string }
  | { t: "kpi"; label?: string; value: string | number; delta?: string | number; unit?: string; id?: string }
  | { t: "log"; lines?: LogLine[]; max?: number; id?: string }
  | { t: "plot"; spec: PlotSpec; id?: string }
  | { t: "network"; spec: NetworkSpec; id?: string }
  | { t: "component"; name: string; props?: unknown; id?: string }
  | { t: "html"; html: string; id?: string }
  | { t: "figure"; html: string; tool?: string; id?: string };

// Node shapes for the science-pack renderers. Interfaces are dropped wholesale by
// the no-esbuild fallback stripper, so they never leak into the emitted JS.
export interface LogLine {
  level?: string;
  text: string;
}
export interface PlotPoint {
  label?: string;
  value?: number;
  x?: number;
  y?: number;
}
export interface PlotSeries {
  name?: string;
  data?: PlotPoint[];
  points?: PlotPoint[];
  values?: number[];
}
export interface PlotSpec {
  type?: string;
  title?: string;
  data?: PlotPoint[];
  series?: PlotSeries[];
  values?: number[];
  z?: number[][];
  xLabels?: string[];
  yLabels?: string[];
}
export interface NetNode {
  id: string;
  label?: string;
  type?: string;
  size?: number;
  color?: string;
}
export interface NetEdge {
  source: string;
  target: string;
  kind?: string;
  style?: string;
  label?: string;
}
export interface NetEncoding {
  type_colors?: Record<string, string>;
  families?: Record<string, string>;
  negated_kinds?: string[];
}
export interface NetPhysics {
  charge?: number;
  linkDistance?: number;
  gravity?: number;
  damping?: number;
}
export interface NetworkSpec {
  title?: string;
  nodes?: NetNode[];
  edges?: NetEdge[];
  encoding?: NetEncoding;
  physics?: NetPhysics;
  onSelect?: NetSelectCb;
}
export interface NetworkController {
  select(id: string | null): void;
  positions(): Record<string, XY>;
  adopt(prev: Record<string, XY>): void;
  destroy(): void;
}
interface XY {
  x: number;
  y: number;
}
// Author-registered components (Task 3). `props` is agent-controlled — untrusted.
export interface ComponentContext {
  id: string;
  state: unknown;
  run: RunFn;
}
export interface ComponentDef {
  props?: unknown;
  mount: MountFn;
  update?: UpdateFn;
}
// Small helper node shapes used by the renderers/registry below.
interface ImageNode {
  src?: string;
  alt?: string;
  caption?: string;
}
interface KpiNode {
  label?: string;
  value?: string | number;
  delta?: string | number;
  unit?: string;
}
interface LogNode {
  lines?: LogLine[];
  max?: number;
}
interface FigureNode {
  html?: string;
  tool?: string;
}
interface ComponentNode {
  t?: string;
  name?: string;
  props?: unknown;
}
interface InstanceEntry {
  node: WidgetNode;
  el: HTMLElement;
}
interface PatchOp {
  op?: string;
  id?: string;
  target?: string;
  parent?: string;
  index?: number;
  node?: WidgetNode;
  props?: AnyRecord;
}
interface FocusSnap {
  name: string;
  start: number;
  end: number;
  value: string;
}
interface ScrollSnap {
  idx: number[];
  tops: number[];
  lefts: number[];
}
// Named callback/marker aliases so the fallback stripper drops the annotations
// (a bare `(id: string) => void` has no leading uppercase/primitive token).
type NetSelectCb = (id: string) => void;
type RunFn = (text: string, target: HTMLElement | string, opts?: PromptOptions) => Promise<string>;
type MountFn = (el: HTMLElement, props: unknown, ctx: ComponentContext) => void;
type UpdateFn = (el: HTMLElement, props: unknown, prev: unknown) => void;
type MountComponentFn = (name: string, props: unknown, id?: string) => HTMLElement | null;
type NodeWithId = { id?: string };
type WithNet = { __brNet?: NetworkController };

export interface WidgetContext {
  // name → live value getter, registered by inputs/selects/checkboxes.
  fields: Map<string, () => string | boolean>;
  // dispatched by a button (carrying collected form fields on submit).
  onAction: (action: string, payload: unknown) => void;
  // optional bridge to the author component registry (Task 3).
  mountComponent?: MountComponentFn;
}

function wEl(tag: string, cls?: string): HTMLElement {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

// A widget node whose `t` is not one we render. Named so the no-esbuild
// fallback stripper removes the `as UnknownWidget` cast cleanly (an inline
// `as { t?: string }` object-type cast would confuse its line scanner).
type UnknownWidget = { t?: string };
// Warn at most once per unknown widget kind, so a forward-compatible frame full
// of a new kind doesn't flood the console.
const warnedWidgetKinds: Set<string> = new Set();

/** Build a detached DOM subtree from a widget node. Recursive; unknown node
 *  types render a neutral labeled placeholder rather than throwing, so a
 *  newer agent emitting a not-yet-supported kind degrades gracefully. */
export function renderWidget(node: WidgetNode, ctx: WidgetContext): HTMLElement {
  switch (node.t) {
    case "card": {
      const c = wEl("div", "br-card");
      if (node.title) {
        const h = wEl("div", "br-card__title");
        h.textContent = node.title;
        c.appendChild(h);
      }
      for (const ch of node.children) c.appendChild(renderWidget(ch, ctx));
      return c;
    }
    case "row":
    case "col": {
      const r = wEl("div", node.t === "row" ? "br-row" : "br-col");
      for (const ch of node.children) r.appendChild(renderWidget(ch, ctx));
      return r;
    }
    case "text": {
      const t = wEl("div", node.muted ? "br-text br-text--muted" : "br-text");
      if (node.markdown) t.innerHTML = renderMarkdown(node.value);
      else t.textContent = node.value;
      return t;
    }
    case "badge": {
      const b = wEl("span", "br-badge");
      b.textContent = node.value;
      return b;
    }
    case "table": {
      const tbl = wEl("table", "br-table");
      const thead = wEl("thead");
      const htr = wEl("tr");
      for (const col of node.columns) {
        const th = wEl("th");
        th.textContent = col;
        htr.appendChild(th);
      }
      thead.appendChild(htr);
      tbl.appendChild(thead);
      const tbody = wEl("tbody");
      for (const row of node.rows) {
        const tr = wEl("tr");
        for (const cell of row) {
          const td = wEl("td");
          td.textContent = String(cell);
          tr.appendChild(td);
        }
        tbody.appendChild(tr);
      }
      tbl.appendChild(tbody);
      return tbl;
    }
    case "chart": {
      const w = wEl("div", "br-chart");
      w.innerHTML = renderChart(JSON.stringify(node.spec));
      return w;
    }
    case "graph": {
      const w = wEl("div", "br-visual");
      w.innerHTML = renderGraph(JSON.stringify(node.spec));
      return w;
    }
    case "stat": {
      const s = wEl("div", "br-stat");
      if (node.label) {
        const l = wEl("div", "br-stat__label");
        l.textContent = node.label;
        s.appendChild(l);
      }
      const v = wEl("div", "br-stat__value");
      v.textContent = String(node.value);
      if (node.unit) {
        const u = wEl("span", "br-stat__unit");
        u.textContent = " " + node.unit;
        v.appendChild(u);
      }
      s.appendChild(v);
      if (node.delta) {
        // A leading "-" reads as a decrease; everything else is neutral/up.
        const down = node.delta.trim().indexOf("-") === 0;
        const d = wEl("div", down ? "br-stat__delta br-stat__delta--down" : "br-stat__delta");
        d.textContent = node.delta;
        s.appendChild(d);
      }
      return s;
    }
    case "divider":
      return wEl("hr", "br-divider");
    case "progress": {
      const p = wEl("div", "br-progress");
      if (node.label) {
        const l = wEl("div", "br-progress__label");
        l.textContent = node.label;
        p.appendChild(l);
      }
      const track = wEl("div", "br-progress__track");
      const bar = wEl("div", "br-progress__bar");
      const pct = Math.max(0, Math.min(1, Number(node.value) || 0)) * 100;
      bar.style.width = pct.toFixed(1) + "%";
      track.appendChild(bar);
      p.appendChild(track);
      return p;
    }
    case "input": {
      const wrap = wEl("label", "br-field");
      if (node.label) {
        const l = wEl("span", "br-field__label");
        l.textContent = node.label;
        wrap.appendChild(l);
      }
      const i = document.createElement("input");
      i.className = "br-input";
      i.type = node.inputType || "text";
      if (node.value) i.value = node.value;
      if (node.placeholder) i.placeholder = node.placeholder;
      ctx.fields.set(node.name, () => i.value);
      wrap.appendChild(i);
      return wrap;
    }
    case "select": {
      const wrap = wEl("label", "br-field");
      if (node.label) {
        const l = wEl("span", "br-field__label");
        l.textContent = node.label;
        wrap.appendChild(l);
      }
      const s = document.createElement("select");
      s.className = "br-select";
      for (const opt of node.options) {
        const o = document.createElement("option");
        // Accept both {value,label} objects and bare strings. The server-side
        // validator (and `ui_ask`) allow plain-string options, so a
        // string-options select must render its choices, not blank entries.
        if (typeof opt === "string") {
          o.value = opt;
          o.textContent = opt;
        } else {
          o.value = opt.value;
          o.textContent = opt.label ?? opt.value;
        }
        if (node.value === o.value) o.selected = true;
        s.appendChild(o);
      }
      ctx.fields.set(node.name, () => s.value);
      wrap.appendChild(s);
      return wrap;
    }
    case "checkbox": {
      const wrap = wEl("label", "br-check");
      const c = document.createElement("input");
      c.type = "checkbox";
      c.checked = node.checked === true;
      ctx.fields.set(node.name, () => c.checked);
      wrap.appendChild(c);
      if (node.label) {
        const l = wEl("span");
        l.textContent = node.label;
        wrap.appendChild(l);
      }
      return wrap;
    }
    case "button": {
      const b = document.createElement("button");
      b.className =
        node.variant === "secondary"
          ? "br-btn br-btn--secondary"
          : node.variant === "ghost"
            ? "br-btn br-btn--ghost"
            : "br-btn";
      b.textContent = node.label;
      const action = node.action;
      const submit = node.submit === true;
      b.addEventListener("click", () => {
        let payload: unknown;
        if (submit) {
          const collected: Record<string, string | boolean> = {};
          ctx.fields.forEach((get, name) => {
            collected[name] = get();
          });
          payload = collected;
        }
        ctx.onAction(action, payload);
      });
      return b;
    }
    case "form": {
      const f = wEl("div", "br-form");
      for (const ch of node.children) f.appendChild(renderWidget(ch, ctx));
      return f;
    }
    // ── SDK v2 science pack ──
    case "markdown": {
      const d = wEl("div", "br-md");
      d.innerHTML = renderMarkdown(node.md || "");
      return d;
    }
    case "image":
      return renderImage(node);
    case "kpi":
      return renderKpi(node);
    case "log":
      return renderLog(node);
    case "plot": {
      const w = wEl("div", "br-plot");
      w.innerHTML = renderPlot(node.spec);
      return w;
    }
    case "network":
      return createNetwork(node.spec);
    case "component": {
      const name = String(node.name || "");
      if (ctx.mountComponent) {
        const mounted = ctx.mountComponent(name, node.props, (node as NodeWithId).id);
        if (mounted) return mounted;
      }
      return unknownWidgetEl("component:" + name);
    }
    case "html": {
      // `node.html` is ALREADY server-sanitized (privileged kind, `ui.allow_html`).
      const h = wEl("div", "br-html");
      h.innerHTML = String(node.html || "");
      return h;
    }
    case "figure":
      return renderFigure(node);
    default: {
      const kind = String((node as UnknownWidget).t || "unknown");
      return unknownWidgetEl(kind);
    }
  }
}

/** The neutral, warn-once placeholder for an unknown/unregistered widget kind. */
function unknownWidgetEl(kind: string): HTMLElement {
  if (!warnedWidgetKinds.has(kind)) {
    warnedWidgetKinds.add(kind);
    try {
      console.warn("[Biorouter] unsupported widget kind: " + kind);
    } catch {
      /* console may be unavailable */
    }
  }
  const d = wEl("div", "br-unknown-widget");
  d.textContent = "[unsupported: " + kind + "]";
  return d;
}

// ── science-pack leaf renderers (dependency-free, theme-token styled) ────────

/** Refuse `javascript:` and insecure `http:` image sources client-side (the
 *  server also validates); https / data: / relative references are allowed. */
function isSafeImageSrc(s: string): boolean {
  const v = (s || "").trim().toLowerCase();
  if (v.indexOf("javascript:") === 0) return false;
  if (v.indexOf("http:") === 0) return false;
  return true;
}

function renderImage(node: ImageNode): HTMLElement {
  const fig = wEl("figure", "br-image");
  const img = document.createElement("img");
  const src = String(node.src || "");
  if (isSafeImageSrc(src)) img.setAttribute("src", src);
  img.setAttribute("alt", node.alt || "");
  fig.appendChild(img);
  if (node.caption) {
    const cap = wEl("figcaption", "br-image__cap");
    cap.textContent = node.caption;
    fig.appendChild(cap);
  }
  return fig;
}

function renderKpi(node: KpiNode): HTMLElement {
  const c = wEl("div", "br-kpi");
  if (node.label != null && node.label !== "") {
    const l = wEl("div", "br-kpi__label");
    l.textContent = String(node.label);
    c.appendChild(l);
  }
  const v = wEl("div", "br-kpi__value");
  v.textContent = node.value == null ? "" : String(node.value);
  if (node.unit) {
    const u = wEl("span", "br-kpi__unit");
    u.textContent = " " + node.unit;
    v.appendChild(u);
  }
  c.appendChild(v);
  if (node.delta != null && node.delta !== "") {
    const ds = String(node.delta);
    const down = ds.trim().charAt(0) === "-";
    const cls = down ? "br-kpi__delta br-kpi-down" : "br-kpi__delta br-kpi-up";
    const d = wEl("div", cls);
    d.textContent = (down ? "▼ " : "▲ ") + ds;
    c.appendChild(d);
  }
  return c;
}

const LOG_CAP = 500;

function logLineEl(ln: LogLine): HTMLElement {
  const row = wEl("div", "br-log__line");
  const level = ln && ln.level ? String(ln.level).toLowerCase() : "";
  if (level) row.classList.add("br-log__line--" + level);
  row.textContent = ln && ln.text != null ? String(ln.text) : "";
  return row;
}

function logCap(node: LogNode): number {
  return typeof node.max === "number" && node.max > 0 ? node.max : LOG_CAP;
}

function atLogBottom(box: HTMLElement): boolean {
  return box.scrollHeight - box.scrollTop - box.clientHeight < 4;
}

function renderLog(node: LogNode): HTMLElement {
  const box = wEl("div", "br-log");
  box.setAttribute("data-br-log", "1");
  const lines = Array.isArray(node.lines) ? node.lines : [];
  const max = logCap(node);
  const start = lines.length > max ? lines.length - max : 0;
  const kept = lines.slice(start);
  for (const ln of kept) box.appendChild(logLineEl(ln));
  // keep the node's own array in sync with what is rendered, so appends cap right.
  node.lines = kept;
  try {
    box.scrollTop = box.scrollHeight;
  } catch {
    /* no layout (jsdom) */
  }
  return box;
}

/** Append lines to a rendered log (the `set_props {append:[…]}` fast-path):
 *  auto-scroll only when already pinned to the bottom; cap oldest-out. */
function appendLogLines(box: HTMLElement, node: LogNode, add: LogLine[]): void {
  const wasBottom = atLogBottom(box);
  const lines = Array.isArray(node.lines) ? node.lines : (node.lines = []);
  for (const ln of add) {
    lines.push(ln);
    box.appendChild(logLineEl(ln));
  }
  const max = logCap(node);
  while (lines.length > max) {
    lines.shift();
    if (box.firstChild) box.removeChild(box.firstChild);
  }
  if (wasBottom) {
    try {
      box.scrollTop = box.scrollHeight;
    } catch {
      /* no layout */
    }
  }
}

function renderFigure(node: FigureNode): HTMLElement {
  const wrap = wEl("div", "br-figure");
  const frame = document.createElement("iframe");
  frame.className = "br-figure__frame";
  // Self-contained autovis document: sandbox to scripts only, no same-origin.
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("srcdoc", String(node.html || ""));
  frame.style.width = "100%";
  frame.style.minHeight = "360px";
  frame.style.border = "0";
  wrap.appendChild(frame);
  // Grow to fit the autovis `ui-size-change` postMessage convention (capped).
  const onMsg = (ev: MessageEvent) => {
    if (!ev || (frame.contentWindow && ev.source !== frame.contentWindow)) return;
    const data = ev.data;
    if (data && data.type === "ui-size-change") {
      const h = Number(data.height);
      if (isFinite(h) && h > 0) {
        frame.style.height = Math.max(120, Math.min(1200, h)) + "px";
      }
    }
  };
  try {
    window.addEventListener("message", onMsg);
  } catch {
    /* no window (non-browser) */
  }
  return wrap;
}

// ── plot: scatter / area / box / heatmap over the themed-SVG conventions ─────

const PLOT_W = 520;
const PLOT_H = 240;

function plotFrame(inner: string, title?: string): string {
  const t = title
    ? `<text x="${PLOT_W / 2}" y="18" font-size="12" font-weight="600" text-anchor="middle" fill="var(--br-text)">${escapeHtml(
        title
      )}</text>`
    : "";
  return `<div class="br-chart"><svg viewBox="0 0 ${PLOT_W} ${PLOT_H}" width="100%" preserveAspectRatio="xMidYMid meet" role="img">${t}${inner}</svg></div>`;
}

const PLOT_PENDING = '<div class="br-chart br-chart--pending"></div>';

function renderPlot(spec: PlotSpec): string {
  if (!spec || typeof spec !== "object") return PLOT_PENDING;
  const type = spec.type || "bar";
  if (type === "scatter") return renderScatter(spec);
  if (type === "area") return renderArea(spec);
  if (type === "box") return renderBox(spec);
  if (type === "heatmap") return renderHeatmap(spec);
  // bar / line / pie reuse the existing chart renderer verbatim.
  return renderChart(JSON.stringify(spec));
}

function cleanPoints(pts?: PlotPoint[]): PlotPoint[] {
  const out: PlotPoint[] = [];
  if (!Array.isArray(pts)) return out;
  for (const d of pts) {
    if (!d) continue;
    const x = typeof d.x === "number" ? d.x : Number(d.x);
    const yr = typeof d.y === "number" ? d.y : typeof d.value === "number" ? d.value : Number(d.value);
    if (isFinite(x) && isFinite(yr)) out.push({ x: x, y: yr, label: d.label });
    if (out.length >= 2000) break;
  }
  return out;
}

function scatterSeries(spec: PlotSpec): PlotSeries[] {
  if (Array.isArray(spec.series) && spec.series.length) {
    return spec.series.map((s) => ({ name: s.name, points: cleanPoints(s.points || s.data) }));
  }
  const p = cleanPoints(spec.data);
  return p.length ? [{ points: p }] : [];
}

function renderScatter(spec: PlotSpec): string {
  const series = scatterSeries(spec);
  if (!series.length) return PLOT_PENDING;
  const padL = 44;
  const padR = 14;
  const padB = 34;
  const padT = spec.title ? 28 : 14;
  const allX: number[] = [];
  const allY: number[] = [];
  for (const s of series) {
    for (const p of s.points || []) {
      allX.push(Number(p.x));
      allY.push(Number(p.y));
    }
  }
  const xmin = Math.min(...allX);
  const xmax = Math.max(...allX);
  const ymin = Math.min(...allY);
  const ymax = Math.max(...allY);
  const xspan = xmax - xmin || 1;
  const yspan = ymax - ymin || 1;
  const plotW = PLOT_W - padL - padR;
  const plotH = PLOT_H - padT - padB;
  const sx = (v: number) => padL + (plotW * (v - xmin)) / xspan;
  const sy = (v: number) => padT + plotH * (1 - (v - ymin) / yspan);
  let body = "";
  series.forEach((s, si) => {
    const fill = CHART_PALETTE[si % CHART_PALETTE.length];
    for (const p of s.points || []) {
      body += `<circle cx="${sx(Number(p.x)).toFixed(1)}" cy="${sy(Number(p.y)).toFixed(1)}" r="3" fill="${fill}" opacity="0.82"/>`;
    }
  });
  const baseline = (padT + plotH).toFixed(1);
  const axis =
    `<line x1="${padL}" y1="${baseline}" x2="${PLOT_W - padR}" y2="${baseline}" stroke="var(--br-border)"/>` +
    `<line x1="${padL}" y1="${padT}" x2="${padL}" y2="${baseline}" stroke="var(--br-border)"/>`;
  return plotFrame(axis + body, spec.title);
}

function renderArea(spec: PlotSpec): string {
  const series = normalizeSeries(spec as ChartSpec);
  if (!series.length) return PLOT_PENDING;
  const longest = series.reduce((a, s) => (s.data.length > a.length ? s.data : a), series[0].data);
  const n = longest.length;
  const padL = 44;
  const padR = 14;
  const padB = 40;
  const padT = spec.title ? 28 : 14;
  const allVals = series.flatMap((s) => s.data.map((d) => d.value));
  const max = Math.max(...allVals, 0);
  const min = Math.min(...allVals, 0);
  const span = max - min || 1;
  const plotW = PLOT_W - padL - padR;
  const plotH = PLOT_H - padT - padB;
  const x = (i: number) => padL + (plotW * (i + 0.5)) / Math.max(n, 1);
  const y = (v: number) => padT + plotH * (1 - (v - min) / span);
  let body = "";
  series.forEach((s, si) => {
    const stroke = CHART_PALETTE[si % CHART_PALETTE.length];
    const pts = s.data.map((d, i) => `${x(i).toFixed(1)},${y(d.value).toFixed(1)}`).join(" ");
    const baseY = y(min).toFixed(1);
    const first = x(0).toFixed(1);
    const last = x(Math.max(s.data.length - 1, 0)).toFixed(1);
    body += `<polygon fill="${stroke}" opacity="0.18" points="${first},${baseY} ${pts} ${last},${baseY}"/>`;
    body += `<polyline fill="none" stroke="${stroke}" stroke-width="2.5" points="${pts}"/>`;
  });
  return plotFrame(body, spec.title);
}

function quartiles(vals: number[]): number[] {
  const s = vals.filter((v) => isFinite(v)).slice().sort((a, b) => a - b);
  if (!s.length) return [0, 0, 0, 0, 0];
  const q = (p: number) => {
    const idx = (s.length - 1) * p;
    const lo = Math.floor(idx);
    const hi = Math.ceil(idx);
    if (lo === hi) return s[lo];
    return s[lo] + (s[hi] - s[lo]) * (idx - lo);
  };
  return [s[0], q(0.25), q(0.5), q(0.75), s[s.length - 1]];
}

function seriesValues(s: PlotSeries): number[] {
  if (Array.isArray(s.values)) return s.values;
  const out: number[] = [];
  const pts = s.data || s.points || [];
  for (const d of pts) {
    const v = d && typeof d.value === "number" ? d.value : Number(d && d.value);
    if (isFinite(v)) out.push(v);
  }
  return out;
}

function boxSeries(spec: PlotSpec): PlotSeries[] {
  if (Array.isArray(spec.series) && spec.series.length) {
    return spec.series.map((s) => ({ name: s.name, values: seriesValues(s) }));
  }
  if (Array.isArray(spec.values)) return [{ values: spec.values }];
  return [];
}

function renderBox(spec: PlotSpec): string {
  const series = boxSeries(spec);
  if (!series.length) return PLOT_PENDING;
  const stats = series.map((s) => quartiles((s.values || []).slice(0, 2000)));
  const padL = 44;
  const padR = 14;
  const padB = 40;
  const padT = spec.title ? 28 : 14;
  const allv: number[] = [];
  for (const st of stats) {
    allv.push(st[0]);
    allv.push(st[4]);
  }
  const max = Math.max(...allv);
  const min = Math.min(...allv);
  const span = max - min || 1;
  const plotW = PLOT_W - padL - padR;
  const plotH = PLOT_H - padT - padB;
  const y = (v: number) => padT + plotH * (1 - (v - min) / span);
  const n = series.length;
  const slot = plotW / Math.max(n, 1);
  const bw = Math.min(slot * 0.5, 46);
  let body = "";
  stats.forEach((st, si) => {
    const cx = padL + slot * (si + 0.5);
    const color = CHART_PALETTE[si % CHART_PALETTE.length];
    const lo = st[0];
    const q1 = st[1];
    const med = st[2];
    const q3 = st[3];
    const hi = st[4];
    const half = bw / 2;
    body += `<line x1="${cx.toFixed(1)}" y1="${y(hi).toFixed(1)}" x2="${cx.toFixed(1)}" y2="${y(lo).toFixed(1)}" stroke="var(--br-text-muted)"/>`;
    body += `<rect x="${(cx - half).toFixed(1)}" y="${y(q3).toFixed(1)}" width="${bw.toFixed(1)}" height="${Math.abs(y(q1) - y(q3)).toFixed(1)}" fill="${color}" fill-opacity="0.5" stroke="${color}"/>`;
    body += `<line x1="${(cx - half).toFixed(1)}" y1="${y(med).toFixed(1)}" x2="${(cx + half).toFixed(1)}" y2="${y(med).toFixed(1)}" stroke="var(--br-text)" stroke-width="2"/>`;
    const label = series[si].name;
    if (label) {
      body += `<text x="${cx.toFixed(1)}" y="${PLOT_H - padB + 16}" font-size="10" text-anchor="middle" fill="var(--br-text-muted)">${escapeHtml(
        String(label)
      ).slice(0, 10)}</text>`;
    }
  });
  return plotFrame(body, spec.title);
}

function renderHeatmap(spec: PlotSpec): string {
  const z = Array.isArray(spec.z) ? spec.z : [];
  const rows = z.length;
  let cols = 0;
  for (const r of z) if (Array.isArray(r) && r.length > cols) cols = r.length;
  if (!rows || !cols) return PLOT_PENDING;
  let mn = Infinity;
  let mx = -Infinity;
  for (const r of z) {
    if (!Array.isArray(r)) continue;
    for (const c of r) {
      const v = Number(c);
      if (isFinite(v)) {
        if (v < mn) mn = v;
        if (v > mx) mx = v;
      }
    }
  }
  if (!isFinite(mn)) {
    mn = 0;
    mx = 1;
  }
  const span = mx - mn || 1;
  const padL = spec.yLabels ? 60 : 20;
  const padR = 14;
  const padT = spec.title ? 28 : 14;
  const padB = spec.xLabels ? 34 : 14;
  const gw = (PLOT_W - padL - padR) / cols;
  const gh = (PLOT_H - padT - padB) / rows;
  let body = "";
  for (let ri = 0; ri < rows; ri++) {
    const row = z[ri];
    for (let ci = 0; ci < cols; ci++) {
      const raw = Array.isArray(row) ? Number(row[ci]) : NaN;
      const tt = isFinite(raw) ? (raw - mn) / span : 0;
      const op = (0.12 + 0.85 * tt).toFixed(3);
      const rx = (padL + ci * gw).toFixed(1);
      const ry = (padT + ri * gh).toFixed(1);
      body += `<rect x="${rx}" y="${ry}" width="${(gw + 0.5).toFixed(1)}" height="${(gh + 0.5).toFixed(1)}" fill="var(--br-accent)" fill-opacity="${op}"/>`;
    }
  }
  if (Array.isArray(spec.yLabels)) {
    spec.yLabels.slice(0, rows).forEach((lab, ri) => {
      body += `<text x="${padL - 6}" y="${(padT + ri * gh + gh / 2 + 3).toFixed(1)}" font-size="9" text-anchor="end" fill="var(--br-text-muted)">${escapeHtml(
        String(lab)
      ).slice(0, 12)}</text>`;
    });
  }
  if (Array.isArray(spec.xLabels)) {
    spec.xLabels.slice(0, cols).forEach((lab, ci) => {
      body += `<text x="${(padL + ci * gw + gw / 2).toFixed(1)}" y="${PLOT_H - padB + 14}" font-size="9" text-anchor="middle" fill="var(--br-text-muted)">${escapeHtml(
        String(lab)
      ).slice(0, 8)}</text>`;
    });
  }
  return plotFrame(body, spec.title);
}

// ── network: a canvas force-directed graph (Barnes-Hut), jsdom-safe ──────────
// Adapted (not copied) from the BioOKF Studio engine: quadtree repulsion, spring
// edges, center gravity, warm-start, energy-based auto-freeze, zoom/pan/drag,
// viewport culling, hover focus + selection. All canvas ops are guarded so the
// engine never throws when a 2D context is unavailable (jsdom) or a stub.

const NET_FALLBACK_PALETTE = [
  "#cf6d47",
  "#4a7ec2",
  "#5bbe5e",
  "#b0842f",
  "#8a5bbe",
  "#3fa39a",
  "#c2506d",
  "#7a736c",
];

function createNetwork(spec: NetworkSpec): HTMLElement {
  const wrap = wEl("div", "br-network");
  const canvas = document.createElement("canvas");
  canvas.className = "br-network__canvas";
  wrap.appendChild(canvas);
  const controller = buildNetworkEngine(canvas, spec || {});
  (wrap as WithNet).__brNet = controller;
  (canvas as WithNet).__brNet = controller;
  return wrap;
}

function buildNetworkEngine(canvas: HTMLCanvasElement, spec: NetworkSpec): NetworkController {
  const rawNodes = Array.isArray(spec.nodes) ? spec.nodes : [];
  const rawEdges = Array.isArray(spec.edges) ? spec.edges : [];
  const encoding = spec.encoding || {};
  const physics = spec.physics || {};
  const typeColors = encoding.type_colors || {};
  const negated = new Set();
  for (const k of encoding.negated_kinds || []) negated.add(String(k));

  const BH_THETA = 0.9;
  const REPULSE = typeof physics.charge === "number" ? Math.abs(physics.charge) : 4200;
  const LINK = typeof physics.linkDistance === "number" ? physics.linkDistance : 92;
  const GRAV = typeof physics.gravity === "number" ? physics.gravity : 0.012;
  const DAMP = typeof physics.damping === "number" ? physics.damping : 0.82;
  const DPR = getDpr();

  const byId = {};
  const nodes = [];
  const total = Math.max(rawNodes.length, 1);
  rawNodes.forEach((raw, i) => {
    const id = raw && raw.id != null ? String(raw.id) : String(i);
    if (byId[id]) return;
    const ang = (i / total) * Math.PI * 2;
    const rad = 150 + (i % 6) * 18;
    const px = Math.cos(ang) * rad;
    const py = Math.sin(ang) * rad;
    const node = {
      id: id,
      label: raw && raw.label != null ? String(raw.label) : id,
      type: raw && raw.type != null ? String(raw.type) : "",
      size: raw && typeof raw.size === "number" ? raw.size : 1,
      color: raw && raw.color ? String(raw.color) : "",
      x: px,
      y: py,
      vx: 0,
      vy: 0,
      deg: 0,
      hub: false,
    };
    byId[id] = node;
    nodes.push(node);
  });

  const edges = [];
  for (const e of rawEdges) {
    if (!e) continue;
    const s = String(e.source);
    const t = String(e.target);
    if (!byId[s] || !byId[t]) continue;
    const kind = e.kind != null ? String(e.kind) : "";
    const neg = negated.has(kind);
    const dashed = e.style === "dashed" || neg;
    edges.push({ source: s, target: t, kind: kind, label: e.label != null ? String(e.label) : "", dashed: dashed, neg: neg });
    byId[s].deg++;
    byId[t].deg++;
  }

  // hubs = top-6 by degree, drawn larger.
  const ranked = nodes.slice().sort((a, b) => b.deg - a.deg);
  const hubSet = new Set();
  for (let i = 0; i < ranked.length && i < 6; i++) hubSet.add(ranked[i].id);
  for (const n of nodes) n.hub = hubSet.has(n.id);

  // prebuilt neighbor map → O(1) hover focus.
  const neighbors = {};
  for (const n of nodes) neighbors[n.id] = new Set();
  for (const e of edges) {
    neighbors[e.source].add(e.target);
    neighbors[e.target].add(e.source);
  }

  const view = { k: 1, x: 0, y: 0 };
  let alpha = 1;
  let settled = false;
  let settledFrames = 0;
  let alive = true;
  let W = 0;
  let H = 0;
  let drag = null;
  let panning = null;
  let hover = null;
  let selected = null;
  let moved = false;
  let tip = null;
  const listeners = [];
  if (typeof spec.onSelect === "function") listeners.push(spec.onSelect);

  let ctx = null;
  try {
    ctx = canvas.getContext("2d");
  } catch {
    ctx = null;
  }

  const typeIndex = {};
  let typeSeq = 0;
  function colorFor(n) {
    if (n.color) return n.color;
    const ty = n.type || "";
    if (typeColors && typeColors[ty]) return typeColors[ty];
    if (!(ty in typeIndex)) {
      typeIndex[ty] = typeSeq % NET_FALLBACK_PALETTE.length;
      typeSeq++;
    }
    return NET_FALLBACK_PALETTE[typeIndex[ty]];
  }

  class QNode {
    constructor(x, y, w, h) {
      this.x = x;
      this.y = y;
      this.w = w;
      this.h = h;
      this.mass = 0;
      this.cmx = 0;
      this.cmy = 0;
      this.body = null;
      this.children = null;
    }
    quad(nx, ny, hw, hh) {
      return (nx >= this.x + hw ? 1 : 0) | (ny >= this.y + hh ? 2 : 0);
    }
    insert(nx, ny, node) {
      if (this.mass === 0 && !this.children) {
        this.body = node;
        this.mass = 1;
        this.cmx = nx;
        this.cmy = ny;
        return;
      }
      if (!this.children) {
        const ob = this.body;
        this.body = null;
        this.children = [null, null, null, null];
        const hw0 = this.w / 2;
        const hh0 = this.h / 2;
        const qi0 = this.quad(ob.x, ob.y, hw0, hh0);
        this.children[qi0] = new QNode(this.x + (qi0 & 1 ? hw0 : 0), this.y + (qi0 & 2 ? hh0 : 0), hw0, hh0);
        this.children[qi0].insert(ob.x, ob.y, ob);
      }
      const hw = this.w / 2;
      const hh = this.h / 2;
      const qi = this.quad(nx, ny, hw, hh);
      if (!this.children[qi]) {
        this.children[qi] = new QNode(this.x + (qi & 1 ? hw : 0), this.y + (qi & 2 ? hh : 0), hw, hh);
      }
      this.children[qi].insert(nx, ny, node);
      this.mass++;
      this.cmx = (this.cmx * (this.mass - 1) + nx) / this.mass;
      this.cmy = (this.cmy * (this.mass - 1) + ny) / this.mass;
    }
    force(nx, ny, node, a) {
      if (this.mass === 0) return;
      if (this.body && this.body !== node) {
        let dx = nx - this.body.x;
        let dy = ny - this.body.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) {
          d2 = 0.01;
          dx = Math.random() - 0.5;
          dy = Math.random() - 0.5;
        }
        const d = Math.sqrt(d2);
        const rep = REPULSE / d2;
        node.vx += (dx / d) * rep * a;
        node.vy += (dy / d) * rep * a;
        return;
      }
      if (this.children) {
        const dx = nx - this.cmx;
        const dy = ny - this.cmy;
        const d2 = dx * dx + dy * dy;
        const d = Math.sqrt(d2) || 0.01;
        const s = Math.max(this.w, this.h);
        if (s / d < BH_THETA) {
          const rep = (REPULSE * this.mass) / d2;
          node.vx += (dx / d) * rep * a;
          node.vy += (dy / d) * rep * a;
        } else {
          for (const ch of this.children) if (ch) ch.force(nx, ny, node, a);
        }
      }
    }
  }

  function tick(a) {
    const M = nodes.length;
    if (!M) return;
    let mnx = 1e9;
    let mny = 1e9;
    let mxx = -1e9;
    let mxy = -1e9;
    for (const n of nodes) {
      if (n.x < mnx) mnx = n.x;
      if (n.y < mny) mny = n.y;
      if (n.x > mxx) mxx = n.x;
      if (n.y > mxy) mxy = n.y;
    }
    const qs = Math.max(mxx - mnx || 1, mxy - mny || 1);
    const root = new QNode(mnx - qs * 0.1, mny - qs * 0.1, qs * 1.2, qs * 1.2);
    for (const n of nodes) root.insert(n.x, n.y, n);
    for (const n of nodes) root.force(n.x, n.y, n, a);
    // edge springs — rest length scaled by endpoint size.
    for (const e of edges) {
      const s = byId[e.source];
      const t = byId[e.target];
      if (!s || !t) continue;
      const dx = t.x - s.x;
      const dy = t.y - s.y;
      const d = Math.sqrt(dx * dx + dy * dy) || 0.01;
      const rest = LINK * (1 + (nodeR(s) + nodeR(t)) / 24);
      const f = (d - rest) * 0.045 * a;
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      s.vx += fx;
      s.vy += fy;
      t.vx -= fx;
      t.vy -= fy;
    }
    for (const n of nodes) {
      n.vx += -n.x * GRAV * a;
      n.vy += -n.y * GRAV * a;
    }
    for (const n of nodes) {
      if (n === drag) {
        n.vx = 0;
        n.vy = 0;
        continue;
      }
      n.x += n.vx;
      n.y += n.vy;
      n.vx *= DAMP;
      n.vy *= DAMP;
    }
  }

  function measure() {
    const rect = canvas.getBoundingClientRect ? canvas.getBoundingClientRect() : { width: 0, height: 0 };
    let cw = canvas.clientWidth || rect.width || 0;
    let ch = canvas.clientHeight || rect.height || 0;
    if (!cw) cw = 320;
    if (!ch) ch = 320;
    W = cw;
    H = ch;
    const needW = Math.round(cw * DPR);
    const needH = Math.round(ch * DPR);
    if (needW > 0 && needH > 0 && (canvas.width !== needW || canvas.height !== needH)) {
      canvas.width = needW;
      canvas.height = needH;
    }
  }

  function toScreen(x, y) {
    return [x * view.k + view.x + W / 2, y * view.k + view.y + H / 2];
  }
  function toWorld(sx, sy) {
    return [(sx - W / 2 - view.x) / view.k, (sy - H / 2 - view.y) / view.k];
  }
  function nodeR(n) {
    const base = n.hub ? 9 : 6;
    const scale = n.size > 1 ? Math.min(2, Math.sqrt(n.size)) : 1;
    return base * scale;
  }

  function fitView() {
    if (!nodes.length) return;
    let mnx = 1e9;
    let mny = 1e9;
    let mxx = -1e9;
    let mxy = -1e9;
    for (const n of nodes) {
      if (n.x < mnx) mnx = n.x;
      if (n.y < mny) mny = n.y;
      if (n.x > mxx) mxx = n.x;
      if (n.y > mxy) mxy = n.y;
    }
    const pad = 60;
    const gw = mxx - mnx || 1;
    const gh = mxy - mny || 1;
    const uw = Math.max(80, W - pad * 2);
    const uh = Math.max(80, H - pad * 2);
    const k = Math.min(uw / gw, uh / gh, 1.6);
    view.k = k > 0 && isFinite(k) ? k : 1;
    view.x = -((mnx + mxx) / 2) * view.k;
    view.y = -((mny + mxy) / 2) * view.k;
  }

  function drawEdge(x1, y1, x2, y2, e, dim) {
    let col = "rgba(120,128,106,0.35)";
    if (e.neg) col = dim ? "rgba(207,80,71,0.16)" : "rgba(207,80,71,0.62)";
    else if (dim) col = "rgba(120,128,106,0.1)";
    ctx.save();
    if (e.dashed) ctx.setLineDash([4, 3]);
    else ctx.setLineDash([]);
    ctx.strokeStyle = col;
    ctx.lineWidth = e.neg ? 1.1 : 0.9;
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
    ctx.restore();
    if (e.neg && !dim) {
      // reddish strike at the midpoint reinforces a negated relationship.
      const mx = (x1 + x2) / 2;
      const my = (y1 + y2) / 2;
      ctx.save();
      ctx.strokeStyle = "rgba(207,80,71,0.9)";
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      ctx.moveTo(mx - 4, my - 4);
      ctx.lineTo(mx + 4, my + 4);
      ctx.stroke();
      ctx.restore();
    }
  }

  function drawNode(n, a, isFocus) {
    const p = toScreen(n.x, n.y);
    const r = Math.max(2, nodeR(n) * view.k);
    ctx.globalAlpha = a;
    ctx.beginPath();
    ctx.arc(p[0], p[1], r, 0, 7);
    ctx.fillStyle = colorFor(n);
    ctx.fill();
    ctx.lineWidth = isFocus ? 2 : 1;
    ctx.strokeStyle = isFocus ? "#cf6d47" : "rgba(20,18,12,0.55)";
    ctx.stroke();
    ctx.globalAlpha = 1;
  }

  function drawLabels(focusId, focusN) {
    ctx.textBaseline = "middle";
    ctx.fillStyle = "rgba(40,34,25,0.95)";
    for (const n of nodes) {
      const isFocus = focusId === n.id;
      const isNb = focusN ? focusN.has(n.id) : false;
      const show = isFocus || (n.hub && !focusId) || isNb || view.k >= 1.5;
      if (!show) continue;
      const p = toScreen(n.x, n.y);
      const r = Math.max(2, nodeR(n) * view.k);
      const raw = n.label.length > 28 ? n.label.slice(0, 27) + "…" : n.label;
      ctx.font = (n.hub ? "600 " : "450 ") + "11px -apple-system,system-ui,sans-serif";
      ctx.fillText(raw, p[0] + r + 5, p[1]);
    }
  }

  function drawReal() {
    if (typeof ctx.setTransform === "function") ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    if (typeof ctx.clearRect === "function") ctx.clearRect(0, 0, W, H);
    const focusId = (selected && selected.id) || (hover && hover.id) || null;
    const focusN = focusId ? neighbors[focusId] : null;
    for (const e of edges) {
      const s = byId[e.source];
      const t = byId[e.target];
      if (!s || !t) continue;
      const p1 = toScreen(s.x, s.y);
      const p2 = toScreen(t.x, t.y);
      const dim = focusId ? e.source !== focusId && e.target !== focusId : false;
      // viewport cull: skip edges fully off-screen when not focused.
      if (!focusId && offScreen(p1) && offScreen(p2)) continue;
      drawEdge(p1[0], p1[1], p2[0], p2[1], e, dim);
    }
    for (const n of nodes) {
      const p = toScreen(n.x, n.y);
      if (offScreen(p)) continue;
      const isFocus = focusId === n.id;
      const isNb = focusN ? focusN.has(n.id) : false;
      const a = focusId && !isFocus && !isNb ? 0.22 : 1;
      drawNode(n, a, isFocus);
    }
    drawLabels(focusId, focusN);
  }

  function offScreen(p) {
    return p[0] < -80 || p[0] > W + 80 || p[1] < -80 || p[1] > H + 80;
  }

  function draw() {
    if (!ctx) return;
    try {
      drawReal();
    } catch {
      /* stub/partial 2D context (jsdom) — physics + selection still run */
    }
  }

  function reheat(v) {
    if (settled || alpha < v) {
      alpha = Math.max(alpha, v);
      settled = false;
      settledFrames = 0;
    }
  }

  function loop() {
    if (!alive) return;
    measure();
    if (alpha > 0.005) {
      tick(alpha);
      alpha *= 0.94;
      let ke = 0;
      for (const n of nodes) ke += n.vx * n.vx + n.vy * n.vy;
      if (ke < nodes.length * 0.008) {
        settledFrames++;
        if (settledFrames > 30) {
          alpha = 0;
          settled = true;
        }
      } else {
        settledFrames = 0;
      }
    }
    draw();
    schedule();
  }
  function schedule() {
    if (!alive) return;
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(loop);
    else if (typeof setTimeout === "function") setTimeout(loop, 16);
  }

  function pickNode(sx, sy) {
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      const p = toScreen(n.x, n.y);
      const r = Math.max(2, nodeR(n) * view.k) + 4;
      const dx = sx - p[0];
      const dy = sy - p[1];
      if (dx * dx + dy * dy <= r * r) return n;
    }
    return null;
  }
  function localXY(ev) {
    const rect = canvas.getBoundingClientRect ? canvas.getBoundingClientRect() : { left: 0, top: 0 };
    return [(ev.clientX || 0) - rect.left, (ev.clientY || 0) - rect.top];
  }
  function ensureTip() {
    if (tip) return tip;
    tip = document.createElement("div");
    tip.className = "br-network__tip";
    tip.style.display = "none";
    const host = canvas.parentElement;
    if (host) host.appendChild(tip);
    return tip;
  }
  function showTooltip(n, xy) {
    const el = ensureTip();
    if (!n) {
      el.style.display = "none";
      return;
    }
    el.textContent = n.label + (n.type ? " · " + n.type : "");
    el.style.display = "block";
    const host = el.parentElement;
    const padding = 8;
    const offset = 12;
    const hostWidth = host ? host.clientWidth : W;
    const hostHeight = host ? host.clientHeight : H;
    const maxLeft = Math.max(padding, hostWidth - el.offsetWidth - padding);
    const maxTop = Math.max(padding, hostHeight - el.offsetHeight - padding);
    const left = Math.min(Math.max(padding, xy[0] + offset), maxLeft);
    const preferredTop = xy[1] + offset;
    const flippedTop = xy[1] - el.offsetHeight - offset;
    const top =
      preferredTop <= maxTop
        ? preferredTop
        : Math.min(Math.max(padding, flippedTop), maxTop);
    el.style.left = left + "px";
    el.style.top = top + "px";
  }
  function onDown(ev) {
    const xy = localXY(ev);
    moved = false;
    const n = pickNode(xy[0], xy[1]);
    if (n) {
      drag = n;
      reheat(1);
    } else {
      panning = { x: xy[0], y: xy[1] };
    }
  }
  function onMove(ev) {
    const xy = localXY(ev);
    if (drag) {
      const w = toWorld(xy[0], xy[1]);
      drag.x = w[0];
      drag.y = w[1];
      moved = true;
      reheat(0.3);
      return;
    }
    if (panning) {
      view.x += xy[0] - panning.x;
      view.y += xy[1] - panning.y;
      panning = { x: xy[0], y: xy[1] };
      moved = true;
      return;
    }
    const n = pickNode(xy[0], xy[1]);
    const prev = hover && hover.id;
    hover = n;
    if (prev !== (n && n.id)) showTooltip(n, xy);
  }
  function onUp(ev) {
    if (!drag && !panning) return;
    const xy = localXY(ev);
    if (!moved) {
      const n = pickNode(xy[0], xy[1]);
      selectNode(n ? n.id : null);
    }
    drag = null;
    panning = null;
  }
  function onWheel(ev) {
    if (ev.preventDefault) ev.preventDefault();
    const xy = localXY(ev);
    const w = toWorld(xy[0], xy[1]);
    const nk = Math.max(0.25, Math.min(5, view.k * Math.exp(-(ev.deltaY || 0) * 0.0014)));
    view.k = nk;
    view.x = xy[0] - W / 2 - w[0] * view.k;
    view.y = xy[1] - H / 2 - w[1] * view.k;
    reheat(0.25);
  }

  function selectNode(id) {
    selected = id != null && byId[id] ? byId[id] : null;
    const chosen = selected ? selected.id : null;
    try {
      // Bubbles so the runtime's document-level listener can auto-emit a
      // `node_selected` signal; author listeners on the canvas still fire.
      const ev = new CustomEvent("br-network-select", { detail: { id: chosen }, bubbles: true });
      canvas.dispatchEvent(ev);
    } catch {
      /* CustomEvent unavailable */
    }
    for (const cb of listeners) {
      try {
        cb(chosen);
      } catch {
        /* author callback errors are non-fatal */
      }
    }
  }

  function snapshot() {
    const out = {};
    for (const n of nodes) out[n.id] = { x: n.x, y: n.y };
    return out;
  }
  function adopt(prev) {
    if (!prev || typeof prev !== "object") return;
    let any = false;
    for (const n of nodes) {
      const p = prev[n.id];
      if (p && isFinite(p.x) && isFinite(p.y)) {
        n.x = p.x;
        n.y = p.y;
        n.vx = 0;
        n.vy = 0;
        any = true;
      }
    }
    if (any) {
      fitView();
      reheat(0.3);
    }
  }

  let ro = null;
  function destroy() {
    alive = false;
    try {
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      canvas.removeEventListener("wheel", onWheel);
    } catch {
      /* ignore */
    }
    if (ro) {
      try {
        ro.disconnect();
      } catch {
        /* ignore */
      }
    }
  }

  // ── init: warm-start layout, wire interaction, start the loop ──
  measure();
  for (let i = 0; i < 20; i++) tick(0.9 * Math.pow(0.985, i) + 0.02);
  fitView();
  try {
    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
  } catch {
    /* no event target */
  }
  if (typeof ResizeObserver !== "undefined") {
    try {
      ro = new ResizeObserver(() => {
        const wasZero = W === 0 || H === 0;
        measure();
        if (wasZero && nodes.length) fitView();
      });
      ro.observe(canvas);
    } catch {
      ro = null;
    }
  }
  schedule();

  return {
    select: (id) => selectNode(id),
    positions: () => snapshot(),
    adopt: (prev) => adopt(prev),
    destroy: () => destroy(),
  };
}

function getDpr(): number {
  try {
    return window.devicePixelRatio || 1;
  } catch {
    return 1;
  }
}

// ── author component registry (Task 3) ───────────────────────────────────────

class ComponentRegistry {
  private defs: Map<string, ComponentDef> = new Map();
  register(name: string, def: ComponentDef): void {
    if (name && def && typeof def.mount === "function") this.defs.set(String(name), def);
  }
  get(name: string): ComponentDef | undefined {
    return this.defs.get(String(name));
  }
}

// ── instance morphing helpers (Task 1) ───────────────────────────────────────

/** The network controller attached to a rendered `network` instance, if any. */
function readNet(el: HTMLElement): NetworkController | null {
  const n = (el as WithNet).__brNet;
  return n ? n : null;
}

/** Carry sim positions from an old network instance to its replacement (matched
 *  by node id), then tear the old one down so its animation loop stops. */
function adoptNetwork(oldEl: HTMLElement, newEl: HTMLElement): void {
  const oldNet = readNet(oldEl);
  const newNet = readNet(newEl);
  if (oldNet && newNet) {
    try {
      newNet.adopt(oldNet.positions());
    } catch {
      /* ignore */
    }
  }
  if (oldNet) {
    try {
      oldNet.destroy();
    } catch {
      /* ignore */
    }
  }
}

function shallowMergeNode(node: WidgetNode, props: AnyRecord): WidgetNode {
  const out = Object.assign({}, node, props);
  return out as WidgetNode;
}

/** Snapshot the focused input inside `root` so a re-render can restore it. */
function captureFocus(root: HTMLElement): FocusSnap | null {
  const active = document.activeElement as HTMLElement | null;
  if (!active || !root.contains(active)) return null;
  const tag = active.tagName ? active.tagName.toLowerCase() : "";
  if (tag !== "input" && tag !== "textarea" && tag !== "select") return null;
  const field = active as HTMLInputElement;
  let start = -1;
  let end = -1;
  try {
    start = field.selectionStart == null ? -1 : field.selectionStart;
    end = field.selectionEnd == null ? -1 : field.selectionEnd;
  } catch {
    /* selection not supported on this input type */
  }
  return {
    name: active.getAttribute("name") || "",
    start: start,
    end: end,
    value: field.value == null ? "" : String(field.value),
  };
}

/** Re-focus the same-named input inside a freshly re-rendered subtree, keeping
 *  the caret/selection and (when the new field is blank) the in-progress value. */
function restoreFocus(root: HTMLElement, snap: FocusSnap | null): void {
  if (!snap) return;
  let sel = null;
  const list = root.querySelectorAll("input, textarea, select");
  for (let i = 0; i < list.length; i++) {
    const el = list[i] as HTMLElement;
    if ((el.getAttribute("name") || "") === snap.name) {
      sel = el;
      break;
    }
  }
  if (!sel) return;
  const field = sel as HTMLInputElement;
  try {
    field.focus();
    if (snap.value && (field.value == null || field.value === "")) field.value = snap.value;
    if (snap.start >= 0 && typeof field.setSelectionRange === "function") {
      field.setSelectionRange(snap.start, snap.end);
    }
  } catch {
    /* focus/selection unsupported */
  }
}

/** Capture scroll offsets of scrolled descendants, keyed by DOM position, so an
 *  in-place re-render of a structurally-identical subtree keeps them. */
function captureScroll(root: HTMLElement): ScrollSnap {
  const idx: number[] = [];
  const tops: number[] = [];
  const lefts: number[] = [];
  const all = root.querySelectorAll("*");
  for (let i = 0; i < all.length; i++) {
    const el = all[i] as HTMLElement;
    const st = el.scrollTop || 0;
    const sl = el.scrollLeft || 0;
    if (st > 0 || sl > 0) {
      idx.push(i);
      tops.push(st);
      lefts.push(sl);
    }
  }
  return { idx: idx, tops: tops, lefts: lefts };
}

function restoreScroll(root: HTMLElement, snap: ScrollSnap): void {
  if (!snap || !snap.idx.length) return;
  const all = root.querySelectorAll("*");
  for (let k = 0; k < snap.idx.length; k++) {
    const el = all[snap.idx[k]] as HTMLElement;
    if (!el) continue;
    try {
      el.scrollTop = snap.tops[k];
      el.scrollLeft = snap.lefts[k];
    } catch {
      /* ignore */
    }
  }
}

// ---------------------------------------------------------------------------
// Agent-driven UI runtime
// ---------------------------------------------------------------------------
// The agent's `ui_*` tools emit `{type:"ui", cmd:…}` frames; this applies them to
// the page. Everything it creates is confined to elements it owns (`.br-dock`,
// `.br-panel`, `.br-toasts`, `.br-modal-host`) plus the CSS custom properties on
// `:root` — an app's own markup is only ever *targeted*, never rewritten, unless
// the agent explicitly names it.

export interface UiCommand {
  type?: string;
  cmd: string;
  id?: string;
  title?: string | null;
  place?: string;
  body?: WidgetNode[];
  collapsible?: boolean;
  remove?: boolean;
  target?: string;
  mode?: string;
  note?: string | null;
  scroll?: boolean;
  accent?: string | null;
  density?: string | null;
  preset?: string;
  sidebarWidth?: number | null;
  message?: string;
  level?: string;
  timeoutMs?: number;
  state?: Record<string, unknown>;
  // ── shared reactive state document (SDK v2) ──
  // Frame version stamp (`"v": 1`), tolerated on every ui frame.
  v?: number;
  // `state` frame modes: "snapshot" replaces the doc; "patch" applies RFC-6902
  // ops; absent `mode` (legacy) treats `state` as a snapshot of unknown version.
  doc?: unknown;
  version?: number;
  patch?: unknown;
  requestId?: string;
  prompt?: string;
  submitLabel?: string;
  fields?: AskFieldSpec[];
  // ── ui_patch (SDK v2): incremental instance edits by id ──
  ops?: unknown[];
  // ── app_call (SDK v2 Phase 3): invoke an author-registered action ──
  // Rides the `ui` frame (`cmd:"app_call"`) but is dispatched to `br.actions`.
  callId?: string;
  action?: string;
  args?: unknown;
  // ── theme packs + layout grammar (SDK v2 Phase 5, §3.6) ──
  // `theme` gains `pack` (a curated token set); `layout` gains `areas`/`sizes`
  // (a bounded grid grammar) alongside the existing preset aliases.
  pack?: string;
  areas?: unknown[];
  sizes?: unknown[];
  // ── presence layer (SDK v2 Phase 5, §3.5) ──
  // Any agent frame may carry `narrate` — shown verbatim in the presence chip.
  narrate?: string;
  // ── multi-agent (SDK v2 §3.8) ──
  // A worker profile's ui frame carries `agent`; presence attributes it to that
  // profile ("<profile> · …") instead of the generic "AI · …".
  agent?: string;
  // ── ui_suggest (SDK v2 §3.5): non-blocking mixed-initiative chips ──
  chips?: SuggestChip[];
  // The clicked chip's label, carried on the synthetic `suggest` command the
  // runtime hands `onCommand` when a chip has no prompt of its own.
  label?: string;
}

/** One `ui_suggest` chip: a label plus an optional prompt to send on click. */
export interface SuggestChip {
  label: string;
  prompt?: string;
}

export interface AskFieldSpec {
  name: string;
  label?: string;
  type?: string;
  options?: string[];
  value?: string;
  placeholder?: string;
}

// Named aliases (see the note near `Listener`): the no-esbuild fallback stripper
// keys off an uppercase/primitive leading type token, so a bare
// `(cmd: UiCommand) => void` annotation would survive into the emitted JS.
type StateListener = (state: Record<string, unknown>) => void;
type CommandListener = (cmd: UiCommand) => void;
type KeyHandler = (e: KeyboardEvent) => void;
type FieldGetters = Map<string, () => string | boolean>;
// Same reason: `br.state` callback/return types go through named aliases so the
// fallback stripper drops the annotation (an inline `(v: unknown) => void` has
// no leading uppercase/primitive token and would survive into the JS).
type ValueSub = (value: unknown) => void;
type DocUpdater = (doc: unknown) => unknown;
type Unsub = () => void;

/** Where a `place` maps in the DOM. `dock` is the always-available drawer. */
const DOCK_PLACES: Record<string, string> = {
  dock: "br-dock--right",
  right: "br-dock--right",
  left: "br-dock--left",
  bottom: "br-dock--bottom",
};

// ── Shared reactive state: JSON Pointer + RFC-6902 (dependency-free) ─────────
// A small, self-contained slice of RFC-6901 (pointers) and RFC-6902 (patch)
// sufficient for the state channel: the binding index needs only pointer
// resolution, and the client mirrors the server's doc by applying add/replace/
// remove/move/copy/test ops. Not a general JSON-Patch library — deliberately
// minimal and side-effect-contained.

// Named aliases so the fallback stripper drops `as AnyRecord` / `as AnyArray`
// casts (generic/bracketed casts like `as Record<string, unknown>` trip its
// comma/`]`-terminated scanner).
type AnyRecord = Record<string, unknown>;
type AnyArray = unknown[];

/** One declarative binding: an element + how it consumes a pointer value. */
/**
 * Read a form control's value in the type the state document should hold.
 *
 * A `<input type=range>` yields the STRING "0.37"; writing that into state and
 * then comparing it against a number is a silent-divergence bug waiting to
 * happen, so range/number controls coerce to a real number here.
 */
function readControlValue(el: HTMLElement): unknown {
  const tag = el.tagName.toLowerCase();
  if (tag === "input") {
    const input = el as HTMLInputElement;
    const type = (input.type || "text").toLowerCase();
    if (type === "checkbox") return input.checked;
    if (type === "range" || type === "number") {
      const n = Number(input.value);
      return Number.isFinite(n) ? n : input.value;
    }
    return input.value;
  }
  if (tag === "select") return (el as HTMLSelectElement).value;
  if (tag === "textarea") return (el as HTMLTextAreaElement).value;
  return (el as HTMLInputElement).value;
}

/** Push a state value back into a form control (doc → DOM). */
function writeControlValue(el: HTMLElement, value: unknown): void {
  const tag = el.tagName.toLowerCase();
  if (tag === "input") {
    const input = el as HTMLInputElement;
    if ((input.type || "").toLowerCase() === "checkbox") {
      input.checked = Boolean(value);
      return;
    }
    input.value = value === null || value === undefined ? "" : String(value);
    return;
  }
  if (tag === "select" || tag === "textarea") {
    (el as HTMLSelectElement).value =
      value === null || value === undefined ? "" : String(value);
  }
}

interface BindEntry {
  el: Element;
  kind: string; // "text" | "attr" | "show" | "model"
  pointer: string;
  attr: string; // attribute name for kind "attr"
}

/** One `state.subscribe` registration. `last` is the JSON of the pointer's
 *  value at the previous fire, so we only notify on real change. */
interface StateSub {
  pointer: string;
  fn: ValueSub;
  last: string;
}

/** RFC-6901: parse a JSON Pointer into its (unescaped) reference tokens.
 *  "" → whole document ([]); "/a/b" → ["a","b"]; "/" → [""]. */
function parsePointer(pointer: string): string[] {
  if (!pointer) return [];
  const parts = pointer.split("/");
  parts.shift(); // drop the empty segment before the leading "/"
  const out: string[] = [];
  for (const p of parts) out.push(p.replace(/~1/g, "/").replace(/~0/g, "~"));
  return out;
}

/** Resolve a JSON Pointer against `doc`; `undefined` if any step is missing. */
function pointerGet(doc: unknown, pointer: string): unknown {
  const tokens = parsePointer(pointer);
  let cur = doc;
  for (const tok of tokens) {
    if (cur == null) return undefined;
    if (Array.isArray(cur)) {
      const arr = cur as AnyArray;
      const idx = tok === "-" ? arr.length : parseInt(tok, 10);
      cur = arr[idx];
    } else if (typeof cur === "object") {
      cur = (cur as AnyRecord)[tok];
    } else {
      return undefined;
    }
  }
  return cur;
}

/** The container that holds a pointer's last token (its parent). */
function pointerParent(root: unknown, tokens: string[]): unknown {
  let cur = root;
  for (let i = 0; i < tokens.length - 1; i++) {
    if (cur == null) return null;
    if (Array.isArray(cur)) {
      cur = (cur as AnyArray)[parseInt(tokens[i], 10)];
    } else if (typeof cur === "object") {
      cur = (cur as AnyRecord)[tokens[i]];
    } else {
      return null;
    }
  }
  return cur;
}

function ptrAdd(root: unknown, path: string, value: unknown): unknown {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return value; // replace whole doc
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent as AnyArray;
    const idx = key === "-" ? arr.length : parseInt(key, 10);
    if (isFinite(idx)) arr.splice(idx, 0, value);
  } else {
    (parent as AnyRecord)[key] = value;
  }
  return root;
}

function ptrReplace(root: unknown, path: string, value: unknown): unknown {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return value;
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent as AnyArray;
    const idx = parseInt(key, 10);
    if (idx >= 0 && idx < arr.length) arr[idx] = value;
  } else {
    (parent as AnyRecord)[key] = value;
  }
  return root;
}

function ptrRemove(root: unknown, path: string): unknown {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return null;
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent as AnyArray;
    const idx = parseInt(key, 10);
    if (idx >= 0 && idx < arr.length) arr.splice(idx, 1);
  } else {
    delete (parent as AnyRecord)[key];
  }
  return root;
}

function deepClone(v: unknown): unknown {
  if (v == null || typeof v !== "object") return v;
  try {
    return JSON.parse(JSON.stringify(v));
  } catch {
    return v;
  }
}

function jsonEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Apply RFC-6902 ops to a *clone* of `doc`, returning the new doc. A failed
 *  `test` (or a throwing op) aborts by propagating, so the caller keeps the
 *  pre-patch doc. */
function applyPatch(doc: unknown, ops: unknown): unknown {
  let root = deepClone(doc);
  const list = Array.isArray(ops) ? ops : [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const op = raw as AnyRecord;
    const kind = String(op.op || "");
    const path = typeof op.path === "string" ? op.path : "";
    if (kind === "add") {
      root = ptrAdd(root, path, deepClone(op.value));
    } else if (kind === "replace") {
      root = ptrReplace(root, path, deepClone(op.value));
    } else if (kind === "remove") {
      root = ptrRemove(root, path);
    } else if (kind === "move") {
      const from = typeof op.from === "string" ? op.from : "";
      const moved = deepClone(pointerGet(root, from));
      root = ptrRemove(root, from);
      root = ptrAdd(root, path, moved);
    } else if (kind === "copy") {
      const src = typeof op.from === "string" ? op.from : "";
      const copied = deepClone(pointerGet(root, src));
      root = ptrAdd(root, path, copied);
    } else if (kind === "test") {
      if (!jsonEqual(pointerGet(root, path), op.value)) {
        throw new Error("json-patch test failed at " + path);
      }
    }
  }
  return root;
}

/** The set of pointers a patch touched (its `path`s and any `from`s). */
function patchedPaths(patch: unknown): string[] {
  const out: string[] = [];
  const list = Array.isArray(patch) ? patch : [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const op = raw as AnyRecord;
    if (typeof op.path === "string") out.push(op.path);
    if (typeof op.from === "string") out.push(op.from);
  }
  return out;
}

/** Whether a binding pointer is affected by a set of patched paths: it equals,
 *  is a prefix of (ancestor), or is prefixed by (descendant) some path. Root
 *  ("") on either side touches everything. */
function pointerAffected(bindPointer: string, paths: string[]): boolean {
  for (const p of paths) {
    if (bindPointer === p) return true;
    if (bindPointer === "" || p === "") return true;
    if (bindPointer.indexOf(p + "/") === 0) return true;
    if (p.indexOf(bindPointer + "/") === 0) return true;
  }
  return false;
}

/** Text rendering for a bound value: null/undefined → "", objects → JSON,
 *  everything else → String(value). Never HTML. */
function bindText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "object") {
    try {
      return JSON.stringify(value);
    } catch {
      return String(value);
    }
  }
  return String(value);
}

/** JSON of a value, with a stable sentinel for `undefined` so a subscriber's
 *  change comparison never confuses "absent" with a real string. */
function stateStringify(value: unknown): string {
  const s = JSON.stringify(value);
  return s === undefined ? "\u0000undef" : s;
}

function asRecord(v: unknown): AnyRecord {
  if (v && typeof v === "object" && !Array.isArray(v)) return v as AnyRecord;
  return {};
}

// Attributes an author may bind via `data-br-bind-attr`. `style` and any `on*`
// handler are FORBIDDEN (they execute); `aria-*`/`data-*` are allowed too.
const BIND_ATTR_ALLOW: Record<string, boolean> = {
  href: true,
  src: true,
  title: true,
  alt: true,
  value: true,
  placeholder: true,
  disabled: true,
  hidden: true,
  class: true,
};

/** A non-executing URL for `href`/`src`: relative paths, fragments, `https:` or
 *  `mailto:`. `javascript:`, `data:`, and other schemes are refused. */
function isSafeBindUrl(u: string): boolean {
  const s = (u || "").trim();
  if (s === "") return true;
  if (s.charAt(0) === "#" || s.charAt(0) === "/") return true;
  // No scheme (`word:`) before a path separator → a relative reference.
  if (!/^[a-zA-Z][a-zA-Z0-9+.\-]*:/.test(s)) return true;
  return /^https:/i.test(s) || /^mailto:/i.test(s);
}

// ── theme packs (SDK v2 Phase 5, §3.6) ───────────────────────────────────────
// The curated pack ids. `biorouter` is the base (no `data-br-pack` overrides in
// theme.css); the other five are token-set layers. An unknown pack is ignored.
const KNOWN_PACKS: Record<string, boolean> = {
  biorouter: true,
  clinical: true,
  "lab-notebook": true,
  terminal: true,
  journal: true,
  midnight: true,
};

// ── layout grammar (SDK v2 Phase 5, §3.6) ────────────────────────────────────
// A bounded column-size vocabulary so the agent cannot inject raw CSS. A number
// becomes `<n>fr`; a recognised `<n><unit>` / keyword passes through; anything
// else (and any gap) defaults to `1fr`.
function sizeToken(v: unknown): string {
  if (typeof v === "number" && isFinite(v) && v > 0) return v + "fr";
  if (typeof v === "string") {
    const s = v.trim();
    if (/^\d+(\.\d+)?(fr|px|%|em|rem|vw|vh|ch)$/.test(s)) return s;
    if (s === "auto" || s === "min-content" || s === "max-content") return s;
  }
  return "1fr";
}

/** The unique grid-area names across the rows, in first-seen order, skipping the
 *  `.` empty-cell token. */
function uniqueAreaNames(rows: string[]): string[] {
  const seen: AnyRecord = {};
  const out: string[] = [];
  for (const r of rows) {
    for (const tok of r.split(/\s+/)) {
      if (!tok || tok === ".") continue;
      if (!seen[tok]) {
        seen[tok] = true;
        out.push(tok);
      }
    }
  }
  return out;
}

// ── presence layer (SDK v2 Phase 5, §3.5) ────────────────────────────────────
// Which agent-driven commands surface the ambient activity chip, and the verb
// phrase each shows (unless the frame carried a verbatim `narrate`).
const PRESENCE_CMDS: Record<string, boolean> = {
  panel: true,
  render: true,
  patch: true,
  state: true,
  theme: true,
  layout: true,
  notify: true,
  highlight: true,
  figure: true,
};

/** Trim an `@region:`/`@panel:`/`@` target or title down to a short label. */
function presenceLabel(s: string): string {
  let t = String(s || "").trim();
  if (t.indexOf("@region:") === 0) t = t.slice(8);
  else if (t.indexOf("@panel:") === 0) t = t.slice(7);
  else if (t.charAt(0) === "@") t = t.slice(1);
  if (t.length > 48) t = t.slice(0, 47) + "…";
  return t;
}

/** The chip text for an agent-driven command (before any `narrate` override).
 *  `who` is the actor label — "AI" for the main agent, or a worker profile name
 *  when the frame carried an `agent` (§3.8). */
function presenceTextFor(cmd: UiCommand, who: string): string {
  const p = (who || "AI") + " · ";
  const c = cmd.cmd;
  if (c === "panel") return p + "updating panel " + presenceLabel(cmd.title || cmd.id || "");
  if (c === "render") return p + "updating " + presenceLabel(cmd.target || "view");
  if (c === "patch") return p + "updating the view";
  if (c === "state") return p + "updating data";
  if (c === "theme") return p + "restyling";
  if (c === "layout") return p + "rearranging the layout";
  if (c === "notify") return p + presenceLabel(cmd.message || "notice");
  if (c === "highlight") return p + "highlighting " + presenceLabel(cmd.target || "");
  if (c === "figure") return p + "rendering a figure";
  return p + "working";
}

/** `String(err)` capped at 500 chars, for a `ui_error` frame (never throws). */
function errText(e: unknown): string {
  let s = "error";
  try {
    s = String(e);
  } catch {
    s = "error";
  }
  return s.slice(0, 500);
}

export class UiRuntime {
  private client: BioRouterClient;
  /** The agent's shared state bag (mirrors the state doc when it is an object;
   *  `{}` when the doc is an array/primitive). Kept for back-compat with apps
   *  that read `ui.state` directly. */
  state: Record<string, unknown> = {};
  /** The full shared state document (any JSON value) and its server version. */
  private doc: unknown = {};
  private version = 0;
  private bindings: BindEntry[] = [];
  private scanned = false;
  private stateSubs: StateSub[] = [];
  private stateListeners: StateListener[] = [];
  private commandListeners: CommandListener[] = [];
  private docks: Map<string, HTMLElement> = new Map();
  private panels: Map<string, HTMLElement> = new Map();
  private toastHost: HTMLElement | null = null;
  private modalHost: HTMLElement | null = null;
  private openAsk: string | null = null;
  // ── SDK v2: flat id→instance registry + author component registry ──
  private instances: Map<string, InstanceEntry> = new Map();
  private iidSeq = 0;
  private componentReg: ComponentRegistry = new ComponentRegistry();
  // Whether the `node_selected` auto-signal document listener is installed.
  private netSignalWired = false;
  // ── presence layer (§3.5): the ambient agent-activity chip ──
  private presenceEl: HTMLElement | null = null;
  private presenceTimer: ReturnType<typeof setTimeout> | null = null;
  private presenceMsg: string | null = null;
  // ── ui_suggest (§3.5): one live suggestion row + its Escape handler ──
  private suggestRow: HTMLElement | null = null;
  private suggestHostEl: HTMLElement | null = null;
  private suggestKey: KeyHandler | null = null;

  constructor(client: BioRouterClient) {
    this.client = client;

    // Seed the shared document from the app's DECLARED initial state, before any
    // socket exists. Bindings therefore paint their real values on first paint
    // rather than blank-until-the-first-agent-turn — which is what drove authors
    // to keep a private local `state` object, the very thing that then diverged
    // from the doc the agent reads. The server's snapshot (durable state from a
    // prior session) overwrites this on connect; `version` stays 0 so it loses
    // cleanly to anything authoritative.
    const initial = client.config.stateInitial;
    if (initial !== undefined && initial !== null) {
      this.doc = deepClone(initial);
      this.state = asRecord(this.doc);
    }
  }

  /** Subscribe to state changes the agent makes (`ui_state`). */
  onState(fn: StateListener): this {
    this.stateListeners.push(fn);
    return this;
  }

  /** Observe every UI command (for logging, or to override handling). */
  onCommand(fn: CommandListener): this {
    this.commandListeners.push(fn);
    return this;
  }

  /** The author-declared regions the agent may render into. */
  regions(): string[] {
    const out: string[] = [];
    const nodes = document.querySelectorAll("[data-br-region]");
    for (let i = 0; i < nodes.length; i++) {
      const name = (nodes[i] as HTMLElement).dataset.brRegion;
      if (name) out.push(name);
    }
    return out;
  }

  /**
   * Tell the backend what this page offers, so `ui_describe` returns real
   * targets instead of the agent guessing selectors.
   */
  reportSurface(): void {
    const ids: string[] = [];
    const withId = document.querySelectorAll("[id]");
    for (let i = 0; i < withId.length && ids.length < 200; i++) {
      const id = withId[i].id;
      // Skip our own scaffolding (including the injected theme <style>) — the
      // agent addresses those via @panel:/@region:, not by id.
      if (!id || id.indexOf("br-") === 0 || id === "biorouter-theme") continue;
      ids.push(id);
    }
    this.client.sendRaw({
      type: "ui_surface",
      surface: {
        title: document.title,
        regions: this.regions(),
        ids,
        hasChat: !!document.querySelector("[data-br-chat]"),
        panels: Array.from(this.panels.keys()),
      },
    });
    // Index the author's static bindings now that the DOM (and the connection)
    // is ready, and paint them with the current (possibly empty) doc.
    this.scanBindings();
    this.evalAllBindings();
    this.wireNetworkSelectSignal();
  }

  /** Install (once) a document-level listener that turns a `network` instance's
   *  selection change into a `node_selected` signal (gated on the ready surface
   *  declaring it — see `autoEmitNodeSelected`). */
  private wireNetworkSelectSignal(): void {
    if (this.netSignalWired) return;
    this.netSignalWired = true;
    try {
      document.addEventListener("br-network-select", (e) => {
        const ev = e as NetSelectEvent;
        const nodeId = ev.detail ? ev.detail.id : null;
        const tgt = ev.target as ElementLike;
        const holder = tgt && tgt.closest ? tgt.closest("[data-br-iid]") : null;
        const iid = holder ? holder.getAttribute("data-br-iid") : null;
        this.client.autoEmitNodeSelected(nodeId, iid);
      });
    } catch {
      /* no document (non-browser host) — nothing to wire */
    }
  }

  // ── presence layer (§3.5) ──────────────────────────────────────────────────

  /** Flash the ambient agent-activity chip for an applied ui frame. `narrate`
   *  (verbatim) wins; otherwise a per-command phrase. Nothing shows for a frame
   *  outside the presence set that carries no narration. */
  private notePresence(cmd: UiCommand): void {
    // Attribute a worker profile's frame to that profile ("<profile> · …")
    // rather than the generic "AI · …" (§3.8).
    const who = typeof cmd.agent === "string" && cmd.agent ? cmd.agent : "AI";
    if (typeof cmd.narrate === "string" && cmd.narrate) {
      this.presence(who === "AI" ? cmd.narrate : who + " · " + cmd.narrate);
    } else if (PRESENCE_CMDS[cmd.cmd]) {
      this.presence(presenceTextFor(cmd, who));
    }
  }

  /** Show the ambient agent-activity chip with `msg`; it fades ~2.5 s after the
   *  last update. Exposed as `br.ui.presence(msg)` for authors, and called by the
   *  runtime for every agent-driven frame. The chip never intercepts clicks or
   *  steals focus (CSS `pointer-events: none`, not focusable). */
  presence(msg: string): void {
    const text = msg == null ? "" : String(msg);
    if (!text) return;
    const chip = this.presenceChip();
    if (!chip) return;
    chip.textContent = text;
    chip.classList.add("br-presence--on");
    this.presenceMsg = text;
    if (this.presenceTimer != null) {
      try {
        clearTimeout(this.presenceTimer);
      } catch {
        /* ignore */
      }
    }
    try {
      this.presenceTimer = setTimeout(() => this.fadePresence(), 2500);
    } catch {
      /* no timers (non-browser host) */
    }
  }

  /** The current chip text, or `null` when hidden. A test/inspection hook. */
  presenceText(): string | null {
    return this.presenceMsg;
  }

  private presenceChip(): HTMLElement | null {
    if (this.presenceEl && this.presenceEl.isConnected) return this.presenceEl;
    if (!document.body) return null;
    const el = document.createElement("div");
    el.className = "br-presence";
    el.setAttribute("data-br-presence", "1");
    el.setAttribute("aria-hidden", "true");
    document.body.appendChild(el);
    this.presenceEl = el;
    return el;
  }

  private fadePresence(): void {
    this.presenceMsg = null;
    this.presenceTimer = null;
    if (this.presenceEl) this.presenceEl.classList.remove("br-presence--on");
  }

  // ── ui_suggest (§3.5): non-blocking suggestion chips ───────────────────────

  /** Render up to five dismissible suggestion chips. Clicking a chip sends its
   *  `prompt` via `br.prompt` (or, when it has none, hands `onCommand` a synthetic
   *  `suggest` command carrying the label). Escape or the × dismisses them all. */
  private applySuggest(cmd: UiCommand): void {
    const raw = Array.isArray(cmd.chips) ? cmd.chips : [];
    const chips = raw.slice(0, 5);
    // Only one suggestion row at a time; a new one supersedes the old.
    this.dismissSuggest();
    if (!chips.length) return;
    const host = (cmd.target ? this.resolveTarget(cmd.target) : null) || this.suggestHost();
    if (!host) return;

    const row = document.createElement("div");
    row.className = "br-suggest";
    row.setAttribute("data-br-suggest", "1");
    // Styled inline (theme.css carries only the presence chip): a pill bar of
    // themed buttons that re-enables pointer events inside the fixed host.
    row.style.cssText =
      "display:inline-flex;flex-wrap:wrap;align-items:center;gap:8px;padding:8px 10px;pointer-events:auto;" +
      "background:var(--br-surface);border:1px solid var(--br-border);border-radius:999px;box-shadow:var(--br-shadow-pop);max-width:100%;";
    for (const chip of chips) {
      const label = chip && chip.label != null ? String(chip.label) : "";
      const prompt = chip && typeof chip.prompt === "string" ? chip.prompt : "";
      const b = document.createElement("button");
      b.className = "br-btn br-btn--secondary br-suggest__chip";
      b.type = "button";
      b.textContent = label;
      b.addEventListener("click", () => {
        this.dismissSuggest();
        if (prompt) {
          // Fire-and-forget: the streamed answer arrives via the usual events.
          this.client.prompt(prompt).catch(() => undefined);
        } else {
          this.fireSuggestCommand(label);
        }
      });
      row.appendChild(b);
    }
    const x = document.createElement("button");
    x.className = "br-btn br-btn--ghost br-suggest__x";
    x.type = "button";
    x.setAttribute("aria-label", "Dismiss suggestions");
    x.textContent = "×";
    x.addEventListener("click", () => this.dismissSuggest());
    row.appendChild(x);
    host.appendChild(row);
    this.suggestRow = row;

    const onKey: KeyHandler = (e) => {
      if (e && e.key === "Escape") this.dismissSuggest();
    };
    this.suggestKey = onKey;
    try {
      document.addEventListener("keydown", onKey);
    } catch {
      /* no document */
    }
  }

  /** Hand author `onCommand` listeners a synthetic `suggest` command for a chip
   *  that had no prompt of its own (author-owned handling). */
  private fireSuggestCommand(label: string): void {
    const synthetic: UiCommand = { cmd: "suggest", label: label };
    for (const fn of this.commandListeners) {
      try {
        fn(synthetic);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  }

  private dismissSuggest(): void {
    if (this.suggestRow) {
      if (this.suggestRow.parentElement) this.suggestRow.remove();
      this.suggestRow = null;
    }
    if (this.suggestKey) {
      try {
        document.removeEventListener("keydown", this.suggestKey);
      } catch {
        /* ignore */
      }
      this.suggestKey = null;
    }
  }

  private suggestHost(): HTMLElement | null {
    if (this.suggestHostEl && this.suggestHostEl.isConnected) return this.suggestHostEl;
    if (!document.body) return null;
    const el = document.createElement("div");
    el.className = "br-suggest-host";
    // Fixed, bottom-centered, click-through except over the chip row itself.
    el.style.cssText =
      "position:fixed;left:16px;right:16px;bottom:16px;z-index:58;display:flex;justify-content:center;pointer-events:none;";
    document.body.appendChild(el);
    this.suggestHostEl = el;
    return el;
  }

  /** Apply one agent command. Unknown commands are ignored, not fatal. */
  apply(cmd: UiCommand): void {
    for (const fn of this.commandListeners) {
      try {
        fn(cmd);
      } catch {
        /* listener errors are non-fatal */
      }
    }
    // Presence: an agent-driven frame flashes the ambient activity chip. A
    // verbatim `narrate` always wins; otherwise a per-command phrase. User
    // actions never reach here (this only runs for server-pushed `ui` frames).
    this.notePresence(cmd);
    try {
      switch (cmd.cmd) {
        case "panel":
          this.applyPanel(cmd);
          break;
        case "render":
          this.applyRender(cmd);
          break;
        case "patch":
          this.applyPatchOps(cmd);
          break;
        case "highlight":
          this.applyHighlight(cmd);
          break;
        case "theme":
          this.applyTheme(cmd);
          break;
        case "layout":
          this.applyLayout(cmd);
          break;
        case "notify":
          this.applyNotify(cmd);
          break;
        case "state":
          this.applyState(cmd);
          break;
        case "suggest":
          this.applySuggest(cmd);
          break;
        case "ask":
          this.applyAsk(cmd);
          break;
        case "ask_close":
          this.closeAsk(cmd.requestId || "", false);
          break;
        default:
          break;
      }
    } catch (e) {
      // A malformed command must never take the app down.
      this.applyNotify({
        cmd: "notify",
        message: "The agent sent a UI update this app could not apply.",
        level: "warn",
      });
    }
  }

  // ── targets ──────────────────────────────────────────────────────────────

  /**
   * Resolve a tool's `target` string. `@region:x` / `@panel:x` / `@chat` /
   * `@main` are aliases; anything else is a CSS selector.
   */
  resolveTarget(target: string): HTMLElement | null {
    const t = (target || "").trim();
    if (t.indexOf("@region:") === 0) {
      const name = t.slice("@region:".length);
      return document.querySelector<HTMLElement>(
        `[data-br-region="${cssEscape(name)}"]`
      );
    }
    if (t.indexOf("@panel:") === 0) {
      const panel = this.panels.get(t.slice("@panel:".length));
      return panel ? panel.querySelector<HTMLElement>(".br-panel__body") : null;
    }
    if (t === "@chat") return document.querySelector<HTMLElement>("[data-br-chat]");
    if (t === "@main") return this.mainHost();
    return document.querySelector<HTMLElement>(t);
  }

  private mainHost(): HTMLElement {
    return (
      document.querySelector<HTMLElement>("[data-br-main]") ||
      document.querySelector<HTMLElement>("main") ||
      document.querySelector<HTMLElement>(".br-container") ||
      document.body
    );
  }

  private dock(place: string): HTMLElement {
    const cls = DOCK_PLACES[place] || DOCK_PLACES.dock;
    let el = this.docks.get(cls);
    if (el) return el;
    el = document.createElement("aside");
    el.className = "br-dock " + cls;
    el.setAttribute("data-br-dock", place);
    document.body.appendChild(el);
    this.docks.set(cls, el);
    document.body.classList.add("br-has-" + cls.replace("br-dock--", "dock-"));
    return el;
  }

  private syncDockVisibility(): void {
    this.docks.forEach((el, cls) => {
      const bodyCls = "br-has-" + cls.replace("br-dock--", "dock-");
      document.body.classList.toggle(bodyCls, el.childElementCount > 0);
    });
  }

  // ── commands ─────────────────────────────────────────────────────────────

  private applyPanel(cmd: UiCommand): void {
    const id = cmd.id || "";
    if (!id) return;
    if (cmd.remove) {
      const existing = this.panels.get(id);
      if (existing) {
        this.disposeInstancesIn(existing);
        existing.remove();
      }
      this.panels.delete(id);
      this.syncDockVisibility();
      this.refreshBindings();
      return;
    }

    const place = cmd.place || "dock";
    const panel = document.createElement("section");
    panel.className = "br-panel";
    panel.setAttribute("data-br-panel", id);

    if (cmd.title) {
      const head = document.createElement("header");
      head.className = "br-panel__head";
      const h = document.createElement("h3");
      h.className = "br-panel__title";
      h.textContent = cmd.title;
      head.appendChild(h);
      if (cmd.collapsible !== false) {
        const toggle = document.createElement("button");
        toggle.className = "br-panel__toggle";
        toggle.type = "button";
        toggle.setAttribute("aria-label", "Collapse panel");
        toggle.textContent = "–";
        toggle.addEventListener("click", () => {
          const collapsed = panel.classList.toggle("br-panel--collapsed");
          toggle.textContent = collapsed ? "+" : "–";
        });
        head.appendChild(toggle);
      }
      panel.appendChild(head);
    }

    const body = document.createElement("div");
    body.className = "br-panel__body";
    this.renderInto(body, cmd.body || [], id);
    panel.appendChild(body);

    // Where does it mount? A dock slot, "modal"/"main", or — the common case —
    // a TARGET (`@region:x`, `@panel:x`, a selector) so a titled dashboard card
    // lands inside the author's own region. A same-id panel is replaced in place
    // so a refreshed dashboard doesn't jump around.
    const DOCK_SLOTS = ["dock", "left", "right", "bottom"];
    const isTarget = place.indexOf("@") === 0 || DOCK_SLOTS.indexOf(place) < 0 && place !== "modal" && place !== "main";
    const prev = this.panels.get(id);
    if (prev && prev.parentElement) {
      this.disposeInstancesIn(prev);
      prev.parentElement.replaceChild(panel, prev);
    } else if (isTarget) {
      const host = this.resolveTarget(place);
      if (host) {
        host.appendChild(panel);
      } else {
        // Named a region/selector that isn't there — say so, don't vanish.
        this.applyNotify({
          cmd: "notify",
          message: `The agent tried to mount panel "${id}" into "${place}", which this app does not have.`,
          level: "warn",
        });
        this.dock("dock").appendChild(panel);
      }
    } else if (place === "modal") {
      this.modal().appendChild(panel);
    } else if (place === "main") {
      this.mainHost().appendChild(panel);
    } else {
      this.dock(place).appendChild(panel);
    }
    this.panels.set(id, panel);
    this.syncDockVisibility();
    // The panel body may carry author bindings — index and paint them.
    this.refreshBindings();
  }

  private applyRender(cmd: UiCommand): void {
    const host = this.resolveTarget(cmd.target || "");
    if (!host) {
      // Say so out loud: a silently-dropped render looks like a broken agent.
      this.applyNotify({
        cmd: "notify",
        message: `The agent tried to render into "${cmd.target}", which this app does not have.`,
        level: "warn",
      });
      return;
    }
    const widgetId = cmd.target || "render";
    const body = cmd.body || [];
    if (cmd.mode === "append") {
      this.renderInto(host, body, widgetId, true);
    } else if (this.hasKeyedChildren(host)) {
      // Re-render over a target that already holds instances → morph per-id
      // instead of wiping (preserves focus/scroll/canvas/network positions).
      this.morphInto(host, body, widgetId);
    } else {
      this.renderInto(host, body, widgetId, false);
    }
    // Rendered markup may contain author bindings — re-index and paint.
    this.refreshBindings();
  }

  private hasKeyedChildren(host: HTMLElement): boolean {
    for (let i = 0; i < host.children.length; i++) {
      if (host.children[i].getAttribute("data-br-iid")) return true;
    }
    return false;
  }

  /** A `WidgetContext` bound to `widgetId`, including the component bridge. */
  private makeCtx(widgetId: string): WidgetContext {
    const rt = this;
    return {
      fields: new Map(),
      onAction: (action, payload) => rt.client.sendRaw({ type: "widget_action", widgetId: widgetId, action: action, payload: payload }),
      mountComponent: (name, props, cid) => rt.mountComponent(name, props, cid || widgetId),
    };
  }

  /** The stable id for a node: its own `id`, else a generated one (stamped back
   *  onto the node so nested lookups agree within this render). */
  private nodeId(node: WidgetNode): string {
    const raw = (node as NodeWithId).id;
    if (typeof raw === "string" && raw) return raw;
    this.iidSeq++;
    const gen = "auto-" + this.iidSeq;
    (node as NodeWithId).id = gen;
    return gen;
  }

  /** Render a widget node, catching a throwing renderer: a neutral placeholder
   *  goes in its place and one (rate-limited) `ui_error` is posted, so a bad
   *  node never breaks the render or the socket. Component-mount errors report
   *  themselves inside `mountComponent`, so this stays a single report per node. */
  private renderNode(node: WidgetNode, ctx: WidgetContext): HTMLElement {
    try {
      return renderWidget(node, ctx);
    } catch (e) {
      const kind = String((node as UnknownWidget).t || "widget");
      this.client.reportUiError("widget:" + kind, errText(e), (node as NodeWithId).id);
      return unknownWidgetEl(kind);
    }
  }

  /** Render one node standalone (own ctx), tag it, but do NOT register it. */
  private buildInstanceEl(node: WidgetNode, id: string): HTMLElement {
    const el = this.renderNode(node, this.makeCtx(id));
    el.setAttribute("data-br-iid", id);
    return el;
  }

  /** Render widget nodes into `host`, wiring buttons back into the agent loop
   *  and registering each top-level node as an addressable instance. A shared
   *  ctx keeps a form's fields collectable across sibling nodes. */
  private renderInto(host: HTMLElement, nodes: WidgetNode[], widgetId: string, append?: boolean): void {
    if (!append) {
      this.disposeInstancesIn(host);
      host.innerHTML = "";
    }
    const ctx = this.makeCtx(widgetId);
    for (const node of nodes) {
      const el = this.renderNode(node, ctx);
      const id = this.nodeId(node);
      el.setAttribute("data-br-iid", id);
      this.instances.set(id, { node: node, el: el });
      host.appendChild(el);
    }
  }

  /** Keyed reconciliation: match new body ids against existing instances under
   *  `host`; update matched in place, append new, remove unmatched, reorder. */
  private morphInto(host: HTMLElement, nodes: WidgetNode[], widgetId: string): void {
    const existing: AnyRecord = {};
    for (let i = 0; i < host.children.length; i++) {
      const iid = host.children[i].getAttribute("data-br-iid");
      if (iid) existing[iid] = true;
    }
    const ctx = this.makeCtx(widgetId);
    const seen: AnyRecord = {};
    const ordered: HTMLElement[] = [];
    for (const node of nodes) {
      const id = this.nodeId(node);
      seen[id] = true;
      if (existing[id] && this.instances.has(id)) {
        this.morphInstance(id, node);
      } else {
        const el = this.renderNode(node, ctx);
        el.setAttribute("data-br-iid", id);
        this.instances.set(id, { node: node, el: el });
        host.appendChild(el);
      }
      const entry = this.instances.get(id);
      if (entry) ordered.push(entry.el);
    }
    for (let i = host.children.length - 1; i >= 0; i--) {
      const ch = host.children[i] as HTMLElement;
      const iid = ch.getAttribute("data-br-iid");
      if (iid && !seen[iid]) {
        this.disposeInstance(iid);
        ch.remove();
      }
    }
    for (const el of ordered) host.appendChild(el);
  }

  /** Update a single instance in place. Component nodes update without swapping
   *  their container; everything else re-renders the subtree and restores
   *  focus / scroll / network positions. */
  private morphInstance(id: string, newNode: WidgetNode): void {
    const entry = this.instances.get(id);
    if (!entry) return;
    const oldNode = entry.node;
    const oldEl = entry.el;
    if ((newNode as UnknownWidget).t === "component") {
      this.updateComponent(id, oldNode, newNode, oldEl);
      entry.node = newNode;
      return;
    }
    const parent = oldEl.parentElement;
    const focus = captureFocus(oldEl);
    const scroll = captureScroll(oldEl);
    const newEl = this.buildInstanceEl(newNode, id);
    adoptNetwork(oldEl, newEl);
    if (parent) parent.replaceChild(newEl, oldEl);
    this.instances.set(id, { node: newNode, el: newEl });
    restoreScroll(newEl, scroll);
    restoreFocus(newEl, focus);
  }

  // ── ui_patch ops (add / replace / set_props / remove) ──────────────────────

  private applyPatchOps(cmd: UiCommand): void {
    const ops = Array.isArray(cmd.ops) ? cmd.ops : [];
    for (const raw of ops) {
      if (!raw || typeof raw !== "object") continue;
      this.applyOneOp(raw as PatchOp);
    }
    this.refreshBindings();
  }

  private applyOneOp(op: PatchOp): void {
    const kind = String(op.op || "");
    if (kind === "add") this.opAdd(op);
    else if (kind === "replace") this.opReplace(op);
    else if (kind === "set_props") this.opSetProps(op);
    else if (kind === "remove") this.opRemove(op);
  }

  private resolvePatchHost(op: PatchOp): HTMLElement | null {
    if (op.parent) {
      const p = this.instances.get(op.parent);
      if (p) return p.el;
    }
    if (op.target) return this.resolveTarget(op.target);
    // default target: the main results region, else the app's main host.
    return this.resolveTarget("@region:results") || this.mainHost();
  }

  private opAdd(op: PatchOp): void {
    const node = op.node;
    if (!node) return;
    const id = typeof op.id === "string" && op.id ? op.id : this.nodeId(node);
    (node as NodeWithId).id = id;
    const host = this.resolvePatchHost(op);
    if (!host) return;
    const el = this.buildInstanceEl(node, id);
    this.instances.set(id, { node: node, el: el });
    const index = typeof op.index === "number" ? op.index : -1;
    const ref = index >= 0 && index < host.children.length ? host.children[index] : null;
    if (ref) host.insertBefore(el, ref);
    else host.appendChild(el);
  }

  private opReplace(op: PatchOp): void {
    const id = op.id || "";
    if (!id || !op.node || !this.instances.has(id)) return;
    (op.node as NodeWithId).id = id;
    this.morphInstance(id, op.node);
  }

  private opSetProps(op: PatchOp): void {
    const id = op.id || "";
    const entry = this.instances.get(id);
    if (!entry) return;
    const props = op.props && typeof op.props === "object" ? op.props : {};
    const node = entry.node as UnknownWidget;
    // Log append fast-path: preserves scroll, no full re-render.
    if (node.t === "log" && Array.isArray(props.append)) {
      appendLogLines(entry.el, entry.node as LogNode, props.append);
      if (typeof props.max === "number") (entry.node as LogNode).max = props.max;
      return;
    }
    // Components: merge into the component's own props (not the node header).
    if (node.t === "component") {
      const cn = entry.node as ComponentNode;
      const cur = cn.props && typeof cn.props === "object" ? cn.props : {};
      const mergedProps = Object.assign({}, cur, props);
      const mergedNode = Object.assign({}, entry.node);
      (mergedNode as ComponentNode).props = mergedProps;
      this.morphInstance(id, mergedNode as WidgetNode);
      return;
    }
    const merged = shallowMergeNode(entry.node, props);
    this.morphInstance(id, merged);
  }

  private opRemove(op: PatchOp): void {
    const id = op.id || "";
    const entry = this.instances.get(id);
    if (!entry) return;
    const net = readNet(entry.el);
    if (net) {
      try {
        net.destroy();
      } catch {
        /* ignore */
      }
    }
    if (entry.el.parentElement) entry.el.remove();
    this.instances.delete(id);
  }

  private disposeInstance(id: string): void {
    const entry = this.instances.get(id);
    if (entry) {
      const net = readNet(entry.el);
      if (net) {
        try {
          net.destroy();
        } catch {
          /* ignore */
        }
      }
    }
    this.instances.delete(id);
  }

  /** Dispose every instance whose element lives inside `host` (before a wipe). */
  private disposeInstancesIn(host: HTMLElement): void {
    const found = host.querySelectorAll("[data-br-iid]");
    for (let i = 0; i < found.length; i++) {
      const iid = found[i].getAttribute("data-br-iid");
      if (iid) this.disposeInstance(iid);
    }
  }

  // ── author component registry (`br.components`) ────────────────────────────

  registerComponent(name: string, def: ComponentDef): void {
    this.componentReg.register(name, def);
  }

  private componentCtx(id: string): ComponentContext {
    const client = this.client;
    return {
      id: id,
      state: client.state,
      run: (t, tgt, o) => client.run(t, tgt, o),
    };
  }

  /** Mount a registered component into a fresh container; `null` when the name
   *  is unregistered (renderWidget then shows the neutral placeholder). */
  mountComponent(name: string, props: unknown, id: string): HTMLElement | null {
    const def = this.componentReg.get(name);
    if (!def) return null;
    const el = wEl("div", "br-component");
    el.setAttribute("data-br-component", String(name));
    try {
      def.mount(el, props, this.componentCtx(id));
    } catch (e) {
      // A throwing mount degrades to the neutral placeholder + one ui_error
      // (returning null makes renderWidget render the placeholder in place).
      this.client.reportUiError("component:" + name, errText(e), id);
      return null;
    }
    return el;
  }

  private updateComponent(id: string, oldNode: WidgetNode, newNode: WidgetNode, el: HTMLElement): void {
    const nn = newNode as ComponentNode;
    const on = oldNode as ComponentNode;
    const name = String(nn.name || "");
    const def = this.componentReg.get(name);
    const sameName = on && on.t === "component" && String(on.name || "") === name;
    if (def && typeof def.update === "function" && sameName) {
      try {
        def.update(el, nn.props, on.props);
      } catch (e) {
        this.client.reportUiError("component:" + name, errText(e), id);
      }
    } else {
      el.innerHTML = "";
      if (def) {
        try {
          def.mount(el, nn.props, this.componentCtx(id));
        } catch (e) {
          this.client.reportUiError("component:" + name, errText(e), id);
        }
      } else {
        el.appendChild(unknownWidgetEl("component:" + name));
      }
    }
  }

  /** The network controller for a rendered `network` instance (for programmatic
   *  select / positions). Returns `null` when the id is not a network. */
  network(id: string): NetworkController | null {
    const entry = this.instances.get(id);
    return entry ? readNet(entry.el) : null;
  }

  /** Programmatically select a node in a network instance. */
  selectNetworkNode(id: string, nodeId: string | null): void {
    const net = this.network(id);
    if (net) net.select(nodeId);
  }

  /** Register any addressable containers rendered outside the instance path
   *  (```chart / ```graph fences carry a `data-br-iid`) so `ui_patch replace`
   *  can target them. Kind is inferred from the container class. */
  private indexDomInstances(): void {
    const found = document.querySelectorAll("[data-br-iid]");
    for (let i = 0; i < found.length; i++) {
      const el = found[i] as HTMLElement;
      const iid = el.getAttribute("data-br-iid") || "";
      if (!iid || this.instances.has(iid)) continue;
      const cls = typeof el.className === "string" ? el.className : "";
      let kind = "widget";
      if (cls.indexOf("br-chart") >= 0 || cls.indexOf("br-plot") >= 0) kind = "plot";
      else if (cls.indexOf("br-visual") >= 0 || cls.indexOf("br-graph") >= 0) kind = "network-lite";
      this.instances.set(iid, { node: { t: kind } as WidgetNode, el: el });
    }
  }

  private applyHighlight(cmd: UiCommand): void {
    if (cmd.mode === "clear") {
      this.clearHighlights();
      return;
    }
    this.clearHighlights();
    const el = this.resolveTarget(cmd.target || "");
    if (!el) {
      // The tool reported success, so tell the user something happened rather
      // than leaving them staring at an unchanged page.
      this.applyNotify({
        cmd: "notify",
        message: `The agent tried to highlight "${cmd.target}", which this app does not have.`,
        level: "warn",
      });
      return;
    }
    el.classList.add("br-highlight", "br-highlight--" + (cmd.mode || "outline"));
    // Focus mode dims via a fixed backdrop the target is raised above — NOT via
    // ancestor opacity, which would drag the nested target down with it.
    if (cmd.mode === "focus") this.focusBackdrop();
    if (cmd.note) {
      const note = document.createElement("div");
      note.className = "br-callout";
      note.setAttribute("data-br-callout", "1");
      note.textContent = cmd.note;
      el.insertAdjacentElement("afterend", note);
    }
    if (cmd.scroll !== false && typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }

  private focusBackdrop(): HTMLElement {
    let bd = document.querySelector<HTMLElement>(".br-focus-backdrop");
    if (!bd) {
      bd = document.createElement("div");
      bd.className = "br-focus-backdrop";
      document.body.appendChild(bd);
    }
    return bd;
  }

  private clearHighlights(): void {
    const marked = document.querySelectorAll(".br-highlight");
    for (let i = 0; i < marked.length; i++) {
      marked[i].classList.remove(
        "br-highlight",
        "br-highlight--outline",
        "br-highlight--pulse",
        "br-highlight--focus"
      );
    }
    const notes = document.querySelectorAll("[data-br-callout]");
    for (let i = 0; i < notes.length; i++) notes[i].remove();
    const bd = document.querySelector(".br-focus-backdrop");
    if (bd) bd.remove();
  }

  private applyTheme(cmd: UiCommand): void {
    const root = document.documentElement;

    // Snapshot, so an illegible restyle can be UNDONE. `ui_theme` was
    // fire-and-forget: it emitted the frame and reported success no matter what it
    // did to the page. The agent could black out the app's own scientific regions
    // and be told it worked — and could not have known otherwise, because the
    // failure depends on CSS the agent wrote in an earlier turn.
    const before = {
      pack: root.getAttribute("data-br-pack"),
      mode: root.getAttribute("data-br-theme"),
      density: root.getAttribute("data-br-density"),
      accent: root.style.getPropertyValue("--br-accent"),
    };

    if (typeof cmd.pack === "string") this.applyPack(cmd.pack, root);
    if (cmd.accent) root.style.setProperty("--br-accent", cmd.accent);
    if (cmd.mode) {
      if (cmd.mode === "auto") root.removeAttribute("data-br-theme");
      else root.setAttribute("data-br-theme", cmd.mode);
    }
    if (cmd.density) root.setAttribute("data-br-density", cmd.density);

    // Audit after the browser has recomputed styles.
    requestAnimationFrame(() => {
      const offenders = auditContrast();
      if (!offenders.length) return;

      // Revert exactly what we changed.
      if (before.pack) root.setAttribute("data-br-pack", before.pack);
      else root.removeAttribute("data-br-pack");
      if (before.mode) root.setAttribute("data-br-theme", before.mode);
      else root.removeAttribute("data-br-theme");
      if (before.density) root.setAttribute("data-br-density", before.density);
      else root.removeAttribute("data-br-density");
      if (before.accent) root.style.setProperty("--br-accent", before.accent);
      else root.style.removeProperty("--br-accent");

      const detail = offenders.slice(0, 4).join("; ");
      this.reportUiError(
        "theme",
        `theme reverted: it made ${offenders.length} element(s) illegible (${detail}). ` +
          `The app's own CSS hardcodes colours that do not follow the theme tokens.`
      );
    });
  }

  /** Switch the active theme pack. Validated against the curated set — an unknown
   *  pack is ignored with a warning. A pack owns its complete palette, so clear a
   *  stale generic mode; an explicit mode in the same command is applied next. */
  private applyPack(pack: string, root: HTMLElement): void {
    if (!KNOWN_PACKS[pack]) {
      try {
        console.warn("[Biorouter] ignoring unknown theme pack: " + pack);
      } catch {
        /* console may be unavailable */
      }
      return;
    }
    root.setAttribute("data-br-pack", pack);
    root.removeAttribute("data-br-theme");
  }

  private applyLayout(cmd: UiCommand): void {
    // Layout grammar (SDK v2 §3.6): when `areas` is present, build a CSS grid on
    // the app's main container. The five named presets keep working unchanged.
    if (Array.isArray(cmd.areas) && cmd.areas.length) {
      this.applyGridLayout(cmd);
      return;
    }
    const preset = cmd.preset || "single";
    document.body.setAttribute("data-br-layout", preset);

    // Dock width is a single custom property so an explicit `sidebarWidth`
    // always wins over the preset's default (a CSS rule per preset would beat
    // the inline var and silently ignore the caller).
    const root = document.documentElement;
    const presetWidth: Record<string, string> = {
      dashboard: "min(62vw, 900px)",
      split: "50vw",
    };
    if (cmd.sidebarWidth) {
      root.style.setProperty(
        "--br-dock-w",
        Math.max(200, Math.min(1200, cmd.sidebarWidth)) + "px"
      );
    } else if (presetWidth[preset]) {
      root.style.setProperty("--br-dock-w", presetWidth[preset]);
    } else {
      root.style.removeProperty("--br-dock-w"); // back to the stylesheet default
    }

    // "sidebar-left" has to actually move the panels, or the preset is a lie.
    const side = preset === "sidebar-left" ? "left" : "right";
    const from = this.docks.get(DOCK_PLACES[side === "left" ? "right" : "left"]);
    if (from && from.childElementCount) {
      const to = this.dock(side);
      while (from.firstElementChild) to.appendChild(from.firstElementChild);
    }

    // The dashboard preset lays the dock out as a grid rather than a column.
    for (const [, el] of this.docks) {
      el.classList.toggle("br-dock--grid", preset === "dashboard");
    }
    this.syncDockVisibility();
  }

  /** Build a CSS grid on the main container from a bounded `areas`/`sizes` grammar.
   *  Rows are strings ("a b") or arrays (["a","b"]); columns default to `1fr`.
   *  Each named area is assigned to the element with `data-br-region="<area>"` or
   *  `id="<area>"`; a missing element is warned (once per name) and skipped. */
  private applyGridLayout(cmd: UiCommand): void {
    const rows: string[] = [];
    for (const r of cmd.areas || []) {
      if (typeof r === "string") {
        const s = r.trim();
        if (s) rows.push(s);
      } else if (Array.isArray(r)) {
        const s = r.map((x) => String(x)).join(" ").trim();
        if (s) rows.push(s);
      }
    }
    if (!rows.length) return;
    const host = this.mainHost();
    host.style.setProperty("display", "grid");
    host.style.setProperty("grid-template-areas", rows.map((r) => '"' + r + '"').join(" "));

    let colCount = 0;
    for (const r of rows) {
      const n = r.split(/\s+/).filter((t) => t).length;
      if (n > colCount) colCount = n;
    }
    const sizes = Array.isArray(cmd.sizes) ? cmd.sizes : [];
    const cols: string[] = [];
    for (let i = 0; i < colCount; i++) cols.push(sizeToken(sizes[i]));
    host.style.setProperty("grid-template-columns", cols.join(" "));

    const warned: AnyRecord = {};
    for (const name of uniqueAreaNames(rows)) {
      const el = this.findAreaElement(name);
      if (el) {
        el.style.setProperty("grid-area", name);
      } else if (!warned[name]) {
        warned[name] = true;
        try {
          console.warn("[Biorouter] layout area has no matching element: " + name);
        } catch {
          /* console may be unavailable */
        }
      }
    }
  }

  /** The element for a named grid area: `data-br-region="<name>"` or `id`. */
  private findAreaElement(name: string): HTMLElement | null {
    const byRegion = document.querySelector<HTMLElement>('[data-br-region="' + cssEscape(name) + '"]');
    if (byRegion) return byRegion;
    return document.getElementById(name);
  }

  private toasts(): HTMLElement {
    if (this.toastHost && this.toastHost.isConnected) return this.toastHost;
    const el = document.createElement("div");
    el.className = "br-toasts";
    document.body.appendChild(el);
    this.toastHost = el;
    return el;
  }

  private applyNotify(cmd: UiCommand): void {
    const t = document.createElement("div");
    t.className = "br-toast br-toast--" + (cmd.level || "info");
    t.setAttribute("role", "status");
    t.textContent = cmd.message || "";
    this.toasts().appendChild(t);
    const ms = cmd.timeoutMs === undefined ? 4000 : cmd.timeoutMs;
    if (ms > 0) window.setTimeout(() => t.remove(), ms);
    else {
      t.classList.add("br-toast--sticky");
      t.addEventListener("click", () => t.remove());
    }
  }

  /**
   * Apply a `state` frame. Three shapes:
   *   - `mode:"snapshot"` → replace the whole doc, adopt its version.
   *   - `mode:"patch"`    → apply RFC-6902 ops, adopt the new version.
   *   - no mode (legacy)  → treat `state` as a snapshot of unknown version.
   * A patch that fails to apply leaves the prior doc untouched.
   */
  private applyState(cmd: UiCommand): void {
    if (!this.scanned) this.scanBindings();
    const mode = cmd.mode;
    if (mode === "snapshot") {
      this.doc = cmd.doc === undefined ? {} : cmd.doc;
      if (typeof cmd.version === "number") this.version = cmd.version;
      this.afterStateChange(null);
    } else if (mode === "patch") {
      const before = this.doc;
      try {
        this.doc = applyPatch(this.doc, cmd.patch);
      } catch {
        this.doc = before;
      }
      if (typeof cmd.version === "number") this.version = cmd.version;
      this.afterStateChange(patchedPaths(cmd.patch));
    } else {
      // Legacy `{cmd:"state", state:{…}}` — a full snapshot, version unknown.
      this.doc = cmd.state || {};
      this.afterStateChange(null);
    }
  }

  /** Re-mirror `state`, re-evaluate bindings (all on snapshot, only affected on
   *  patch), then notify `onState` listeners and `subscribe`rs. */
  private afterStateChange(affected: string[] | null): void {
    this.state = asRecord(this.doc);
    if (affected === null) {
      this.evalAllBindings();
    } else {
      for (const b of this.bindings) {
        if (pointerAffected(b.pointer, affected)) this.applyBinding(b);
      }
    }
    for (const fn of this.stateListeners) {
      try {
        fn(this.state);
      } catch {
        /* listener errors are non-fatal */
      }
    }
    for (const sub of this.stateSubs) {
      const val = pointerGet(this.doc, sub.pointer);
      const enc = stateStringify(val);
      if (enc !== sub.last) {
        sub.last = enc;
        try {
          sub.fn(deepClone(val));
        } catch {
          /* subscriber errors are non-fatal */
        }
      }
    }
  }

  // ── declarative bindings ───────────────────────────────────────────────────

  /** Rebuild the pointer→element binding index from the current DOM. Called at
   *  start and after any render that could add/remove bound nodes. */
  private scanBindings(): void {
    this.scanned = true;
    const list: BindEntry[] = [];
    const texts = document.querySelectorAll("[data-br-bind]");
    for (let i = 0; i < texts.length; i++) {
      const el = texts[i];
      const p = el.getAttribute("data-br-bind") || "";
      if (p) list.push({ el: el, kind: "text", pointer: p, attr: "" });
    }
    const shows = document.querySelectorAll("[data-br-bind-show]");
    for (let i = 0; i < shows.length; i++) {
      const el = shows[i];
      const p = el.getAttribute("data-br-bind-show") || "";
      if (p) list.push({ el: el, kind: "show", pointer: p, attr: "" });
    }
    const attrs = document.querySelectorAll("[data-br-bind-attr]");
    for (let i = 0; i < attrs.length; i++) {
      const el = attrs[i];
      const spec = el.getAttribute("data-br-bind-attr") || "";
      // One or more `attrName:/pointer` pairs, comma-separated.
      for (const one of spec.split(",")) {
        const idx = one.indexOf(":");
        if (idx < 0) continue;
        const attr = one.slice(0, idx).trim();
        const p = one.slice(idx + 1).trim();
        if (attr && p) list.push({ el: el, kind: "attr", pointer: p, attr: attr });
      }
    }

    // `data-br-model="/pointer"` — TWO-WAY binding on a form control.
    //
    // Every other binding is one-way (doc → DOM). There was no write-back path at
    // all, so an author who wanted a slider to update state had to hand-roll a
    // listener — and the generated code routinely got it wrong: it listened for
    // `change` while re-rendering the region from a stale local object, so the
    // control snapped back and arrow-key `input` events never reached the doc
    // (a bound range sat at 0.35 no matter how many times it was pressed).
    //
    // With `data-br-model` the SDK owns the write path, so keyboard, pointer and
    // programmatic changes all converge on `br.state.set` and cannot desync.
    const models = document.querySelectorAll("[data-br-model]");
    for (let i = 0; i < models.length; i++) {
      const el = models[i];
      const p = el.getAttribute("data-br-model") || "";
      if (!p) continue;
      list.push({ el: el, kind: "model", pointer: p, attr: "" });
      this.wireModelListener(el as HTMLElement, p);
    }

    this.bindings = list;
  }

  /**
   * Attach the write-back listeners for one `data-br-model` control. Idempotent:
   * `scanBindings` re-runs after every render, and a control that survives a
   * re-render must not accumulate duplicate listeners.
   */
  private wireModelListener(el: HTMLElement, pointer: string): void {
    if (el.dataset.brModelWired === "1") return;
    el.dataset.brModelWired = "1";

    const write = () => {
      this.stateSet(pointer, readControlValue(el));
    };
    // `input` covers typing, dragging and ARROW KEYS on a range; `change` covers
    // select/checkbox and the commit of a native picker. Listening for only one of
    // them is precisely how the generated slider ended up keyboard-dead.
    el.addEventListener("input", write);
    el.addEventListener("change", write);
  }

  /** Rescan + repaint after the DOM structure changed (a render/panel). */
  private refreshBindings(): void {
    this.scanBindings();
    this.evalAllBindings();
    this.indexDomInstances();
  }

  private evalAllBindings(): void {
    for (const b of this.bindings) this.applyBinding(b);
  }

  /** Push one binding's current pointer value into its element. Non-executing
   *  sinks only: `textContent`, an allowlisted attribute, or the `hidden`
   *  property. Never `innerHTML`. */
  private applyBinding(b: BindEntry): void {
    const value = pointerGet(this.doc, b.pointer);
    if (b.kind === "text") {
      b.el.textContent = bindText(value);
    } else if (b.kind === "show") {
      const he = b.el as HTMLElement;
      he.hidden = !value;
    } else if (b.kind === "attr") {
      this.applyBindAttr(b.el, b.attr, value);
    } else if (b.kind === "model") {
      // Do not clobber the control the user is actively editing.
      if (document.activeElement === b.el) return;
      writeControlValue(b.el as HTMLElement, value);
    }
  }

  private applyBindAttr(el: Element, attr: string, value: unknown): void {
    const name = attr.toLowerCase();
    // Executable sinks are refused outright.
    if (name.indexOf("on") === 0 || name === "style") {
      try {
        console.warn("[Biorouter] refused data-br-bind-attr on forbidden attribute: " + name);
      } catch {
        /* ignore */
      }
      return;
    }
    const allowed =
      BIND_ATTR_ALLOW[name] === true ||
      name.indexOf("aria-") === 0 ||
      name.indexOf("data-") === 0;
    if (!allowed) {
      try {
        console.warn("[Biorouter] refused data-br-bind-attr on unlisted attribute: " + name);
      } catch {
        /* ignore */
      }
      return;
    }
    // `value` on a form control is a property, not an attribute — and we never
    // clobber an element the user is actively editing.
    if (name === "value") {
      const tag = el.tagName.toLowerCase();
      if (tag === "input" || tag === "textarea" || tag === "select") {
        if (document.activeElement === el) return;
        const field = el as HTMLInputElement;
        field.value = value == null ? "" : String(value);
        return;
      }
    }
    if (name === "href" || name === "src") {
      const url = value == null ? "" : String(value);
      if (!isSafeBindUrl(url)) {
        try {
          console.warn("[Biorouter] refused unsafe " + name + " value: " + url);
        } catch {
          /* ignore */
        }
        return;
      }
      el.setAttribute(name, url);
      return;
    }
    // Boolean-ish / plain attributes: absence for falsy, else the string value.
    if (value === false || value == null) {
      el.removeAttribute(name);
      return;
    }
    el.setAttribute(name, String(value));
  }

  // ── public state API (surfaced as `br.state`) ─────────────────────────────

  /** The value at a JSON Pointer (deep-cloned), or the whole doc when omitted. */
  stateGet(path?: string): unknown {
    if (path === undefined || path === null || path === "") return deepClone(this.doc);
    return deepClone(pointerGet(this.doc, path));
  }

  /** Optimistically set the value at `path`, then send a `state_write` carrying
   *  the pre-write `baseVersion`. */
  stateSet(path: string, value: unknown): void {
    const baseVersion = this.version;
    try {
      this.doc = applyPatch(this.doc, [{ op: "add", path: path, value: value }]);
    } catch {
      /* keep the prior doc on failure */
    }
    this.afterStateChange([path]);
    this.client.sendRaw({
      type: "state_write",
      set: { path: path, value: value },
      baseVersion: baseVersion,
    });
  }

  /** Optimistically remove the value at `path`, then send a `state_write`. */
  stateRemove(path: string): void {
    const baseVersion = this.version;
    const op = { op: "remove", path: path };
    try {
      this.doc = applyPatch(this.doc, [op]);
    } catch {
      /* keep the prior doc on failure */
    }
    this.afterStateChange([path]);
    this.client.sendRaw({ type: "state_write", patch: [op], baseVersion: baseVersion });
  }

  /** Replace the whole doc with `fn(clone)`'s return value, then send it. */
  stateUpdate(fn: DocUpdater): void {
    const baseVersion = this.version;
    const draft = deepClone(this.doc);
    let next: unknown;
    try {
      next = fn(draft);
    } catch {
      next = draft;
    }
    if (next === undefined) next = draft;
    this.doc = next;
    this.afterStateChange(null);
    this.client.sendRaw({
      type: "state_write",
      set: { path: "", value: next },
      baseVersion: baseVersion,
    });
  }

  /** Fire `fn(value)` whenever the value at `path` changes (JSON-compared).
   *  Returns an unsubscribe function. */
  stateSubscribe(path: string, fn: ValueSub): Unsub {
    const sub: StateSub = {
      pointer: path || "",
      fn: fn,
      last: stateStringify(pointerGet(this.doc, path || "")),
    };
    this.stateSubs.push(sub);
    return () => {
      const i = this.stateSubs.indexOf(sub);
      if (i >= 0) this.stateSubs.splice(i, 1);
    };
  }

  private modal(): HTMLElement {
    if (this.modalHost && this.modalHost.isConnected) return this.modalHost;
    const el = document.createElement("div");
    el.className = "br-modal-host";
    document.body.appendChild(el);
    this.modalHost = el;
    return el;
  }

  /**
   * Render a blocking question from `ui_ask`. The tool call on the server is
   * parked until we send `ui_reply` — so *every* exit path from this form must
   * send one, including dismissal.
   */
  private applyAsk(cmd: UiCommand): void {
    const requestId = cmd.requestId || "";
    if (!requestId) return;
    // Only one question at a time; a second supersedes (and cancels) the first.
    if (this.openAsk) this.closeAsk(this.openAsk, true);
    this.openAsk = requestId;

    const host = this.modal();
    const backdrop = document.createElement("div");
    backdrop.className = "br-modal";
    backdrop.setAttribute("data-br-ask", requestId);

    const card = document.createElement("div");
    card.className = "br-modal__card br-card";
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-modal", "true");

    if (cmd.title) {
      const h = document.createElement("h3");
      h.className = "br-card__title";
      h.textContent = cmd.title;
      card.appendChild(h);
    }
    if (cmd.prompt) {
      const p = document.createElement("div");
      p.className = "br-text";
      p.innerHTML = renderMarkdown(cmd.prompt);
      card.appendChild(p);
    }

    const getters: FieldGetters = new Map();
    const form = document.createElement("div");
    form.className = "br-form";
    for (const f of cmd.fields || []) {
      form.appendChild(buildAskField(f, getters));
    }
    card.appendChild(form);

    const actions = document.createElement("div");
    actions.className = "br-modal__actions";
    const cancel = document.createElement("button");
    cancel.className = "br-btn br-btn--ghost";
    cancel.type = "button";
    cancel.textContent = "Skip";
    cancel.addEventListener("click", () => this.closeAsk(requestId, true));
    const submit = document.createElement("button");
    submit.className = "br-btn";
    submit.type = "button";
    submit.textContent = cmd.submitLabel || "Submit";
    submit.addEventListener("click", () => {
      const payload: Record<string, string | boolean> = {};
      getters.forEach((get, name) => {
        payload[name] = get();
      });
      this.client.sendRaw({ type: "ui_reply", requestId, payload });
      this.dismissAsk(requestId);
    });
    actions.appendChild(cancel);
    actions.appendChild(submit);
    card.appendChild(actions);

    backdrop.appendChild(card);
    host.appendChild(backdrop);

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") this.closeAsk(requestId, true);
    };
    document.addEventListener("keydown", onKey);
    backdrop.addEventListener("br:teardown", () =>
      document.removeEventListener("keydown", onKey)
    );

    const first = card.querySelector<HTMLElement>("input, select, textarea");
    if (first) first.focus();
  }

  /** Remove the form. When `answer` is true, tell the parked tool it was skipped. */
  private closeAsk(requestId: string, answer: boolean): void {
    if (answer) {
      this.client.sendRaw({
        type: "ui_reply",
        requestId,
        payload: { cancelled: true },
      });
    }
    this.dismissAsk(requestId);
  }

  private dismissAsk(requestId: string): void {
    const el = document.querySelector(`[data-br-ask="${cssEscape(requestId)}"]`);
    if (el) {
      el.dispatchEvent(new CustomEvent("br:teardown"));
      el.remove();
    }
    if (this.openAsk === requestId) this.openAsk = null;
  }
}

/** Minimal `CSS.escape` shim for the attribute selectors we build. */
function cssEscape(value: string): string {
  if (window.CSS && typeof window.CSS.escape === "function") {
    return window.CSS.escape(value);
  }
  return String(value).replace(/["\\\]]/g, "\\$&");
}

function buildAskField(f: AskFieldSpec, getters: FieldGetters): HTMLElement {
  const kind = f.type || "text";
  if (kind === "checkbox") {
    const wrap = wEl("label", "br-check");
    const c = document.createElement("input");
    c.type = "checkbox";
    c.checked = f.value === "true";
    getters.set(f.name, () => c.checked);
    wrap.appendChild(c);
    const l = wEl("span");
    l.textContent = f.label || f.name;
    wrap.appendChild(l);
    return wrap;
  }
  const wrap = wEl("label", "br-field");
  const label = wEl("span", "br-field__label");
  label.textContent = f.label || f.name;
  wrap.appendChild(label);
  if (kind === "select") {
    const s = document.createElement("select");
    s.className = "br-select";
    for (const opt of f.options || []) {
      const o = document.createElement("option");
      o.value = opt;
      o.textContent = opt;
      if (f.value === opt) o.selected = true;
      s.appendChild(o);
    }
    getters.set(f.name, () => s.value);
    wrap.appendChild(s);
    return wrap;
  }
  if (kind === "textarea") {
    const ta = document.createElement("textarea");
    ta.className = "br-textarea";
    if (f.value) ta.value = f.value;
    if (f.placeholder) ta.placeholder = f.placeholder;
    getters.set(f.name, () => ta.value);
    wrap.appendChild(ta);
    return wrap;
  }
  const i = document.createElement("input");
  i.className = "br-input";
  i.type = kind === "number" ? "number" : "text";
  if (f.value) i.value = f.value;
  if (f.placeholder) i.placeholder = f.placeholder;
  getters.set(f.name, () => i.value);
  wrap.appendChild(i);
  return wrap;
}

export function mountChat(client: BioRouterClient, host: HTMLElement): void {
  host.classList.add("br-chat");
  const log = document.createElement("div");
  log.className = "br-chat__log";
  const form = document.createElement("div");
  form.className = "br-chat__form";
  const status = document.createElement("div");
  status.className = "br-run-status";
  const input = document.createElement("input");
  input.className = "br-input";
  input.placeholder = "Ask the agent…";
  const send = document.createElement("button");
  send.className = "br-btn";
  send.type = "button";
  send.textContent = "Send";
  form.appendChild(input);
  form.appendChild(send);
  host.appendChild(log);
  host.appendChild(status);
  host.appendChild(form);
  mountTimeline(client, status, { maxItems: 12 });

  const add = (cls: string, html: boolean, content: string): HTMLElement => {
    const el = document.createElement("div");
    el.className = "br-msg br-msg--" + cls;
    if (html) el.innerHTML = content;
    else el.textContent = content;
    log.appendChild(el);
    log.scrollTop = log.scrollHeight;
    return el;
  };
  if (client.config.greeting) add("agent", false, client.config.greeting);

  let current: HTMLElement | null = null;
  let buffer = "";
  let typing: HTMLElement | null = null;
  const clearTyping = () => {
    if (typing) {
      typing.remove();
      typing = null;
    }
  };
  const showTyping = () => {
    clearTyping();
    typing = document.createElement("div");
    typing.className = "br-msg br-msg--agent";
    typing.innerHTML =
      '<span class="br-typing"><span></span><span></span><span></span></span>';
    log.appendChild(typing);
    log.scrollTop = log.scrollHeight;
  };

  client.on("message", (ev) => {
    if (ev.type !== "message") return;
    clearTyping();
    if (!current) {
      current = add("agent", true, "");
      buffer = "";
    }
    buffer += ev.delta;
    current.innerHTML = client.renderMarkdown(buffer);
    log.scrollTop = log.scrollHeight;
  });
  client.on("tool", (ev) => {
    if (ev.type === "tool") add("tool", false, ev.name + " (" + ev.status + ")");
  });
  client.on("error", () => {
    clearTyping();
    add("agent", false, "Could not reach the agent.");
  });

  const submit = async () => {
    const text = input.value.trim();
    if (!text) return;
    add("user", false, text);
    input.value = "";
    current = null;
    showTyping();
    try {
      await client.prompt(text);
    } catch (e) {
      clearTyping();
      // Show why. "Failed to get a response" told the user nothing actionable —
      // the message from `dial()` names the endpoints tried and the fix.
      add("agent", false, (e as Error).message || "Failed to get a response.");
    }
  };
  send.addEventListener("click", submit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  });
}

/**
 * Read the app config the served page baked in. Prefers a CSP-friendly
 * `<script type="application/json" id="biorouter-app-config">` element (its
 * textContent parsed as JSON) so the page needs no inline executable config;
 * falls back to the legacy `window.BIOROUTER_APP_CONFIG` global for older pages.
 * Malformed JSON in the tag is warned about and also falls back to the global.
 */
function readAppConfig(): Partial<AppConfig> {
  try {
    const el = document.getElementById("biorouter-app-config");
    const text = el ? el.textContent : "";
    if (text && text.trim()) {
      try {
        const parsed = JSON.parse(text);
        if (parsed && typeof parsed === "object") return parsed as Partial<AppConfig>;
      } catch (e) {
        try {
          console.warn(
            "[biorouter] could not parse #biorouter-app-config JSON; falling back to window.BIOROUTER_APP_CONFIG",
            e
          );
        } catch {
          /* console may be unavailable */
        }
      }
    }
  } catch {
    /* no document / getElementById (non-browser host) — use the global */
  }
  return window.BIOROUTER_APP_CONFIG || {};
}

/**
 * Create (and globally register) the app client from the served page's config
 * (a `#biorouter-app-config` JSON script tag, or the legacy
 * `window.BIOROUTER_APP_CONFIG`). Auto-mounts a chat panel when requested.
 */
export function createApp(overrides: Partial<AppConfig> = {}): BioRouterClient {
  const cfg: AppConfig = {
    appId: "app",
    autoChat: true,
    ui: true,
    ...readAppConfig(),
    ...overrides,
  };
  const client = new BioRouterClient(cfg);
  window.BioRouter = client;
  window.Biorouter = client;

  // A persisted theme pack is already stamped onto <html> by the server and owns
  // its complete palette. Do not overwrite it with the legacy light default.
  // Unthemed apps remain deterministic instead of following the viewer's OS;
  // callers can still explicitly request light, dark, or auto.
  const root = document.documentElement;
  if (cfg.theme === "auto") {
    root.removeAttribute("data-br-theme");
  } else if (cfg.theme === "light" || cfg.theme === "dark") {
    root.setAttribute("data-br-theme", cfg.theme);
  } else if (!root.hasAttribute("data-br-pack") && !root.hasAttribute("data-br-theme")) {
    root.setAttribute("data-br-theme", "light");
  }

  // Connect eagerly so agent-driven UI works in apps that never call prompt()
  // (a dashboard the agent populates on load), and so a dead backend surfaces
  // immediately as a banner instead of on the user's first click.
  const start = () => {
    if (cfg.autoChat) {
      const host = document.querySelector<HTMLElement>("[data-br-chat]");
      if (host && !host.dataset.brMounted) {
        host.dataset.brMounted = "1";
        mountChat(client, host);
      }
    }
    client.connect().catch((e: Error) => mountBackendError(e.message));
  };
  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", start);
  else start();
  return client;
}

/**
 * A persistent banner when the turn finished but a worker agent it consulted never
 * answered.
 *
 * The main agent used to receive a soft "the profile did not answer within 120s"
 * TEXT result, treat it as an ordinary paragraph, and complete the turn — so a
 * multi-agent app could show a confident finished answer while half its reasoning
 * had silently timed out. The server now marks the `done` frame; the page renders
 * it regardless of what the model chose to say.
 */
export function showDegradedBanner(missingProfiles: string[]): void {
  const existing = document.querySelector("[data-br-degraded]");
  if (existing) existing.remove();

  const bar = document.createElement("div");
  bar.className = "br-degraded";
  bar.setAttribute("data-br-degraded", "1");
  bar.setAttribute("role", "status");

  const title = document.createElement("strong");
  title.textContent = "Incomplete answer";
  const body = document.createElement("div");
  body.textContent = missingProfiles.length
    ? `These agents were consulted but never answered: ${missingProfiles.join(", ")}. ` +
      `Anything that depended on them is missing from this result.`
    : "An agent this app consulted never answered, so this result is incomplete.";

  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "br-degraded-dismiss";
  dismiss.textContent = "Dismiss";
  dismiss.addEventListener("click", () => bar.remove());

  bar.appendChild(title);
  bar.appendChild(body);
  bar.appendChild(dismiss);
  document.body.appendChild(bar);
}

/**
 * A visible, actionable banner when no Biorouter daemon could be reached. This
 * is the failure an exported app used to show as a bare console error, leaving
 * the user with a UI that silently did nothing.
 */
export function mountBackendError(message: string): void {
  if (document.querySelector("[data-br-backend-error]")) return;
  const bar = document.createElement("div");
  bar.className = "br-backend-error";
  bar.setAttribute("data-br-backend-error", "1");
  bar.setAttribute("role", "alert");
  const title = document.createElement("strong");
  title.textContent = "No Biorouter backend";
  const body = document.createElement("div");
  body.textContent = message;
  bar.appendChild(title);
  bar.appendChild(body);
  document.body.insertBefore(bar, document.body.firstChild);
}

/** Remove the backend-unreachable banner once a connection succeeds. */
export function clearBackendError(): void {
  const bar = document.querySelector("[data-br-backend-error]");
  if (bar) bar.remove();
}
