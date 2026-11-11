/* PREVENT UCSF Risk Calculator. Built by build.mjs from src/ (Node TypeScript stripping). */
(function () {
const __mod = {};
// ---- src/sdk.ts
__mod.sdk = (function () {

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

                            
                                                                         
                
     
                                                                            
                                                        
     
                    
     
                                                                               
                                                                                
                                                                               
                        
     
                       
                                                           
                    
                                                                                   
                     
                                                                                    
               
                                                                        
                                                                            
                                                               
                                    
     
                                                                          
                                                                               
                                                                                
                                                                
     
                   
     
                                                                                
    
                                                                                  
                                                                                   
                                                                                    
     
                         
 

                             
                                     
                   
                                                         
               
 

/** Options for `br.dnd.catalog(...)`. */
                                    
                                                                              
                               
                                                                    
                                               
                                                                           
                  
                                                    
                                                
 

                                
                        
                                                                               
                                                                              
                                                                        
                      
                      
                                                                            
                                                                                 
                                                                         
                 
     
                                                   
    
                                                                                 
                                                                                 
                                                                                 
                                                    
     
                                          
     
                                                                          
                                                                                
                                                                                
                                                
     
                     
 

                                  
                    
 

                                  
                
                 
                
 

/** Events emitted while the agent answers a prompt. */
                        
                                                                                                                                                                            
                                      
                                      
                                                               
                                                                    
                                      
                                                                                
                                                                         
                                                                                
                                                            
                                                                
                                                                                                        
                                                                                            
                                                                                                 
                                                                           
                                                                                
                                                                                
                                                  
                                                                    
                                                                   
                                                   
                                                                                            
                                                         
                                                                      
                                                                         
                                                                     
                                                                              
                                                                                 
                                                                              
                                                                                 
                                                                         
                                                                                 
                                        
                                                                                    
                                                 
                                                                                 
                                                                           
                                                                                          
                                                                                                 
                                             
                                                                            
                                 

                                    
                                         

// Named aliases so the no-esbuild fallback type-stripper can remove these
// annotations (it keys off an uppercase/primitive leading type token, which a
// bare `(() => void)` annotation lacks).
                            
                                   

/** The `capability_report` frame's body (the server's `CapabilityReport`).
 *  Named so the field reads survive the no-esbuild fallback stripper. */
                                       
                               
                            
                            
                                            
                                         
                                         
                                 
 

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
const KNOWN_EVENT_KINDS_MAP                          = {
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

const KNOWN_EVENT_KINDS           = Object.keys(KNOWN_EVENT_KINDS_MAP);

/** Whether this SDK version knows the frame type at all. */
function isKnownEventKind(kind        )          {
  return KNOWN_EVENT_KINDS.indexOf(kind) >= 0;
}

// ── SDK v2 §3.8: multi-agent client facade ───────────────────────────────────
/** The optional `agent` routing field any v2 frame (client→server prompt/call,
 *  or server→client worker stream) may carry. Named so `(ev as AgentTagged)`
 *  survives the no-esbuild fallback stripper (an inline object-type cast does
 *  not). */
                       
                 
 

/** A scoped worker handle from `br.agent(name)`: every outgoing prompt/call
 *  carries `agent:name`, and `on()` fires only for frames whose `agent === name`.
 *  Named so the annotations strip cleanly. */
                       
                                                            
                                                           
                                                                                            
                                                                                         
                                                 
 

/** A prompt awaiting its terminal `done`/`error`, scoped to one worker profile. */
                         
                     
                   
 

/** The routing `agent` of a frame, or `""` for a main-facade frame (no agent). */
function frameAgent(ev         )         {
  const a = (ev               ).agent;
  return typeof a === "string" ? a : "";
}

// ── SDK v2 Phase 3: app.actions / br.call / app.signals ──────────────────────
// Single-ident named aliases (see note above): a bare inline function/object
// annotation would survive the no-esbuild fallback stripper, so every callback
// and record shape used in *code* positions goes through a named type.
                                                
                                           
                           
                                         

/** The `output` frame's fields, named so a `msg as OutputFrame` cast survives the
 *  fallback stripper (an inline object-type cast does not). */
                       
                  
                  
                   
 

/** The `br-network-select` CustomEvent shape (named for a strip-safe cast). */
                           
                    
 
                          
                           
                       
 
                       
                                                
                                                
 

/** Normalised argument bag for `br.call` (and the run-control subset). */
                    
                
                 
                
                         
                      
                      
                                                                                
                 
 

/** What `br.call` resolves to: a structured `value` (from an `output` frame), or
 *  the accumulated `text` of the turn, or `superseded` when a newer call replaced
 *  it before it finished. */
                      
                  
                
                       
 

/** A `br.call` turn awaiting its `output`/`done`/`error`. */
                       
                 
              
                       
                   
                  
                   
                       
                                                                    
                 
 

/** A trailing-edge debounce slot for `br.call`, keyed by call-key. */
                           
                                       
                       
 

/** A pending coalesced signal (trailing-edge, last-payload-wins). */
                     
                   
                                                
 

/** One declared signal from the `ready` surface (`coalesceMs` defaults to 250). */
                      
               
                      
 

/** The `ready` frame's optional advertised surface. */
                        
                         
                     
 

/** Live handle for an in-flight `br.run`, so a superseding run can resolve the
 *  prior one early with its partial text. */
                     
                       
                   
                  
                         
 

/** A trailing-edge debounce slot for `br.run`, keyed by run target. */
                          
                                       
                      
                   
 

// ── SDK v2 Phase 4: br.kb (knowledge bases) + br.model.status ────────────────
// Single-ident named aliases (see the note near ResolveFn): every callback and
// record shape used in a *code* position goes through a named type so the
// no-esbuild fallback stripper can erase the annotation.
/** What a `br.kb` request resolves to — op-shaped and opaque to the SDK. */
                        
/** Resolve fn for a pending `br.kb` request. */
                                       
/** `br.kb.ingest` progress callback. */
                                            
/** `br.model.status()` resolve fn. */
                                                   

/** A `kb_progress` frame streamed during an `ingest`. */
                      
                
                  
               
 
/** Options common to every `br.kb` call (per-call timeout override). */
                      
                     
 
/** Options for `br.kb.search`. */
                        
                 
                     
 
/** The server-side `params` a `br.kb.search` sends. */
                          
                
                 
 
/** Options for `br.kb.ingest`. */
                        
                            
                     
 
/** A `br.kb` request awaiting its terminal `kb_result`. */
                     
                
             
                     
                   
                            
                                                
                   
 
/** The fields of a `kb_result` frame (named for a strip-safe cast). */
                         
                 
                   
                 
 
/** The fields of a `kb_progress` frame (named for a strip-safe cast). */
                           
                 
                 
                  
               
 
/** What `br.model.status()` resolves to. */
                       
                    
                 
                  
                  
 
/** The fields of a `model_status` reply frame (named for a strip-safe cast). */
                            
                    
                 
                  
                  
 
/** A pending `br.model.status()` awaiting its reply (or timeout). */
                              
                              
                   
                                                
                   
 
/** A KB `graph()` result's loose node shape (defensive field access). */
                      
               
                 
                  
                  
                 
                 
                  
 
/** A KB `graph()` result's loose edge shape (defensive field access). */
                      
                   
                   
                 
               
                 
                     
                      
                  
                 
 
/** A KB `graph()` result envelope (`{nodes, edges}`, both loose). */
                       
                    
                    
 

                
                    
                                     
                                
                                
                                                                             
                                                 
   
 

/** Stable per-app client id (persisted) so sessions resume across reloads. */
function getClientId(appId        )         {
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
function sameOriginEndpoint(appId        )                {
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
function resolveEndpoints(cfg           )           {
  const out           = [];
  const add = (e               ) => {
    if (e && out.indexOf(e) < 0) out.push(e);
  };
  // An explicit single endpoint always wins (a remote daemon, a test harness).
  if (cfg.endpoint) add(cfg.endpoint);
  else add(sameOriginEndpoint(cfg.appId));
  for (const e of cfg.endpoints || []) add(e);
  if (!cfg.endpoint) add(sameOriginEndpoint(cfg.appId));
  return out;
}

function withClientId(base        , appId        , token         )         {
  const cid = encodeURIComponent(getClientId(appId));
  const sep = base.indexOf("?") >= 0 ? "&" : "?";
  let url = `${base}${sep}client_id=${cid}`;
  // SDK v2 socket authority: carry the per-app token when the served page
  // supplied one, so the daemon can authorize the upgrade (same-origin only).
  if (token) url += `&token=${encodeURIComponent(token)}`;
  return url;
}

/** Open one WebSocket, resolving on `open` and rejecting on the first error. */
function openSocket(url        )                     {
  return new Promise((resolve, reject) => {
    let ws           ;
    try {
      ws = new WebSocket(url);
    } catch (e) {
      reject(e         );
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
const PROGRESS_SINKS = new Set             ();

/** Whether the app has somewhere of its own to show progress. */
function hasProgressSink()          {
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
                
            
            
            
            
 

/** Parse a computed `rgb()` / `rgba()` colour. Returns null for anything else. */
function parseRgb(value        )              {
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
function luminance(r        , g        , b        )         {
  const f = (c        ) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function contrastRatio(fg      , bg      )         {
  const l1 = luminance(fg.r, fg.g, fg.b);
  const l2 = luminance(bg.r, bg.g, bg.b);
  const hi = Math.max(l1, l2);
  const lo = Math.min(l1, l2);
  return (hi + 0.05) / (lo + 0.05);
}

/** The nearest ancestor with an opaque background — what the text actually sits on. */
function effectiveBackground(el         )              {
  let node                 = el;
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
function auditContrast()           {
  const offenders           = [];
  const candidates = document.querySelectorAll             (
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
function mountRunError(message        )       {
  const host =
    document.querySelector             ("[data-br-chat]") ||
    document.querySelector             (".br-output") ||
    document.body;
  const card = document.createElement("div");
  card.className = "br-run-failed";
  card.setAttribute("data-br-run-error", "1");
  card.setAttribute("role", "alert");
  card.textContent = message;
  host.appendChild(card);
}

class BioRouterClient {
           config           ;
          ws                   = null;
          readyPromise                       = null;
  // Lazily-populated so new/unknown event kinds (BRSDK v2, future frames) work
  // without enumerating every key — `on()` seeds buckets on demand.
          listeners                             = {};
  // Frame types we have already warned about (see `noteUnknownFrame`).
          unknownKinds                          = {};
  // Capabilities advertised by the server in the `ready` frame (deny-by-default).
          capabilities           = [];
  // Durable-session info latched from the `ready` frame.
  sessionId                = null;
  resumed = false;
          activeResolve                   = null;
          activeReject                  = null;
          tokensWaiters                                                                     = [];
          historyWaiters                                                            = [];
  // Last widget tree the server sent for each widget id.
          widgetStore                          = new Map();
  /** The agent-driven UI runtime: applies `ui` frames to the page. */
           ui           ;
  /** The endpoint that actually connected (useful in diagnostics). */
  activeEndpoint                = null;
  // ── SDK v2 Phase 3 ──
  // Author-registered action handlers (`br.actions`), invoked by `app_call`.
          actionHandlers                             = new Map();
  // `br.call` turns awaiting resolution, keyed by callId, plus the currently
  // streaming turn (message deltas / done / error attach to it) and the
  // in-flight *superseding* call per call-key (so a newer one can cancel it).
          pendingCalls                           = new Map();
          callInFlight                           = new Map();
          callDebounce                               = new Map();
          activeCall                     = null;
          callSeq = 0;
  // `br.signals`: client-side trailing-edge coalescing + the `ready`-advertised
  // surface (signal names + coalesce windows, and the action names the agent
  // may `app_call`).
          signalPending                         = new Map();
  /** Frames that must survive a closed socket (see `sendReliable`). */
          outbox            = [];
          signalDeclMap                          = new Map();
          declaredSignals               = [];
          declaredActions           = [];
  // In-flight `br.run` (for supersede) + its trailing-edge debounce slots.
          activeRun                   = null;
          runDebounce                              = new Map();
  // ── SDK v2 Phase 4 ──
  // In-flight `br.kb` requests, keyed by reqId, plus a monotonic reqId counter
  // and a latch for whether a `ready` frame has been seen (so the KB grant
  // pre-check only fires once the capability vocabulary is known).
          pendingKb                         = new Map();
          reqSeq = 0;
          readySeen = false;
  // Callers parked on `br.model.status()` until the reply (or a 10s timeout).
          modelStatusWaiters                       = [];
  // ── SDK v2 Phase 6: ui_error feedback loop ──
  // Send-times of recent `ui_error` frames (rolling 30s window) + a count of
  // errors dropped since the last delivered frame (rides the next one).
          uiErrorTimes           = [];
          uiErrorDropped = 0;
  // ── SDK v2 §3.8: multi-agent facade ──
  // Worker profiles the server advertised in `ready.profiles` (empty = none).
          declaredAgents           = [];
  // In-flight worker prompts + calls, keyed by profile, so a worker frame's
  // `done`/`error` settles the right facade's promise (the main, no-agent path
  // keeps using `activeResolve`/`activeReject`/`activeCall`).
          agentInflight                             = new Map();
          agentActiveCall                           = new Map();
  // Facade-scoped listeners: agent name (or "" for the main facade) → per-kind
  // buckets. Global `on()` still fires for ALL frames (back-compat); this adds
  // the filtered routing so `br.agent(x).on()` sees only x's frames.
          agentListeners                                          = new Map();

  constructor(config           ) {
    this.config = config;
    this.ui = new UiRuntime(this);
  }

  /** Register a listener for an agent event. Returns `this` for chaining. */
  on(kind           , fn          )       {
    (this.listeners[kind] ||= []).push(fn);
    return this;
  }

          emit(ev            )       {
    this.fire(this.listeners[ev.type], ev);
    // Wildcard listeners see EVERY frame, including one whose `type` no per-kind
    // bucket can ever match because this SDK has never heard of it.
    this.fire(this.listeners[ANY_EVENT], ev);
  }

          fire(bucket                        , ev            )       {
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
          noteUnknownFrame(kind        )       {
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
  connect()                {
    if (this.readyPromise) return this.readyPromise;
    this.readyPromise = this.dial().catch((e       ) => {
      // Let a later call retry (e.g. after the user starts biorouterd).
      this.readyPromise = null;
      throw e;
    });
    return this.readyPromise;
  }

          async dial()                {
    const candidates = resolveEndpoints(this.config);
    if (!candidates.length) {
      throw new Error(
        "No Biorouter endpoint to connect to. This page was opened from a file:// URL " +
          "with no fallback configured — serve it with `npm start` instead."
      );
    }
    const tried           = [];
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

          settleActive(err        )       {
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

          handleFrame(raw        )       {
    let msg            ;
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
      this.widgetStore.set(msg.id, msg.tree              );
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
  agents()           {
    return this.declaredAgents.slice();
  }

  /** A scoped handle for a declared worker profile: `{prompt, ask, call, run, on}`
   *  where every outgoing prompt/call carries `agent:name`, and `on()` fires only
   *  for frames whose `agent === name`. An unknown profile (not in
   *  `ready.profiles`) makes the async methods reject immediately with a clear
   *  error; `on()` is a harmless no-op (no frame will ever match). */
  agent(name        )              {
    const self = this;
    const facade              = {
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
          agentReject(name        )                        {
    if (this.declaredAgents.indexOf(name) >= 0) return null;
    const known = this.declaredAgents.join(", ") || "(none)";
    return Promise.reject(
      new Error('Unknown agent profile "' + name + '". Declared profiles: ' + known + ".")
    );
  }

  /** Clone `opts` with `agent` stamped on, for a facade-scoped prompt/run. */
          withAgent(opts                           , name        )                {
    const base = opts || {};
    return { images: base.images, debounceMs: base.debounceMs, supersede: base.supersede, agent: name };
  }

  /** Facade-scoped `call`: normalise, stamp the agent, and schedule. */
          callWithAgent(
    name        ,
    nameOrOpts                   ,
    args         ,
    opts           
  )                      {
    const o = this.normalizeCall(nameOrOpts, args, opts);
    o.agent = name;
    return new Promise((resolve, reject) => {
      this.scheduleCall(o, resolve, reject);
    });
  }

  /** Register a facade-scoped listener (agent name, or "" for the main facade). */
          addAgentListener(name        , kind           , fn          )       {
    let buckets = this.agentListeners.get(name);
    if (!buckets) {
      buckets = {};
      this.agentListeners.set(name, buckets);
    }
    (buckets[kind] ||= []).push(fn);
  }

  /** Dispatch a frame to the facade whose name matches its `agent` (or "" for a
   *  main frame). Errors in a facade listener are non-fatal. */
          routeFacade(msg            )       {
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
          settleAgentPrompt(name        , err        )       {
    const rec = this.agentInflight.get(name);
    if (!rec) return;
    this.agentInflight.delete(name);
    if (err) rec.reject(err);
    else rec.resolve();
  }

  /** Settle a worker profile's in-flight `br.call` (resolve `{text}` on done). */
          settleAgentCall(name        , err              )       {
    const pending = this.agentActiveCall.get(name);
    if (!pending) return;
    if (err) this.rejectCall(pending, err);
    else this.resolveCall(pending, { text: pending.textBuf });
  }

  /** Reject every in-flight worker prompt/call (on socket close/error). */
          rejectAllAgents(err       )       {
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
  history()                                                 {
    return new Promise((resolve) => {
      this.historyWaiters.push(resolve);
      if (!this.send({ type: "history" })) {
        this.historyWaiters = this.historyWaiters.filter((w) => w !== resolve);
        resolve([]);
      }
    });
  }

  /** Current context-window usage (BRSDK context API). */
  tokens()                                                          {
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
          httpBase()         {
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
  async listModels()                     {
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
  selectModel(provider        , model        )       {
    this.send({ type: "modelselect", provider, model });
  }

  /** Namespaced model surface: `br.model.list()` / `br.model.select(p, m)` /
   *  `br.model.status()` (is the routed provider ready — e.g. is llamacpp still
   *  downloading, what context size). */
  get model() {
    return {
      list: () => this.listModels(),
      select: (provider        , model        ) => this.selectModel(provider, model),
      status: (timeoutMs         ) =>
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
      search: (query        , opts               ) => this.kbSearch(query, opts),
      page: (path        , opts             ) => this.kbRequest("page", { path: path }, opts),
      graph: (opts             ) => this.kbRequest("graph", {}, opts),
      history: (limit         , opts             ) =>
        this.kbRequest("history", typeof limit === "number" ? { limit: limit } : {}, opts),
      ingest: (items         , opts               ) => this.kbIngest(items, opts),
      graphToNetwork: (graph         ) => graphToNetwork(graph),
    };
  }

  // ── SDK v2 Phase 4: br.kb request plumbing ──────────────────────────────────

          kbSearch(query        , opts               )                    {
    const o = opts || {};
    const q = typeof query === "string" ? query : "";
    const params                 = { query: q };
    if (typeof o.limit === "number") params.limit = o.limit;
    return this.kbRequest("search", params, o);
  }

          kbRequest(op        , params         , opts             )                    {
    const o = opts || {};
    const timeoutMs = typeof o.timeoutMs === "number" && o.timeoutMs > 0 ? o.timeoutMs : 30000;
    if (this.kbGrantMissing()) return Promise.reject(this.kbGrantError());
    return new Promise((resolve, reject) => {
      this.startKb(op, params, timeoutMs, undefined, resolve, reject);
    });
  }

          kbIngest(items         , opts               )                    {
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
          kbGrantMissing()          {
    if (!this.readySeen) return false;
    const kbTokens = ["data:knowledge", "data", "kb", "knowledge"];
    for (const t of kbTokens) {
      if (this.has(t)) return false;
    }
    return true;
  }

          kbGrantError()        {
    return new Error(
      'br.kb: this app has no knowledge-base grant — declare a data.sources[kind:"knowledge"] ' +
        "capability in the manifest to read or ingest knowledge bases."
    );
  }

          startKb(
    op        ,
    params         ,
    timeoutMs        ,
    onProgress                          ,
    resolve           ,
    reject          
  )       {
    const reqId = this.freshReqId();
    const timer = setTimeout(() => {
      const p = this.pendingKb.get(reqId);
      if (p) this.rejectKb(p, new Error("br.kb " + op + " timed out after " + timeoutMs + "ms"));
    }, timeoutMs);
    const pending            = {
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
      (e       ) => this.rejectKb(pending, e)
    );
  }

          freshReqId()         {
    this.reqSeq++;
    return "req-" + this.reqSeq + "-" + Date.now().toString(36);
  }

          resolveKb(msg            )       {
    const frame = msg                 ;
    const reqId = typeof frame.reqId === "string" ? frame.reqId : "";
    const pending = reqId ? this.pendingKb.get(reqId) : undefined;
    if (!pending) return;
    if (typeof frame.error === "string" && frame.error) this.rejectKb(pending, new Error(frame.error));
    else this.settleKb(pending, frame.result);
  }

          progressKb(msg            )       {
    const frame = msg                   ;
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

          settleKb(pending           , result         )       {
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

          rejectKb(pending           , err       )       {
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

          requestModelStatus(timeoutMs        )                       {
    return new Promise((resolve, reject) => {
      const rec                     = { resolve: resolve, reject: reject, timer: 0, settled: false };
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
        (e       ) => this.failModelStatus(rec, e)
      );
    });
  }

          resolveModelStatus(msg            )       {
    const frame = msg                    ;
    const status              = {
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

          failModelStatus(rec                    , err       )       {
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
      get: (path         ) => this.ui.stateGet(path),
      set: (path        , value         ) => this.ui.stateSet(path, value),
      remove: (path        ) => this.ui.stateRemove(path),
      update: (fn            ) => this.ui.stateUpdate(fn),
      subscribe: (path        , fn          ) => this.ui.stateSubscribe(path, fn),
    };
  }

  /** Author component registry (`br.components.register(name, def)`). A
   *  `component` node renders a container and calls the registered `mount`;
   *  `set_props`/`replace` call `update` if present, else remount. Props are
   *  agent-controlled — treat them as untrusted input. */
  get components() {
    return {
      register: (name        , def              ) => this.ui.registerComponent(name, def),
    };
  }

  /** HITL: approve a pending tool surfaced via an `approval` event.
   *  `action` is "allow_once" (default) or "always_allow". */
  approve(requestId        , action         = "allow_once")       {
    this.send({ type: "approve", request: requestId, action });
  }

  /** HITL: reject a pending tool, with an optional human-readable reason. */
  reject(requestId        , reason         )       {
    this.send({ type: "reject", request: requestId, reason });
  }

  /** Render a widget tree into `target` and wire its actions back to the agent
   *  (a submit button sends a `widget_action` frame scoped to this widget id). */
          renderWidgetInto(
    id        ,
    tree            ,
    target                      
  )              {
    const host =
      typeof target === "string"
        ? (document.querySelector(target)                      )
        : target;
    const ctx                = {
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
      render: (id        , tree            , target                      ) =>
        this.renderWidgetInto(id, tree, target),
      action: (widgetId        , action        , payload          ) =>
        this.send({ type: "widget_action", widgetId, action, payload }),
      // alias kept for symmetry with the agent-side naming
      submit: (widgetId        , action        , payload          ) =>
        this.send({ type: "widget_action", widgetId, action, payload }),
      get: (id        ) => this.widgetStore.get(id),
    };
  }

  /** Whether the server advertised a given BRSDK capability in `ready`. */
  has(capability        )          {
    return this.capabilities.indexOf(capability) >= 0;
  }

  /** Send a raw client frame. Public so the UI runtime can answer `ui_ask`. */
  sendRaw(frame         )          {
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
  reportUiError(where        , message        , instance         )       {
    try {
      const now = Date.now();
      const cutoff = now - 30000;
      const kept           = [];
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
          send(frame         )          {
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
          sendReliable(frame         )       {
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
          flushOutbox()       {
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
  async prompt(text        , opts                = {})                {
    await this.connect();
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error("Not connected to the Biorouter backend.");
    }
    const agent = opts.agent;
    return new Promise      ((resolve, reject) => {
      // A worker prompt settles against its own profile's `done`/`error`; the
      // main (no-agent) prompt keeps using the shared active slots (§3.8).
      if (agent) this.agentInflight.set(agent, { resolve: resolve, reject: reject });
      else {
        this.activeResolve = resolve;
        this.activeReject = reject;
      }
      this.ws .send(
        JSON.stringify({ type: "prompt", text, images: opts.images || [], agent: agent })
      );
    });
  }

  /** Ask the agent to stop the current turn. */
  cancel()       {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: "cancel" }));
    }
  }

  /** Remove a previously-registered listener. */
  off(kind           , fn          )       {
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
  onAny(fn          )       {
    (this.listeners[ANY_EVENT] ||= []).push(fn);
    return this;
  }

  /** Remove a wildcard listener registered with `onAny`. */
  offAny(fn          )       {
    const arr = this.listeners[ANY_EVENT];
    if (arr) {
      const i = arr.indexOf(fn);
      if (i >= 0) arr.splice(i, 1);
    }
    return this;
  }

  /** Convenience: collect the full reply for one prompt as a single string. */
  async ask(text        , opts                = {})                  {
    let buf = "";
    // A worker ask (opts.agent set) accumulates only that profile's deltas; the
    // main ask accumulates only un-tagged (no-agent) deltas — a no-op filter for
    // single-agent apps, where no frame carries an `agent` at all (§3.8).
    const want = opts.agent || "";
    const onMsg           = (ev) => {
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
    text        ,
    target                      ,
    opts                = {}
  )                  {
    const key = typeof target === "string" ? target : "@el";
    // Trailing-edge debounce: within the window, only the last run on this
    // target survives; earlier ones resolve early with "" (no partial yet).
    if (opts.debounceMs && opts.debounceMs > 0) {
      return this.debounceRun(key, text, target, opts);
    }
    return this.startRun(text, target, opts);
  }

          debounceRun(
    key        ,
    text        ,
    target                      ,
    opts               
  )                  {
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
          startRun(
    text        ,
    target                      ,
    opts               
  )                  {
    const superseding = !!opts.supersede;
    if (superseding && this.activeRun && this.activeRun.superseding && !this.activeRun.settled) {
      this.cancel();
      this.activeRun.resolveEarly();
    }
    let userResolve             = () => {};
    let userReject           = () => {};
    const userPromise = new Promise        ((res, rej) => {
      userResolve = res;
      userReject = rej;
    });
    const handle            = {
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
        ? document.querySelector             (target)
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
        const answer = el.querySelector             (".br-run-answer");
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

          async launchRun(
    text        ,
    target                      ,
    opts               ,
    handle           ,
    userResolve            ,
    userReject          
  )                {
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
        userReject(e         );
      }
    } finally {
      if (this.activeRun === handle) this.activeRun = null;
    }
  }

          runChain                = Promise.resolve();

          async doRun(
    text        ,
    target                      ,
    opts                = {},
    handle            
  )                  {
    // `startRun` already resolved the target on the click and reported a miss
    // visibly; this re-resolve only covers a direct internal caller.
    const el =
      typeof target === "string"
        ? document.querySelector             (target)
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
          ? document.querySelector             (opts.progress)
          : opts.progress || null;
    const routeAway =
      opts.progress === false || !!explicitProgress || hasProgressSink();

    el.innerHTML = routeAway
      ? '<div class="br-run-answer"><span class="br-spinner"></span> Starting agent run…</div>'
      : '<div class="br-run-status"></div><div class="br-run-answer"><span class="br-spinner"></span> Starting agent run…</div>';
    const statusEl = el.querySelector             (".br-run-status");
    const answerEl = el.querySelector             (".br-run-answer") ;

    // Only mount a timeline of our own when the app has nowhere else to show one.
    const timelineHost = explicitProgress || statusEl;
    const stopTimeline =
      opts.progress === false || !timelineHost
        ? () => undefined
        : mountTimeline(this, timelineHost, { maxItems: 18 });
    // A worker run streams only its profile's frames; a main run streams only
    // un-tagged frames (a no-op filter for single-agent apps) (§3.8).
    const want = opts.agent || "";
    const onMsg           = (ev) => {
      if (ev.type === "message" && frameAgent(ev) === want) {
        buf += ev.delta;
        if (handle) handle.partial = buf;
        answerEl.innerHTML = this.renderMarkdown(buf);
      }
    };
    const onTool           = (ev) => {
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
      const emsg = (e         ).message || "request failed";
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
  renderMarkdown(md        )         {
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
      register: (name        , handler               ) => this.registerAction(name, handler),
      list: () => this.listActions(),
    };
  }

          registerAction(name        , handler               )       {
    if (name && typeof handler === "function") this.actionHandlers.set(String(name), handler);
  }

          listActions()           {
    const out           = [];
    this.actionHandlers.forEach((_v, k) => out.push(k));
    return out;
  }

  /** Dispatch an `app_call` frame to a registered handler and reply with
   *  `app_result`. Never throws — every failure path answers the call. */
          dispatchAction(cmd           )       {
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
            err && (err         ).message ? String((err         ).message) : "action handler failed";
          this.send({ type: "app_result", callId: callId, error: emsg });
          // Also surface it on the ui_error feedback loop (rate-limited).
          this.reportUiError("action:" + action, errText(err));
        }
      );
  }

  /** Cap an action result at 64 KB of serialized JSON. Over the cap, the JSON is
   *  truncated (with a `…[truncated]` marker) inside a wrapper object so the
   *  agent still receives a well-formed, bounded value. */
          serializeActionResult(result         )          {
    let json        ;
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
  call(nameOrOpts                   , args          , opts           )                      {
    const o = this.normalizeCall(nameOrOpts, args, opts);
    return new Promise((resolve, reject) => {
      this.scheduleCall(o, resolve, reject);
    });
  }

          normalizeCall(nameOrOpts                   , args          , opts           )           {
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

          callKey(o          )         {
    const base = o.name || o.text || "__call";
    // Namespace the debounce/supersede key by profile so a worker's calls never
    // collide with the main agent's on the same name/text (§3.8).
    return o.agent ? o.agent + "::" + base : base;
  }

          scheduleCall(o          , resolve             , reject          )       {
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

          dispatchCall(o          , key        , resolve             , reject          )       {
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
    const pending              = {
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
      (e       ) => this.rejectCall(pending, e)
    );
  }

          freshCallId()         {
    this.callSeq++;
    return "call-" + this.callSeq + "-" + Date.now().toString(36);
  }

          resolveOutput(msg            )       {
    const frame = msg               ;
    const cid = typeof frame.callId === "string" ? frame.callId : "";
    // No callId → fall back to the active call for this frame's profile (§3.8):
    // a worker frame settles that worker's call, a main frame settles activeCall.
    const agent = frameAgent(msg);
    const fallback = agent ? this.agentActiveCall.get(agent) : this.activeCall;
    const pending = cid ? this.pendingCalls.get(cid) : fallback;
    if (!pending) return;
    this.resolveCall(pending, { value: frame.value });
  }

          settleActiveCall(err              )       {
    const pending = this.activeCall;
    if (!pending) return;
    if (err) this.rejectCall(pending, err);
    else this.resolveCall(pending, { text: pending.textBuf });
  }

          resolveCall(pending             , result            )       {
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

          rejectCall(pending             , err       )       {
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
      emit: (name        , payload         ) => this.emitSignal(name, payload),
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
      catalog: (opts                   ) => this.mountDndCatalog(opts),
    };
  }

          mountDndCatalog(opts                   ) {
    const source =
      typeof opts.source === "string"
        ? document.querySelector             (opts.source)
        : opts.source;
    const zones                =
      typeof opts.target === "string"
        ? Array.from(document.querySelectorAll             (opts.target))
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
      Array.from(source.querySelectorAll             ("[data-br-item]"));
    let picked                     = null;
    let zoneIdx = 0;

    const live = document.createElement("div");
    live.className = "br-sr-only";
    live.setAttribute("aria-live", "polite");
    source.appendChild(live);
    const announce = (m        ) => {
      live.textContent = m;
    };

    const itemId = (el             ) => el.dataset.brItem || el.id || "";
    const zoneId = (el             ) => el.dataset.brZone || el.id || "";

    const setPicked = (el                    ) => {
      if (picked) picked.classList.remove("is-picked");
      picked = el;
      if (picked) {
        picked.classList.add("is-picked");
        announce(
          `${itemId(picked)} picked up. Choose a zone, then press Enter to drop.`
        );
      }
    };

    const highlight = (zone                    ) => {
      for (const z of zones) z.classList.toggle("is-over", z === zone);
    };

    const drop = (item             , zone             ) => {
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

    const zoneAt = (x        , y        )                     =>
      zones.find((z) => {
        const r = z.getBoundingClientRect();
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      }) || null;

    // ── Pointer path (works for a real mouse AND a synthetic coordinate drag) ──
    const onPointerDown = (e              ) => {
      const item = (e.target                      )?.closest             (
        "[data-br-item]"
      );
      if (!item || !source.contains(item)) return;
      e.preventDefault();
      item.setPointerCapture?.(e.pointerId);
      setPicked(item);

      const onMove = (m              ) => highlight(zoneAt(m.clientX, m.clientY));
      const onUp = (u              ) => {
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
    const onZoneClick = (e       ) => {
      const zone = e.currentTarget               ;
      if (picked) drop(picked, zone);
    };

    // ── Keyboard parity ─────────────────────────────────────────────────────
    const onKeyDown = (e               ) => {
      const item = (e.target                      )?.closest             (
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

          emitSignal(name        , payload         )       {
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
    const rec            = { payload: payload, timer: 0 };
    rec.timer = setTimeout(() => {
      this.signalPending.delete(name);
      this.sendReliable({ type: "signal", name: name, payload: rec.payload });
    }, coalesceMs);
    this.signalPending.set(name, rec);
  }

  /** Latch the `ready` frame's advertised surface. Absent/partial surfaces are
   *  tolerated (Phase-3 servers add it; older ones do not). */
          latchSurface(surface               )       {
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
  autoEmitNodeSelected(nodeId               , instanceId               )       {
    if (!this.signalDeclMap.has("node_selected")) return;
    this.emitSignal("node_selected", { id: nodeId, instance: instanceId });
  }
}

// ---------------------------------------------------------------------------
// SDK v2 Phase 4: br.kb.graphToNetwork — pure KB-graph → network-spec mapper.
// ---------------------------------------------------------------------------

/** First non-empty string in `vals`, else `undefined`. Used to pick a field
 *  across the several names a KB graph might use (id/name, label/title, …). */
function firstString(vals           )                     {
  for (const v of vals) {
    if (typeof v === "string" && v) return v;
  }
  return undefined;
}

/** Map one KB graph node (object or a bare id string) into a `network` NetNode,
 *  or `null` when it has no usable id. Type is optional — missing-type nodes are
 *  tolerated (they simply carry no `type`). */
function kbNode(n         )                 {
  if (typeof n === "string") return n ? { id: n, label: n } : null;
  if (!n || typeof n !== "object") return null;
  const o = n              ;
  const id = firstString([o.id, o.name]);
  if (!id) return null;
  const label = firstString([o.label, o.title, o.name]) || id;
  const type = firstString([o.type, o.kind, o.group]);
  const node          = { id: id, label: label };
  if (type) node.type = type;
  return node;
}

/** Map one KB graph edge into a `network` NetEdge, or `null` when either
 *  endpoint is missing. Accepts `source`/`target` or `from`/`to`, and derives
 *  `kind` from any of kind/relation/predicate/label/type (all optional). */
function kbEdge(e         )                 {
  if (!e || typeof e !== "object") return null;
  const o = e              ;
  const source = firstString([o.source, o.from]);
  const target = firstString([o.target, o.to]);
  if (!source || !target) return null;
  const kind = firstString([o.kind, o.relation, o.predicate, o.label, o.type]);
  const edge          = { source: source, target: target };
  if (kind) edge.kind = kind;
  return edge;
}

/** Map a knowledge-base `graph()` result (`{nodes, edges}`) into the `network`
 *  component's spec shape (`{nodes:[{id,label,type}], edges:[{source,target,kind}]}`)
 *  so an `explorer` app can do `ui.network(...)` in one line. Pure and defensive:
 *  a missing/mis-shaped graph yields empty arrays, id-less nodes and dangling
 *  edges are dropped, and missing `type`/`kind` are simply omitted. */
function graphToNetwork(graph         )              {
  const g = (graph && typeof graph === "object" ? graph : {})               ;
  const rawNodes = Array.isArray(g.nodes) ? g.nodes : [];
  const rawEdges = Array.isArray(g.edges) ? g.edges : [];
  const nodes            = [];
  for (const n of rawNodes) {
    const node = kbNode(n);
    if (node) nodes.push(node);
  }
  const edges            = [];
  for (const e of rawEdges) {
    const edge = kbEdge(e);
    if (edge) edges.push(edge);
  }
  return { nodes: nodes, edges: edges };
}

// ---------------------------------------------------------------------------
// Markdown rendering (compact, no external deps, HTML-escaped first).
// ---------------------------------------------------------------------------

function escapeHtml(s        )         {
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

function inline(s        )         {
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
function normalizeSeries(spec           )                {
  const clean = (pts                          )               =>
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
function renderChart(json        )         {
  let spec           ;
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
  const x = (i        ) => padL + (plotW * (i + 0.5)) / Math.max(n, 1);
  const y = (v        ) => padT + plotH * (1 - (v - min) / span);
  const esc = (s        ) => escapeHtml(String(s));
  const palette = CHART_PALETTE;
  const color = (i        ) => palette[i % palette.length];
  const title = spec.title
    ? `<text x="${W / 2}" y="18" font-size="12" font-weight="600" text-anchor="middle" fill="var(--br-text)">${esc(
        spec.title
      )}</text>`
    : "";
  const svg = (inner        ) =>
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

function parseGraphEdges(text        )              {
  const nodeMap = new Map                   ();
  const edges              = [];
  const ensure = (id        ) => {
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

function normalizeGraph(input        )                   {
  try {
    const spec = JSON.parse(input)             ;
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
function renderGraph(input        )         {
  const spec = normalizeGraph(input);
  if (!spec) return '<div class="br-visual br-visual--pending"></div>';
  const rawNodes = Array.isArray(spec.nodes) ? spec.nodes : [];
  const rawEdges = Array.isArray(spec.edges) ? spec.edges : [];
  const nodes = new Map                   ();
  const ensureNode = (id        , label         ) => {
    const clean = String(id || "").trim();
    if (!clean) return;
    if (!nodes.has(clean)) nodes.set(clean, { id: clean, label: label || clean });
  };
  for (const n of rawNodes) {
    if (typeof n === "string") ensureNode(n);
    else ensureNode(n.id, n.label);
  }
  const edges              = [];
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
  const esc = (s        ) => escapeHtml(String(s));
  const pos = new Map                    ();
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
      const p = pos.get(n.id) ;
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
function withFenceIid(html        , kind        )         {
  fenceSeq++;
  const id = kind + "#" + fenceSeq;
  return html.replace(/^<div /, '<div data-br-iid="' + id + '" ');
}

function renderMarkdown(md        )         {
  const src = escapeHtml(md || "");
  const lines = src.split("\n");
  const out           = [];
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
      const code           = [];
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
      const cells = (row        ) =>
        row
          .trim()
          .replace(/^\||\|$/g, "")
          .split("|")
          .map((c) => c.trim());
      const header = cells(line);
      i += 2; // skip header + separator
      const body             = [];
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
      const liText = (ul || ol) [1];
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
    const para           = [line];
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
function fileToImageInput(file      )                      {
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

function eventSummary(ev            )                         {
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
      const rep                       = ev.report || {};
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
  client                 ,
  row             ,
  requestId        
)       {
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
  const settle = (verdict        ) => {
    // Replacing the children removes both buttons, so a second click cannot
    // send a second decision for a request the daemon has already resolved.
    actions.textContent = verdict;
    actions.setAttribute("data-br-approval-settled", verdict);
  };
  const button = (label        , kind        , verdict        , pick           ) => {
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
function settleApprovalControls(host             , requestId        , verdict        )       {
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
function mountTimeline(
  client                 ,
  target                      ,
  options                  = {}
) {
  const host =
    typeof target === "string"
      ? (document.querySelector(target)                      )
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
  const entries                = [];
  const add = (ev            ) => {
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

                        
                                                         
                                        
                                        
                                                                     
                                 
                                                                          
                                 
                                 
                                                                                        
                    
                                                    
                                                                                                          
                                                                                                                             
                                                                      
                                                                                      
                                         
                                                                          
                                              
                                                                            
                                                                                                             
                                                              
                                              
                                                    
                                                                  
                                            
                                                              

// Node shapes for the science-pack renderers. Interfaces are dropped wholesale by
// the no-esbuild fallback stripper, so they never leak into the emitted JS.
                          
                 
               
 
                            
                 
                 
             
             
 
                             
                
                     
                       
                    
 
                           
                
                 
                     
                        
                    
                 
                     
                     
 
                          
             
                 
                
                
                 
 
                          
                 
                 
                
                 
                 
 
                              
                                       
                                    
                           
 
                             
                  
                        
                   
                   
 
                              
                 
                    
                    
                         
                       
                         
 
                                    
                                  
                                  
                                        
                  
 
              
            
            
 
// Author-registered components (Task 3). `props` is agent-controlled — untrusted.
                                   
             
                 
             
 
                               
                  
                 
                    
 
// Small helper node shapes used by the renderers/registry below.
                     
               
               
                   
 
                   
                 
                          
                          
                
 
                   
                    
               
 
                      
                
                
 
                         
             
                
                  
 
                         
                   
                  
 
                   
              
              
                  
                  
                 
                    
                    
 
                     
               
                
              
                
 
                      
                
                 
                  
 
// Named callback/marker aliases so the fallback stripper drops the annotations
// (a bare `(id: string) => void` has no leading uppercase/primitive token).
                                        
                                                                                                   
                                                                                
                                                                         
                                                                                          
                                  
                                               

                                
                                                                       
                                              
                                                                       
                                                       
                                                               
                                    
 

function wEl(tag        , cls         )              {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

// A widget node whose `t` is not one we render. Named so the no-esbuild
// fallback stripper removes the `as UnknownWidget` cast cleanly (an inline
// `as { t?: string }` object-type cast would confuse its line scanner).
                                    
// Warn at most once per unknown widget kind, so a forward-compatible frame full
// of a new kind doesn't flood the console.
const warnedWidgetKinds              = new Set();

/** Build a detached DOM subtree from a widget node. Recursive; unknown node
 *  types render a neutral labeled placeholder rather than throwing, so a
 *  newer agent emitting a not-yet-supported kind degrades gracefully. */
function renderWidget(node            , ctx               )              {
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
        let payload         ;
        if (submit) {
          const collected                                   = {};
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
        const mounted = ctx.mountComponent(name, node.props, (node              ).id);
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
      const kind = String((node                 ).t || "unknown");
      return unknownWidgetEl(kind);
    }
  }
}

/** The neutral, warn-once placeholder for an unknown/unregistered widget kind. */
function unknownWidgetEl(kind        )              {
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
function isSafeImageSrc(s        )          {
  const v = (s || "").trim().toLowerCase();
  if (v.indexOf("javascript:") === 0) return false;
  if (v.indexOf("http:") === 0) return false;
  return true;
}

function renderImage(node           )              {
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

function renderKpi(node         )              {
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

function logLineEl(ln         )              {
  const row = wEl("div", "br-log__line");
  const level = ln && ln.level ? String(ln.level).toLowerCase() : "";
  if (level) row.classList.add("br-log__line--" + level);
  row.textContent = ln && ln.text != null ? String(ln.text) : "";
  return row;
}

function logCap(node         )         {
  return typeof node.max === "number" && node.max > 0 ? node.max : LOG_CAP;
}

function atLogBottom(box             )          {
  return box.scrollHeight - box.scrollTop - box.clientHeight < 4;
}

function renderLog(node         )              {
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
function appendLogLines(box             , node         , add           )       {
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

function renderFigure(node            )              {
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
  const onMsg = (ev              ) => {
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

function plotFrame(inner        , title         )         {
  const t = title
    ? `<text x="${PLOT_W / 2}" y="18" font-size="12" font-weight="600" text-anchor="middle" fill="var(--br-text)">${escapeHtml(
        title
      )}</text>`
    : "";
  return `<div class="br-chart"><svg viewBox="0 0 ${PLOT_W} ${PLOT_H}" width="100%" preserveAspectRatio="xMidYMid meet" role="img">${t}${inner}</svg></div>`;
}

const PLOT_PENDING = '<div class="br-chart br-chart--pending"></div>';

function renderPlot(spec          )         {
  if (!spec || typeof spec !== "object") return PLOT_PENDING;
  const type = spec.type || "bar";
  if (type === "scatter") return renderScatter(spec);
  if (type === "area") return renderArea(spec);
  if (type === "box") return renderBox(spec);
  if (type === "heatmap") return renderHeatmap(spec);
  // bar / line / pie reuse the existing chart renderer verbatim.
  return renderChart(JSON.stringify(spec));
}

function cleanPoints(pts              )              {
  const out              = [];
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

function scatterSeries(spec          )               {
  if (Array.isArray(spec.series) && spec.series.length) {
    return spec.series.map((s) => ({ name: s.name, points: cleanPoints(s.points || s.data) }));
  }
  const p = cleanPoints(spec.data);
  return p.length ? [{ points: p }] : [];
}

function renderScatter(spec          )         {
  const series = scatterSeries(spec);
  if (!series.length) return PLOT_PENDING;
  const padL = 44;
  const padR = 14;
  const padB = 34;
  const padT = spec.title ? 28 : 14;
  const allX           = [];
  const allY           = [];
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
  const sx = (v        ) => padL + (plotW * (v - xmin)) / xspan;
  const sy = (v        ) => padT + plotH * (1 - (v - ymin) / yspan);
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

function renderArea(spec          )         {
  const series = normalizeSeries(spec             );
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
  const x = (i        ) => padL + (plotW * (i + 0.5)) / Math.max(n, 1);
  const y = (v        ) => padT + plotH * (1 - (v - min) / span);
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

function quartiles(vals          )           {
  const s = vals.filter((v) => isFinite(v)).slice().sort((a, b) => a - b);
  if (!s.length) return [0, 0, 0, 0, 0];
  const q = (p        ) => {
    const idx = (s.length - 1) * p;
    const lo = Math.floor(idx);
    const hi = Math.ceil(idx);
    if (lo === hi) return s[lo];
    return s[lo] + (s[hi] - s[lo]) * (idx - lo);
  };
  return [s[0], q(0.25), q(0.5), q(0.75), s[s.length - 1]];
}

function seriesValues(s            )           {
  if (Array.isArray(s.values)) return s.values;
  const out           = [];
  const pts = s.data || s.points || [];
  for (const d of pts) {
    const v = d && typeof d.value === "number" ? d.value : Number(d && d.value);
    if (isFinite(v)) out.push(v);
  }
  return out;
}

function boxSeries(spec          )               {
  if (Array.isArray(spec.series) && spec.series.length) {
    return spec.series.map((s) => ({ name: s.name, values: seriesValues(s) }));
  }
  if (Array.isArray(spec.values)) return [{ values: spec.values }];
  return [];
}

function renderBox(spec          )         {
  const series = boxSeries(spec);
  if (!series.length) return PLOT_PENDING;
  const stats = series.map((s) => quartiles((s.values || []).slice(0, 2000)));
  const padL = 44;
  const padR = 14;
  const padB = 40;
  const padT = spec.title ? 28 : 14;
  const allv           = [];
  for (const st of stats) {
    allv.push(st[0]);
    allv.push(st[4]);
  }
  const max = Math.max(...allv);
  const min = Math.min(...allv);
  const span = max - min || 1;
  const plotW = PLOT_W - padL - padR;
  const plotH = PLOT_H - padT - padB;
  const y = (v        ) => padT + plotH * (1 - (v - min) / span);
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

function renderHeatmap(spec          )         {
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

function createNetwork(spec             )              {
  const wrap = wEl("div", "br-network");
  const canvas = document.createElement("canvas");
  canvas.className = "br-network__canvas";
  wrap.appendChild(canvas);
  const controller = buildNetworkEngine(canvas, spec || {});
  (wrap           ).__brNet = controller;
  (canvas           ).__brNet = controller;
  return wrap;
}

function buildNetworkEngine(canvas                   , spec             )                    {
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

function getDpr()         {
  try {
    return window.devicePixelRatio || 1;
  } catch {
    return 1;
  }
}

// ── author component registry (Task 3) ───────────────────────────────────────

class ComponentRegistry {
          defs                            = new Map();
  register(name        , def              )       {
    if (name && def && typeof def.mount === "function") this.defs.set(String(name), def);
  }
  get(name        )                           {
    return this.defs.get(String(name));
  }
}

// ── instance morphing helpers (Task 1) ───────────────────────────────────────

/** The network controller attached to a rendered `network` instance, if any. */
function readNet(el             )                           {
  const n = (el           ).__brNet;
  return n ? n : null;
}

/** Carry sim positions from an old network instance to its replacement (matched
 *  by node id), then tear the old one down so its animation loop stops. */
function adoptNetwork(oldEl             , newEl             )       {
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

function shallowMergeNode(node            , props           )             {
  const out = Object.assign({}, node, props);
  return out              ;
}

/** Snapshot the focused input inside `root` so a re-render can restore it. */
function captureFocus(root             )                   {
  const active = document.activeElement                      ;
  if (!active || !root.contains(active)) return null;
  const tag = active.tagName ? active.tagName.toLowerCase() : "";
  if (tag !== "input" && tag !== "textarea" && tag !== "select") return null;
  const field = active                    ;
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
function restoreFocus(root             , snap                  )       {
  if (!snap) return;
  let sel = null;
  const list = root.querySelectorAll("input, textarea, select");
  for (let i = 0; i < list.length; i++) {
    const el = list[i]               ;
    if ((el.getAttribute("name") || "") === snap.name) {
      sel = el;
      break;
    }
  }
  if (!sel) return;
  const field = sel                    ;
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
function captureScroll(root             )             {
  const idx           = [];
  const tops           = [];
  const lefts           = [];
  const all = root.querySelectorAll("*");
  for (let i = 0; i < all.length; i++) {
    const el = all[i]               ;
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

function restoreScroll(root             , snap            )       {
  if (!snap || !snap.idx.length) return;
  const all = root.querySelectorAll("*");
  for (let k = 0; k < snap.idx.length; k++) {
    const el = all[snap.idx[k]]               ;
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

                            
                
              
              
                        
                 
                      
                        
                   
                  
                
                       
                   
                         
                          
                  
                               
                   
                 
                     
                                  
                                                  
                                                                 
             
                                                                               
                                                                                 
                
                   
                  
                     
                  
                       
                          
                                                              
                  
                                                                        
                                                                               
                  
                  
                 
                                                              
                                                                               
                                                                    
                
                    
                    
                                                
                                                                               
                   
                                    
                                                                                
                                                               
                 
                                                                        
                        
                                                                             
                                                                    
                 
 

/** One `ui_suggest` chip: a label plus an optional prompt to send on click. */
                              
                
                  
 

                               
               
                 
                
                     
                 
                       
 

// Named aliases (see the note near `Listener`): the no-esbuild fallback stripper
// keys off an uppercase/primitive leading type token, so a bare
// `(cmd: UiCommand) => void` annotation would survive into the emitted JS.
                                                              
                                                
                                             
                                                        
// Same reason: `br.state` callback/return types go through named aliases so the
// fallback stripper drops the annotation (an inline `(v: unknown) => void` has
// no leading uppercase/primitive token and would survive into the JS).
                                         
                                            
                        

/** Where a `place` maps in the DOM. `dock` is the always-available drawer. */
const DOCK_PLACES                         = {
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
                                         
                          

/** One declarative binding: an element + how it consumes a pointer value. */
/**
 * Read a form control's value in the type the state document should hold.
 *
 * A `<input type=range>` yields the STRING "0.37"; writing that into state and
 * then comparing it against a number is a silent-divergence bug waiting to
 * happen, so range/number controls coerce to a real number here.
 */
function readControlValue(el             )          {
  const tag = el.tagName.toLowerCase();
  if (tag === "input") {
    const input = el                    ;
    const type = (input.type || "text").toLowerCase();
    if (type === "checkbox") return input.checked;
    if (type === "range" || type === "number") {
      const n = Number(input.value);
      return Number.isFinite(n) ? n : input.value;
    }
    return input.value;
  }
  if (tag === "select") return (el                     ).value;
  if (tag === "textarea") return (el                       ).value;
  return (el                    ).value;
}

/** Push a state value back into a form control (doc → DOM). */
function writeControlValue(el             , value         )       {
  const tag = el.tagName.toLowerCase();
  if (tag === "input") {
    const input = el                    ;
    if ((input.type || "").toLowerCase() === "checkbox") {
      input.checked = Boolean(value);
      return;
    }
    input.value = value === null || value === undefined ? "" : String(value);
    return;
  }
  if (tag === "select" || tag === "textarea") {
    (el                     ).value =
      value === null || value === undefined ? "" : String(value);
  }
}

                     
              
                                                     
                  
                                                 
 

/** One `state.subscribe` registration. `last` is the JSON of the pointer's
 *  value at the previous fire, so we only notify on real change. */
                    
                  
               
               
 

/** RFC-6901: parse a JSON Pointer into its (unescaped) reference tokens.
 *  "" → whole document ([]); "/a/b" → ["a","b"]; "/" → [""]. */
function parsePointer(pointer        )           {
  if (!pointer) return [];
  const parts = pointer.split("/");
  parts.shift(); // drop the empty segment before the leading "/"
  const out           = [];
  for (const p of parts) out.push(p.replace(/~1/g, "/").replace(/~0/g, "~"));
  return out;
}

/** Resolve a JSON Pointer against `doc`; `undefined` if any step is missing. */
function pointerGet(doc         , pointer        )          {
  const tokens = parsePointer(pointer);
  let cur = doc;
  for (const tok of tokens) {
    if (cur == null) return undefined;
    if (Array.isArray(cur)) {
      const arr = cur            ;
      const idx = tok === "-" ? arr.length : parseInt(tok, 10);
      cur = arr[idx];
    } else if (typeof cur === "object") {
      cur = (cur             )[tok];
    } else {
      return undefined;
    }
  }
  return cur;
}

/** The container that holds a pointer's last token (its parent). */
function pointerParent(root         , tokens          )          {
  let cur = root;
  for (let i = 0; i < tokens.length - 1; i++) {
    if (cur == null) return null;
    if (Array.isArray(cur)) {
      cur = (cur            )[parseInt(tokens[i], 10)];
    } else if (typeof cur === "object") {
      cur = (cur             )[tokens[i]];
    } else {
      return null;
    }
  }
  return cur;
}

function ptrAdd(root         , path        , value         )          {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return value; // replace whole doc
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent            ;
    const idx = key === "-" ? arr.length : parseInt(key, 10);
    if (isFinite(idx)) arr.splice(idx, 0, value);
  } else {
    (parent             )[key] = value;
  }
  return root;
}

function ptrReplace(root         , path        , value         )          {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return value;
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent            ;
    const idx = parseInt(key, 10);
    if (idx >= 0 && idx < arr.length) arr[idx] = value;
  } else {
    (parent             )[key] = value;
  }
  return root;
}

function ptrRemove(root         , path        )          {
  const tokens = parsePointer(path);
  if (tokens.length === 0) return null;
  const parent = pointerParent(root, tokens);
  const key = tokens[tokens.length - 1];
  if (parent == null || typeof parent !== "object") return root;
  if (Array.isArray(parent)) {
    const arr = parent            ;
    const idx = parseInt(key, 10);
    if (idx >= 0 && idx < arr.length) arr.splice(idx, 1);
  } else {
    delete (parent             )[key];
  }
  return root;
}

function deepClone(v         )          {
  if (v == null || typeof v !== "object") return v;
  try {
    return JSON.parse(JSON.stringify(v));
  } catch {
    return v;
  }
}

function jsonEqual(a         , b         )          {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Apply RFC-6902 ops to a *clone* of `doc`, returning the new doc. A failed
 *  `test` (or a throwing op) aborts by propagating, so the caller keeps the
 *  pre-patch doc. */
function applyPatch(doc         , ops         )          {
  let root = deepClone(doc);
  const list = Array.isArray(ops) ? ops : [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const op = raw             ;
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
function patchedPaths(patch         )           {
  const out           = [];
  const list = Array.isArray(patch) ? patch : [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const op = raw             ;
    if (typeof op.path === "string") out.push(op.path);
    if (typeof op.from === "string") out.push(op.from);
  }
  return out;
}

/** Whether a binding pointer is affected by a set of patched paths: it equals,
 *  is a prefix of (ancestor), or is prefixed by (descendant) some path. Root
 *  ("") on either side touches everything. */
function pointerAffected(bindPointer        , paths          )          {
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
function bindText(value         )         {
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
function stateStringify(value         )         {
  const s = JSON.stringify(value);
  return s === undefined ? "\u0000undef" : s;
}

function asRecord(v         )            {
  if (v && typeof v === "object" && !Array.isArray(v)) return v             ;
  return {};
}

// Attributes an author may bind via `data-br-bind-attr`. `style` and any `on*`
// handler are FORBIDDEN (they execute); `aria-*`/`data-*` are allowed too.
const BIND_ATTR_ALLOW                          = {
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
function isSafeBindUrl(u        )          {
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
const KNOWN_PACKS                          = {
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
function sizeToken(v         )         {
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
function uniqueAreaNames(rows          )           {
  const seen            = {};
  const out           = [];
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
const PRESENCE_CMDS                          = {
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
function presenceLabel(s        )         {
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
function presenceTextFor(cmd           , who        )         {
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
function errText(e         )         {
  let s = "error";
  try {
    s = String(e);
  } catch {
    s = "error";
  }
  return s.slice(0, 500);
}

class UiRuntime {
          client                 ;
  /** The agent's shared state bag (mirrors the state doc when it is an object;
   *  `{}` when the doc is an array/primitive). Kept for back-compat with apps
   *  that read `ui.state` directly. */
  state                          = {};
  /** The full shared state document (any JSON value) and its server version. */
          doc          = {};
          version = 0;
          bindings              = [];
          scanned = false;
          stateSubs             = [];
          stateListeners                  = [];
          commandListeners                    = [];
          docks                           = new Map();
          panels                           = new Map();
          toastHost                     = null;
          modalHost                     = null;
          openAsk                = null;
  // ── SDK v2: flat id→instance registry + author component registry ──
          instances                             = new Map();
          iidSeq = 0;
          componentReg                    = new ComponentRegistry();
  // Whether the `node_selected` auto-signal document listener is installed.
          netSignalWired = false;
  // ── presence layer (§3.5): the ambient agent-activity chip ──
          presenceEl                     = null;
          presenceTimer                                       = null;
          presenceMsg                = null;
  // ── ui_suggest (§3.5): one live suggestion row + its Escape handler ──
          suggestRow                     = null;
          suggestHostEl                     = null;
          suggestKey                    = null;

  constructor(client                 ) {
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
  onState(fn               )       {
    this.stateListeners.push(fn);
    return this;
  }

  /** Observe every UI command (for logging, or to override handling). */
  onCommand(fn                 )       {
    this.commandListeners.push(fn);
    return this;
  }

  /** The author-declared regions the agent may render into. */
  regions()           {
    const out           = [];
    const nodes = document.querySelectorAll("[data-br-region]");
    for (let i = 0; i < nodes.length; i++) {
      const name = (nodes[i]               ).dataset.brRegion;
      if (name) out.push(name);
    }
    return out;
  }

  /**
   * Tell the backend what this page offers, so `ui_describe` returns real
   * targets instead of the agent guessing selectors.
   */
  reportSurface()       {
    const ids           = [];
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
          wireNetworkSelectSignal()       {
    if (this.netSignalWired) return;
    this.netSignalWired = true;
    try {
      document.addEventListener("br-network-select", (e) => {
        const ev = e                  ;
        const nodeId = ev.detail ? ev.detail.id : null;
        const tgt = ev.target               ;
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
          notePresence(cmd           )       {
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
  presence(msg        )       {
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
  presenceText()                {
    return this.presenceMsg;
  }

          presenceChip()                     {
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

          fadePresence()       {
    this.presenceMsg = null;
    this.presenceTimer = null;
    if (this.presenceEl) this.presenceEl.classList.remove("br-presence--on");
  }

  // ── ui_suggest (§3.5): non-blocking suggestion chips ───────────────────────

  /** Render up to five dismissible suggestion chips. Clicking a chip sends its
   *  `prompt` via `br.prompt` (or, when it has none, hands `onCommand` a synthetic
   *  `suggest` command carrying the label). Escape or the × dismisses them all. */
          applySuggest(cmd           )       {
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

    const onKey             = (e) => {
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
          fireSuggestCommand(label        )       {
    const synthetic            = { cmd: "suggest", label: label };
    for (const fn of this.commandListeners) {
      try {
        fn(synthetic);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  }

          dismissSuggest()       {
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

          suggestHost()                     {
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
  apply(cmd           )       {
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
  resolveTarget(target        )                     {
    const t = (target || "").trim();
    if (t.indexOf("@region:") === 0) {
      const name = t.slice("@region:".length);
      return document.querySelector             (
        `[data-br-region="${cssEscape(name)}"]`
      );
    }
    if (t.indexOf("@panel:") === 0) {
      const panel = this.panels.get(t.slice("@panel:".length));
      return panel ? panel.querySelector             (".br-panel__body") : null;
    }
    if (t === "@chat") return document.querySelector             ("[data-br-chat]");
    if (t === "@main") return this.mainHost();
    return document.querySelector             (t);
  }

          mainHost()              {
    return (
      document.querySelector             ("[data-br-main]") ||
      document.querySelector             ("main") ||
      document.querySelector             (".br-container") ||
      document.body
    );
  }

          dock(place        )              {
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

          syncDockVisibility()       {
    this.docks.forEach((el, cls) => {
      const bodyCls = "br-has-" + cls.replace("br-dock--", "dock-");
      document.body.classList.toggle(bodyCls, el.childElementCount > 0);
    });
  }

  // ── commands ─────────────────────────────────────────────────────────────

          applyPanel(cmd           )       {
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

          applyRender(cmd           )       {
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

          hasKeyedChildren(host             )          {
    for (let i = 0; i < host.children.length; i++) {
      if (host.children[i].getAttribute("data-br-iid")) return true;
    }
    return false;
  }

  /** A `WidgetContext` bound to `widgetId`, including the component bridge. */
          makeCtx(widgetId        )                {
    const rt = this;
    return {
      fields: new Map(),
      onAction: (action, payload) => rt.client.sendRaw({ type: "widget_action", widgetId: widgetId, action: action, payload: payload }),
      mountComponent: (name, props, cid) => rt.mountComponent(name, props, cid || widgetId),
    };
  }

  /** The stable id for a node: its own `id`, else a generated one (stamped back
   *  onto the node so nested lookups agree within this render). */
          nodeId(node            )         {
    const raw = (node              ).id;
    if (typeof raw === "string" && raw) return raw;
    this.iidSeq++;
    const gen = "auto-" + this.iidSeq;
    (node              ).id = gen;
    return gen;
  }

  /** Render a widget node, catching a throwing renderer: a neutral placeholder
   *  goes in its place and one (rate-limited) `ui_error` is posted, so a bad
   *  node never breaks the render or the socket. Component-mount errors report
   *  themselves inside `mountComponent`, so this stays a single report per node. */
          renderNode(node            , ctx               )              {
    try {
      return renderWidget(node, ctx);
    } catch (e) {
      const kind = String((node                 ).t || "widget");
      this.client.reportUiError("widget:" + kind, errText(e), (node              ).id);
      return unknownWidgetEl(kind);
    }
  }

  /** Render one node standalone (own ctx), tag it, but do NOT register it. */
          buildInstanceEl(node            , id        )              {
    const el = this.renderNode(node, this.makeCtx(id));
    el.setAttribute("data-br-iid", id);
    return el;
  }

  /** Render widget nodes into `host`, wiring buttons back into the agent loop
   *  and registering each top-level node as an addressable instance. A shared
   *  ctx keeps a form's fields collectable across sibling nodes. */
          renderInto(host             , nodes              , widgetId        , append          )       {
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
          morphInto(host             , nodes              , widgetId        )       {
    const existing            = {};
    for (let i = 0; i < host.children.length; i++) {
      const iid = host.children[i].getAttribute("data-br-iid");
      if (iid) existing[iid] = true;
    }
    const ctx = this.makeCtx(widgetId);
    const seen            = {};
    const ordered                = [];
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
      const ch = host.children[i]               ;
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
          morphInstance(id        , newNode            )       {
    const entry = this.instances.get(id);
    if (!entry) return;
    const oldNode = entry.node;
    const oldEl = entry.el;
    if ((newNode                 ).t === "component") {
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

          applyPatchOps(cmd           )       {
    const ops = Array.isArray(cmd.ops) ? cmd.ops : [];
    for (const raw of ops) {
      if (!raw || typeof raw !== "object") continue;
      this.applyOneOp(raw           );
    }
    this.refreshBindings();
  }

          applyOneOp(op         )       {
    const kind = String(op.op || "");
    if (kind === "add") this.opAdd(op);
    else if (kind === "replace") this.opReplace(op);
    else if (kind === "set_props") this.opSetProps(op);
    else if (kind === "remove") this.opRemove(op);
  }

          resolvePatchHost(op         )                     {
    if (op.parent) {
      const p = this.instances.get(op.parent);
      if (p) return p.el;
    }
    if (op.target) return this.resolveTarget(op.target);
    // default target: the main results region, else the app's main host.
    return this.resolveTarget("@region:results") || this.mainHost();
  }

          opAdd(op         )       {
    const node = op.node;
    if (!node) return;
    const id = typeof op.id === "string" && op.id ? op.id : this.nodeId(node);
    (node              ).id = id;
    const host = this.resolvePatchHost(op);
    if (!host) return;
    const el = this.buildInstanceEl(node, id);
    this.instances.set(id, { node: node, el: el });
    const index = typeof op.index === "number" ? op.index : -1;
    const ref = index >= 0 && index < host.children.length ? host.children[index] : null;
    if (ref) host.insertBefore(el, ref);
    else host.appendChild(el);
  }

          opReplace(op         )       {
    const id = op.id || "";
    if (!id || !op.node || !this.instances.has(id)) return;
    (op.node              ).id = id;
    this.morphInstance(id, op.node);
  }

          opSetProps(op         )       {
    const id = op.id || "";
    const entry = this.instances.get(id);
    if (!entry) return;
    const props = op.props && typeof op.props === "object" ? op.props : {};
    const node = entry.node                 ;
    // Log append fast-path: preserves scroll, no full re-render.
    if (node.t === "log" && Array.isArray(props.append)) {
      appendLogLines(entry.el, entry.node           , props.append);
      if (typeof props.max === "number") (entry.node           ).max = props.max;
      return;
    }
    // Components: merge into the component's own props (not the node header).
    if (node.t === "component") {
      const cn = entry.node                 ;
      const cur = cn.props && typeof cn.props === "object" ? cn.props : {};
      const mergedProps = Object.assign({}, cur, props);
      const mergedNode = Object.assign({}, entry.node);
      (mergedNode                 ).props = mergedProps;
      this.morphInstance(id, mergedNode              );
      return;
    }
    const merged = shallowMergeNode(entry.node, props);
    this.morphInstance(id, merged);
  }

          opRemove(op         )       {
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

          disposeInstance(id        )       {
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
          disposeInstancesIn(host             )       {
    const found = host.querySelectorAll("[data-br-iid]");
    for (let i = 0; i < found.length; i++) {
      const iid = found[i].getAttribute("data-br-iid");
      if (iid) this.disposeInstance(iid);
    }
  }

  // ── author component registry (`br.components`) ────────────────────────────

  registerComponent(name        , def              )       {
    this.componentReg.register(name, def);
  }

          componentCtx(id        )                   {
    const client = this.client;
    return {
      id: id,
      state: client.state,
      run: (t, tgt, o) => client.run(t, tgt, o),
    };
  }

  /** Mount a registered component into a fresh container; `null` when the name
   *  is unregistered (renderWidget then shows the neutral placeholder). */
  mountComponent(name        , props         , id        )                     {
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

          updateComponent(id        , oldNode            , newNode            , el             )       {
    const nn = newNode                 ;
    const on = oldNode                 ;
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
  network(id        )                           {
    const entry = this.instances.get(id);
    return entry ? readNet(entry.el) : null;
  }

  /** Programmatically select a node in a network instance. */
  selectNetworkNode(id        , nodeId               )       {
    const net = this.network(id);
    if (net) net.select(nodeId);
  }

  /** Register any addressable containers rendered outside the instance path
   *  (```chart / ```graph fences carry a `data-br-iid`) so `ui_patch replace`
   *  can target them. Kind is inferred from the container class. */
          indexDomInstances()       {
    const found = document.querySelectorAll("[data-br-iid]");
    for (let i = 0; i < found.length; i++) {
      const el = found[i]               ;
      const iid = el.getAttribute("data-br-iid") || "";
      if (!iid || this.instances.has(iid)) continue;
      const cls = typeof el.className === "string" ? el.className : "";
      let kind = "widget";
      if (cls.indexOf("br-chart") >= 0 || cls.indexOf("br-plot") >= 0) kind = "plot";
      else if (cls.indexOf("br-visual") >= 0 || cls.indexOf("br-graph") >= 0) kind = "network-lite";
      this.instances.set(iid, { node: { t: kind }              , el: el });
    }
  }

          applyHighlight(cmd           )       {
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

          focusBackdrop()              {
    let bd = document.querySelector             (".br-focus-backdrop");
    if (!bd) {
      bd = document.createElement("div");
      bd.className = "br-focus-backdrop";
      document.body.appendChild(bd);
    }
    return bd;
  }

          clearHighlights()       {
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

          applyTheme(cmd           )       {
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
          applyPack(pack        , root             )       {
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

          applyLayout(cmd           )       {
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
    const presetWidth                         = {
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
          applyGridLayout(cmd           )       {
    const rows           = [];
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
    const cols           = [];
    for (let i = 0; i < colCount; i++) cols.push(sizeToken(sizes[i]));
    host.style.setProperty("grid-template-columns", cols.join(" "));

    const warned            = {};
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
          findAreaElement(name        )                     {
    const byRegion = document.querySelector             ('[data-br-region="' + cssEscape(name) + '"]');
    if (byRegion) return byRegion;
    return document.getElementById(name);
  }

          toasts()              {
    if (this.toastHost && this.toastHost.isConnected) return this.toastHost;
    const el = document.createElement("div");
    el.className = "br-toasts";
    document.body.appendChild(el);
    this.toastHost = el;
    return el;
  }

          applyNotify(cmd           )       {
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
          applyState(cmd           )       {
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
          afterStateChange(affected                 )       {
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
          scanBindings()       {
    this.scanned = true;
    const list              = [];
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
      this.wireModelListener(el               , p);
    }

    this.bindings = list;
  }

  /**
   * Attach the write-back listeners for one `data-br-model` control. Idempotent:
   * `scanBindings` re-runs after every render, and a control that survives a
   * re-render must not accumulate duplicate listeners.
   */
          wireModelListener(el             , pointer        )       {
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
          refreshBindings()       {
    this.scanBindings();
    this.evalAllBindings();
    this.indexDomInstances();
  }

          evalAllBindings()       {
    for (const b of this.bindings) this.applyBinding(b);
  }

  /** Push one binding's current pointer value into its element. Non-executing
   *  sinks only: `textContent`, an allowlisted attribute, or the `hidden`
   *  property. Never `innerHTML`. */
          applyBinding(b           )       {
    const value = pointerGet(this.doc, b.pointer);
    if (b.kind === "text") {
      b.el.textContent = bindText(value);
    } else if (b.kind === "show") {
      const he = b.el               ;
      he.hidden = !value;
    } else if (b.kind === "attr") {
      this.applyBindAttr(b.el, b.attr, value);
    } else if (b.kind === "model") {
      // Do not clobber the control the user is actively editing.
      if (document.activeElement === b.el) return;
      writeControlValue(b.el               , value);
    }
  }

          applyBindAttr(el         , attr        , value         )       {
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
        const field = el                    ;
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
  stateGet(path         )          {
    if (path === undefined || path === null || path === "") return deepClone(this.doc);
    return deepClone(pointerGet(this.doc, path));
  }

  /** Optimistically set the value at `path`, then send a `state_write` carrying
   *  the pre-write `baseVersion`. */
  stateSet(path        , value         )       {
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
  stateRemove(path        )       {
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
  stateUpdate(fn            )       {
    const baseVersion = this.version;
    const draft = deepClone(this.doc);
    let next         ;
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
  stateSubscribe(path        , fn          )        {
    const sub           = {
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

          modal()              {
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
          applyAsk(cmd           )       {
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

    const getters               = new Map();
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
      const payload                                   = {};
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

    const onKey = (e               ) => {
      if (e.key === "Escape") this.closeAsk(requestId, true);
    };
    document.addEventListener("keydown", onKey);
    backdrop.addEventListener("br:teardown", () =>
      document.removeEventListener("keydown", onKey)
    );

    const first = card.querySelector             ("input, select, textarea");
    if (first) first.focus();
  }

  /** Remove the form. When `answer` is true, tell the parked tool it was skipped. */
          closeAsk(requestId        , answer         )       {
    if (answer) {
      this.client.sendRaw({
        type: "ui_reply",
        requestId,
        payload: { cancelled: true },
      });
    }
    this.dismissAsk(requestId);
  }

          dismissAsk(requestId        )       {
    const el = document.querySelector(`[data-br-ask="${cssEscape(requestId)}"]`);
    if (el) {
      el.dispatchEvent(new CustomEvent("br:teardown"));
      el.remove();
    }
    if (this.openAsk === requestId) this.openAsk = null;
  }
}

/** Minimal `CSS.escape` shim for the attribute selectors we build. */
function cssEscape(value        )         {
  if (window.CSS && typeof window.CSS.escape === "function") {
    return window.CSS.escape(value);
  }
  return String(value).replace(/["\\\]]/g, "\\$&");
}

function buildAskField(f              , getters              )              {
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

function mountChat(client                 , host             )       {
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

  const add = (cls        , html         , content        )              => {
    const el = document.createElement("div");
    el.className = "br-msg br-msg--" + cls;
    if (html) el.innerHTML = content;
    else el.textContent = content;
    log.appendChild(el);
    log.scrollTop = log.scrollHeight;
    return el;
  };
  if (client.config.greeting) add("agent", false, client.config.greeting);

  let current                     = null;
  let buffer = "";
  let typing                     = null;
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
      add("agent", false, (e         ).message || "Failed to get a response.");
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
function readAppConfig()                     {
  try {
    const el = document.getElementById("biorouter-app-config");
    const text = el ? el.textContent : "";
    if (text && text.trim()) {
      try {
        const parsed = JSON.parse(text);
        if (parsed && typeof parsed === "object") return parsed                      ;
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
function createApp(overrides                     = {})                  {
  const cfg            = {
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
      const host = document.querySelector             ("[data-br-chat]");
      if (host && !host.dataset.brMounted) {
        host.dataset.brMounted = "1";
        mountChat(client, host);
      }
    }
    client.connect().catch((e       ) => mountBackendError(e.message));
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
function showDegradedBanner(missingProfiles          )       {
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
function mountBackendError(message        )       {
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
function clearBackendError()       {
  const bar = document.querySelector("[data-br-backend-error]");
  if (bar) bar.remove();
}

return { BioRouterClient, graphToNetwork, renderChart, renderGraph, renderMarkdown, fileToImageInput, mountTimeline, renderWidget, UiRuntime, mountChat, createApp, showDegradedBanner, mountBackendError, clearBackendError };
})();

// ---- src/preventData.ts
__mod.preventData = (function () {

const PREVENT_COEFFICIENTS = {"model":"PREVENT base model, 10-year risk (total CVD, ASCVD, heart failure), women and men","source":"Khan SS et al. Circulation 2024;149:430-449, doi:10.1161/CIRCULATIONAHA.123.067626, Supplemental Table S12.A","verification":"Transcribed from the published supplement and verified against the R package preventr 0.12.0 in the case study (risks identical after rounding to 0.001).","risk":"risk = 1 / (1 + exp(-sum(coefficient * term))), where sum includes the Constant term","inputs":{"age":"years, 30-79","sex":"female | male","sbp":"mmHg, 90-180","total_cholesterol_mg_dl":"130-320","hdl_mg_dl":"20-100","bmi":"kg/m2, 18.5-39.9","egfr":"mL/min/1.73 m2, 15-140, CKD-EPI 2021","diabetes":"0/1","current_smoker":"0/1","bp_treatment":"0/1","statin":"0/1"},"terms":{"Age, per 10 years":"(age - 55) / 10","non-HDL-C per 1 mmol/L":"(total_cholesterol_mg_dl - hdl_mg_dl) * 0.02586 - 3.5","HDL-C per 0.3 mmol/L":"(hdl_mg_dl * 0.02586 - 1.3) / 0.3","SBP <110 per 20 mmHg":"(min(sbp, 110) - 110) / 20","SBP ≥110 per 20 mmHg":"(max(sbp, 110) - 130) / 20","Diabetes":"diabetes (0/1)","Current smoking":"current_smoker (0/1)","BMI <30, per 5 kg/m2":"(min(bmi, 30) - 25) / 5","BMI 30+, per 5 kg/m2":"(max(bmi, 30) - 30) / 5","eGFR <60, per -15 ml":"(min(egfr, 60) - 60) / -15","eGFR 60+, per -15 ml":"(max(egfr, 60) - 90) / -15","Anti-hypertensive use":"bp_treatment (0/1)","Statin use":"statin (0/1)","Treated SBP ≥110 mm Hg per 20 mm Hg":"bp_treatment * SBP≥110 term","Treated non-HDL-C":"statin * non-HDL-C term","Age per 10yr * non-HDL-C per 1 mmol/L":"age term * non-HDL-C term","Age per 10yr * HDL-C per 1 mml/L":"age term * HDL-C term","Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":"age term * SBP≥110 term","Age per 10yr * diabetes":"age term * diabetes","Age per 10yr * current smoking":"age term * current_smoker","Age per 10yr * BMI 30+ per 5 kg/m2":"age term * BMI 30+ term","Age per 10yr * eGFR <60, per -15 ml":"age term * eGFR<60 term","Constant":"1"},"coefficients":{"ascvd":{"female":{"Age, per 10 years":0.719883,"non-HDL-C per 1 mmol/L":0.1176967,"HDL-C per 0.3 mmol/L":-0.151185,"SBP <110 per 20 mmHg":-0.0835358,"SBP ≥110 per 20 mmHg":0.3592852,"Diabetes":0.8348585,"Current smoking":0.4831078,"eGFR <60, per -15 ml":0.4864619,"eGFR 60+, per -15 ml":0.0397779,"Anti-hypertensive use":0.2265309,"Statin use":-0.0592374,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0395762,"Treated non-HDL-C":0.0844423,"Age per 10yr * non-HDL-C per 1 mmol/L":-0.0567839,"Age per 10yr * HDL-C per 1 mml/L":0.0325692,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.1035985,"Age per 10yr * diabetes":-0.2417542,"Age per 10yr * current smoking":-0.0791142,"Age per 10yr * eGFR <60, per -15 ml":-0.1671492,"Constant":-3.819975},"male":{"Age, per 10 years":0.7099847,"non-HDL-C per 1 mmol/L":0.1658663,"HDL-C per 0.3 mmol/L":-0.1144285,"SBP <110 per 20 mmHg":-0.2837212,"SBP ≥110 per 20 mmHg":0.3239977,"Diabetes":0.7189597,"Current smoking":0.3956973,"eGFR <60, per -15 ml":0.3690075,"eGFR 60+, per -15 ml":0.0203619,"Anti-hypertensive use":0.2036522,"Statin use":-0.0865581,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0322916,"Treated non-HDL-C":0.114563,"Age per 10yr * non-HDL-C per 1 mmol/L":-0.0300005,"Age per 10yr * HDL-C per 1 mml/L":0.0232747,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.0927024,"Age per 10yr * diabetes":-0.2018525,"Age per 10yr * current smoking":-0.0970527,"Age per 10yr * eGFR <60, per -15 ml":-0.1217081,"Constant":-3.500655}},"hf":{"female":{"Age, per 10 years":0.8998235,"SBP <110 per 20 mmHg":-0.4559771,"SBP ≥110 per 20 mmHg":0.3576505,"Diabetes":1.038346,"Current smoking":0.583916,"BMI <30, per 5 kg/m2":-0.0072294,"BMI 30+, per 5 kg/m2":0.2997706,"eGFR <60, per -15 ml":0.7451638,"eGFR 60+, per -15 ml":0.0557087,"Anti-hypertensive use":0.3534442,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0981511,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.0946663,"Age per 10yr * diabetes":-0.3581041,"Age per 10yr * current smoking":-0.1159453,"Age per 10yr * BMI 30+ per 5 kg/m2":-0.003878,"Age per 10yr * eGFR <60, per -15 ml":-0.1884289,"Constant":-4.310409},"male":{"Age, per 10 years":0.8972642,"SBP <110 per 20 mmHg":-0.6811466,"SBP ≥110 per 20 mmHg":0.3634461,"Diabetes":0.923776,"Current smoking":0.5023736,"BMI <30, per 5 kg/m2":-0.0485841,"BMI 30+, per 5 kg/m2":0.3726929,"eGFR <60, per -15 ml":0.6926917,"eGFR 60+, per -15 ml":0.0251827,"Anti-hypertensive use":0.2980922,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0497731,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.1289201,"Age per 10yr * diabetes":-0.3040924,"Age per 10yr * current smoking":-0.1401688,"Age per 10yr * BMI 30+ per 5 kg/m2":0.0068126,"Age per 10yr * eGFR <60, per -15 ml":-0.1797778,"Constant":-3.946391}},"total_cvd":{"female":{"Age, per 10 years":0.7939329,"non-HDL-C per 1 mmol/L":0.0305239,"HDL-C per 0.3 mmol/L":-0.1606857,"SBP <110 per 20 mmHg":-0.2394003,"SBP ≥110 per 20 mmHg":0.3600781,"Diabetes":0.8667604,"Current smoking":0.5360739,"eGFR <60, per -15 ml":0.6045917,"eGFR 60+, per -15 ml":0.0433769,"Anti-hypertensive use":0.3151672,"Statin use":-0.1477655,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0663612,"Treated non-HDL-C":0.1197879,"Age per 10yr * non-HDL-C per 1 mmol/L":-0.0819715,"Age per 10yr * HDL-C per 1 mml/L":0.0306769,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.0946348,"Age per 10yr * diabetes":-0.27057,"Age per 10yr * current smoking":-0.078715,"Age per 10yr * eGFR <60, per -15 ml":-0.1637806,"Constant":-3.307728},"male":{"Age, per 10 years":0.7688528,"non-HDL-C per 1 mmol/L":0.0736174,"HDL-C per 0.3 mmol/L":-0.0954431,"SBP <110 per 20 mmHg":-0.4347345,"SBP ≥110 per 20 mmHg":0.3362658,"Diabetes":0.7692857,"Current smoking":0.4386871,"eGFR <60, per -15 ml":0.5378979,"eGFR 60+, per -15 ml":0.0164827,"Anti-hypertensive use":0.288879,"Statin use":-0.1337349,"Treated SBP ≥110 mm Hg per 20 mm Hg":-0.0475924,"Treated non-HDL-C":0.150273,"Age per 10yr * non-HDL-C per 1 mmol/L":-0.0517874,"Age per 10yr * HDL-C per 1 mml/L":0.0191169,"Age per 10yr * SBP ≥110 mm Hg per 20 mmHg":-0.1049477,"Age per 10yr * diabetes":-0.2251948,"Age per 10yr * current smoking":-0.0895067,"Age per 10yr * eGFR <60, per -15 ml":-0.1543702,"Constant":-3.031168}}}}         ;

const REFERENCE_COHORT = {"name":"MOCK reference cohort (synthetic, not patient data)","description":"MOCK data for privacy purposes: 20,000 simulated adults aged 30-79 with independently drawn, plausible PREVENT inputs (fixed random seed). It is NOT the UCSF cohort of the case study and has no real outcomes. Replace it with an aggregated local reference cohort to use the app.","mock":true,"n":{"overall":20000,"female":9994,"male":10006},"observed_vs_predicted":"MOCK: no outcomes are available for the simulated cohort.","bins":"10-year PREVENT risk (fraction); bins 0.5 percentage points wide, merged so that each holds at least 11 simulated persons in all three strata (common bin edges); last bin open-ended","percentile":"cumulative count below the patient's risk, with linear interpolation inside the patient's bin, divided by n","distribution":{"total_cvd":{"overall":[{"lo":0,"hi":0.005,"n":870},{"lo":0.005,"hi":0.01,"n":1853},{"lo":0.01,"hi":0.015,"n":1505},{"lo":0.015,"hi":0.02,"n":1156},{"lo":0.02,"hi":0.025,"n":1015},{"lo":0.025,"hi":0.03,"n":790},{"lo":0.03,"hi":0.035,"n":760},{"lo":0.035,"hi":0.04,"n":602},{"lo":0.04,"hi":0.045,"n":644},{"lo":0.045,"hi":0.05,"n":565},{"lo":0.05,"hi":0.055,"n":477},{"lo":0.055,"hi":0.06,"n":435},{"lo":0.06,"hi":0.065,"n":431},{"lo":0.065,"hi":0.07,"n":433},{"lo":0.07,"hi":0.075,"n":396},{"lo":0.075,"hi":0.08,"n":351},{"lo":0.08,"hi":0.085,"n":346},{"lo":0.085,"hi":0.09,"n":295},{"lo":0.09,"hi":0.095,"n":320},{"lo":0.095,"hi":0.1,"n":289},{"lo":0.1,"hi":0.105,"n":308},{"lo":0.105,"hi":0.11,"n":280},{"lo":0.11,"hi":0.115,"n":278},{"lo":0.115,"hi":0.12,"n":257},{"lo":0.12,"hi":0.125,"n":223},{"lo":0.125,"hi":0.13,"n":248},{"lo":0.13,"hi":0.135,"n":251},{"lo":0.135,"hi":0.14,"n":224},{"lo":0.14,"hi":0.145,"n":230},{"lo":0.145,"hi":0.15,"n":210},{"lo":0.15,"hi":0.155,"n":211},{"lo":0.155,"hi":0.16,"n":219},{"lo":0.16,"hi":0.165,"n":199},{"lo":0.165,"hi":0.17,"n":188},{"lo":0.17,"hi":0.175,"n":198},{"lo":0.175,"hi":0.18,"n":195},{"lo":0.18,"hi":0.185,"n":186},{"lo":0.185,"hi":0.19,"n":159},{"lo":0.19,"hi":0.195,"n":197},{"lo":0.195,"hi":0.2,"n":163},{"lo":0.2,"hi":0.205,"n":149},{"lo":0.205,"hi":0.21,"n":134},{"lo":0.21,"hi":0.215,"n":140},{"lo":0.215,"hi":0.22,"n":155},{"lo":0.22,"hi":0.225,"n":143},{"lo":0.225,"hi":0.23,"n":113},{"lo":0.23,"hi":0.235,"n":111},{"lo":0.235,"hi":0.24,"n":101},{"lo":0.24,"hi":0.245,"n":108},{"lo":0.245,"hi":0.25,"n":91},{"lo":0.25,"hi":0.255,"n":87},{"lo":0.255,"hi":0.26,"n":74},{"lo":0.26,"hi":0.265,"n":63},{"lo":0.265,"hi":0.27,"n":68},{"lo":0.27,"hi":0.275,"n":68},{"lo":0.275,"hi":0.28,"n":63},{"lo":0.28,"hi":0.285,"n":52},{"lo":0.285,"hi":0.295,"n":69},{"lo":0.295,"hi":0.305,"n":66},{"lo":0.305,"hi":0.315,"n":49},{"lo":0.315,"hi":0.34,"n":77},{"lo":0.34,"hi":null,"n":62}],"female":[{"lo":0,"hi":0.005,"n":747},{"lo":0.005,"hi":0.01,"n":1099},{"lo":0.01,"hi":0.015,"n":810},{"lo":0.015,"hi":0.02,"n":585},{"lo":0.02,"hi":0.025,"n":515},{"lo":0.025,"hi":0.03,"n":403},{"lo":0.03,"hi":0.035,"n":381},{"lo":0.035,"hi":0.04,"n":286},{"lo":0.04,"hi":0.045,"n":297},{"lo":0.045,"hi":0.05,"n":276},{"lo":0.05,"hi":0.055,"n":243},{"lo":0.055,"hi":0.06,"n":227},{"lo":0.06,"hi":0.065,"n":205},{"lo":0.065,"hi":0.07,"n":203},{"lo":0.07,"hi":0.075,"n":208},{"lo":0.075,"hi":0.08,"n":176},{"lo":0.08,"hi":0.085,"n":179},{"lo":0.085,"hi":0.09,"n":147},{"lo":0.09,"hi":0.095,"n":157},{"lo":0.095,"hi":0.1,"n":129},{"lo":0.1,"hi":0.105,"n":142},{"lo":0.105,"hi":0.11,"n":124},{"lo":0.11,"hi":0.115,"n":130},{"lo":0.115,"hi":0.12,"n":131},{"lo":0.12,"hi":0.125,"n":105},{"lo":0.125,"hi":0.13,"n":124},{"lo":0.13,"hi":0.135,"n":123},{"lo":0.135,"hi":0.14,"n":111},{"lo":0.14,"hi":0.145,"n":113},{"lo":0.145,"hi":0.15,"n":93},{"lo":0.15,"hi":0.155,"n":103},{"lo":0.155,"hi":0.16,"n":108},{"lo":0.16,"hi":0.165,"n":100},{"lo":0.165,"hi":0.17,"n":79},{"lo":0.17,"hi":0.175,"n":88},{"lo":0.175,"hi":0.18,"n":87},{"lo":0.18,"hi":0.185,"n":66},{"lo":0.185,"hi":0.19,"n":71},{"lo":0.19,"hi":0.195,"n":80},{"lo":0.195,"hi":0.2,"n":75},{"lo":0.2,"hi":0.205,"n":52},{"lo":0.205,"hi":0.21,"n":47},{"lo":0.21,"hi":0.215,"n":52},{"lo":0.215,"hi":0.22,"n":55},{"lo":0.22,"hi":0.225,"n":54},{"lo":0.225,"hi":0.23,"n":44},{"lo":0.23,"hi":0.235,"n":41},{"lo":0.235,"hi":0.24,"n":32},{"lo":0.24,"hi":0.245,"n":37},{"lo":0.245,"hi":0.25,"n":22},{"lo":0.25,"hi":0.255,"n":24},{"lo":0.255,"hi":0.26,"n":22},{"lo":0.26,"hi":0.265,"n":19},{"lo":0.265,"hi":0.27,"n":27},{"lo":0.27,"hi":0.275,"n":24},{"lo":0.275,"hi":0.28,"n":20},{"lo":0.28,"hi":0.285,"n":16},{"lo":0.285,"hi":0.295,"n":20},{"lo":0.295,"hi":0.305,"n":17},{"lo":0.305,"hi":0.315,"n":19},{"lo":0.315,"hi":0.34,"n":12},{"lo":0.34,"hi":null,"n":12}],"male":[{"lo":0,"hi":0.005,"n":123},{"lo":0.005,"hi":0.01,"n":754},{"lo":0.01,"hi":0.015,"n":695},{"lo":0.015,"hi":0.02,"n":571},{"lo":0.02,"hi":0.025,"n":500},{"lo":0.025,"hi":0.03,"n":387},{"lo":0.03,"hi":0.035,"n":379},{"lo":0.035,"hi":0.04,"n":316},{"lo":0.04,"hi":0.045,"n":347},{"lo":0.045,"hi":0.05,"n":289},{"lo":0.05,"hi":0.055,"n":234},{"lo":0.055,"hi":0.06,"n":208},{"lo":0.06,"hi":0.065,"n":226},{"lo":0.065,"hi":0.07,"n":230},{"lo":0.07,"hi":0.075,"n":188},{"lo":0.075,"hi":0.08,"n":175},{"lo":0.08,"hi":0.085,"n":167},{"lo":0.085,"hi":0.09,"n":148},{"lo":0.09,"hi":0.095,"n":163},{"lo":0.095,"hi":0.1,"n":160},{"lo":0.1,"hi":0.105,"n":166},{"lo":0.105,"hi":0.11,"n":156},{"lo":0.11,"hi":0.115,"n":148},{"lo":0.115,"hi":0.12,"n":126},{"lo":0.12,"hi":0.125,"n":118},{"lo":0.125,"hi":0.13,"n":124},{"lo":0.13,"hi":0.135,"n":128},{"lo":0.135,"hi":0.14,"n":113},{"lo":0.14,"hi":0.145,"n":117},{"lo":0.145,"hi":0.15,"n":117},{"lo":0.15,"hi":0.155,"n":108},{"lo":0.155,"hi":0.16,"n":111},{"lo":0.16,"hi":0.165,"n":99},{"lo":0.165,"hi":0.17,"n":109},{"lo":0.17,"hi":0.175,"n":110},{"lo":0.175,"hi":0.18,"n":108},{"lo":0.18,"hi":0.185,"n":120},{"lo":0.185,"hi":0.19,"n":88},{"lo":0.19,"hi":0.195,"n":117},{"lo":0.195,"hi":0.2,"n":88},{"lo":0.2,"hi":0.205,"n":97},{"lo":0.205,"hi":0.21,"n":87},{"lo":0.21,"hi":0.215,"n":88},{"lo":0.215,"hi":0.22,"n":100},{"lo":0.22,"hi":0.225,"n":89},{"lo":0.225,"hi":0.23,"n":69},{"lo":0.23,"hi":0.235,"n":70},{"lo":0.235,"hi":0.24,"n":69},{"lo":0.24,"hi":0.245,"n":71},{"lo":0.245,"hi":0.25,"n":69},{"lo":0.25,"hi":0.255,"n":63},{"lo":0.255,"hi":0.26,"n":52},{"lo":0.26,"hi":0.265,"n":44},{"lo":0.265,"hi":0.27,"n":41},{"lo":0.27,"hi":0.275,"n":44},{"lo":0.275,"hi":0.28,"n":43},{"lo":0.28,"hi":0.285,"n":36},{"lo":0.285,"hi":0.295,"n":49},{"lo":0.295,"hi":0.305,"n":49},{"lo":0.305,"hi":0.315,"n":30},{"lo":0.315,"hi":0.34,"n":65},{"lo":0.34,"hi":null,"n":50}]},"ascvd":{"overall":[{"lo":0,"hi":0.005,"n":1787},{"lo":0.005,"hi":0.01,"n":2436},{"lo":0.01,"hi":0.015,"n":1832},{"lo":0.015,"hi":0.02,"n":1366},{"lo":0.02,"hi":0.025,"n":1109},{"lo":0.025,"hi":0.03,"n":1017},{"lo":0.03,"hi":0.035,"n":868},{"lo":0.035,"hi":0.04,"n":731},{"lo":0.04,"hi":0.045,"n":685},{"lo":0.045,"hi":0.05,"n":616},{"lo":0.05,"hi":0.055,"n":578},{"lo":0.055,"hi":0.06,"n":529},{"lo":0.06,"hi":0.065,"n":479},{"lo":0.065,"hi":0.07,"n":449},{"lo":0.07,"hi":0.075,"n":439},{"lo":0.075,"hi":0.08,"n":391},{"lo":0.08,"hi":0.085,"n":439},{"lo":0.085,"hi":0.09,"n":394},{"lo":0.09,"hi":0.095,"n":321},{"lo":0.095,"hi":0.1,"n":327},{"lo":0.1,"hi":0.105,"n":321},{"lo":0.105,"hi":0.11,"n":298},{"lo":0.11,"hi":0.115,"n":297},{"lo":0.115,"hi":0.12,"n":231},{"lo":0.12,"hi":0.125,"n":264},{"lo":0.125,"hi":0.13,"n":232},{"lo":0.13,"hi":0.135,"n":207},{"lo":0.135,"hi":0.14,"n":195},{"lo":0.14,"hi":0.145,"n":160},{"lo":0.145,"hi":0.15,"n":143},{"lo":0.15,"hi":0.155,"n":137},{"lo":0.155,"hi":0.16,"n":126},{"lo":0.16,"hi":0.165,"n":103},{"lo":0.165,"hi":0.17,"n":69},{"lo":0.17,"hi":0.18,"n":138},{"lo":0.18,"hi":null,"n":286}],"female":[{"lo":0,"hi":0.005,"n":1370},{"lo":0.005,"hi":0.01,"n":1353},{"lo":0.01,"hi":0.015,"n":963},{"lo":0.015,"hi":0.02,"n":691},{"lo":0.02,"hi":0.025,"n":541},{"lo":0.025,"hi":0.03,"n":491},{"lo":0.03,"hi":0.035,"n":433},{"lo":0.035,"hi":0.04,"n":398},{"lo":0.04,"hi":0.045,"n":322},{"lo":0.045,"hi":0.05,"n":308},{"lo":0.05,"hi":0.055,"n":272},{"lo":0.055,"hi":0.06,"n":256},{"lo":0.06,"hi":0.065,"n":234},{"lo":0.065,"hi":0.07,"n":222},{"lo":0.07,"hi":0.075,"n":220},{"lo":0.075,"hi":0.08,"n":204},{"lo":0.08,"hi":0.085,"n":221},{"lo":0.085,"hi":0.09,"n":176},{"lo":0.09,"hi":0.095,"n":160},{"lo":0.095,"hi":0.1,"n":159},{"lo":0.1,"hi":0.105,"n":141},{"lo":0.105,"hi":0.11,"n":132},{"lo":0.11,"hi":0.115,"n":137},{"lo":0.115,"hi":0.12,"n":75},{"lo":0.12,"hi":0.125,"n":111},{"lo":0.125,"hi":0.13,"n":82},{"lo":0.13,"hi":0.135,"n":62},{"lo":0.135,"hi":0.14,"n":68},{"lo":0.14,"hi":0.145,"n":36},{"lo":0.145,"hi":0.15,"n":37},{"lo":0.15,"hi":0.155,"n":29},{"lo":0.155,"hi":0.16,"n":31},{"lo":0.16,"hi":0.165,"n":17},{"lo":0.165,"hi":0.17,"n":12},{"lo":0.17,"hi":0.18,"n":18},{"lo":0.18,"hi":null,"n":12}],"male":[{"lo":0,"hi":0.005,"n":417},{"lo":0.005,"hi":0.01,"n":1083},{"lo":0.01,"hi":0.015,"n":869},{"lo":0.015,"hi":0.02,"n":675},{"lo":0.02,"hi":0.025,"n":568},{"lo":0.025,"hi":0.03,"n":526},{"lo":0.03,"hi":0.035,"n":435},{"lo":0.035,"hi":0.04,"n":333},{"lo":0.04,"hi":0.045,"n":363},{"lo":0.045,"hi":0.05,"n":308},{"lo":0.05,"hi":0.055,"n":306},{"lo":0.055,"hi":0.06,"n":273},{"lo":0.06,"hi":0.065,"n":245},{"lo":0.065,"hi":0.07,"n":227},{"lo":0.07,"hi":0.075,"n":219},{"lo":0.075,"hi":0.08,"n":187},{"lo":0.08,"hi":0.085,"n":218},{"lo":0.085,"hi":0.09,"n":218},{"lo":0.09,"hi":0.095,"n":161},{"lo":0.095,"hi":0.1,"n":168},{"lo":0.1,"hi":0.105,"n":180},{"lo":0.105,"hi":0.11,"n":166},{"lo":0.11,"hi":0.115,"n":160},{"lo":0.115,"hi":0.12,"n":156},{"lo":0.12,"hi":0.125,"n":153},{"lo":0.125,"hi":0.13,"n":150},{"lo":0.13,"hi":0.135,"n":145},{"lo":0.135,"hi":0.14,"n":127},{"lo":0.14,"hi":0.145,"n":124},{"lo":0.145,"hi":0.15,"n":106},{"lo":0.15,"hi":0.155,"n":108},{"lo":0.155,"hi":0.16,"n":95},{"lo":0.16,"hi":0.165,"n":86},{"lo":0.165,"hi":0.17,"n":57},{"lo":0.17,"hi":0.18,"n":120},{"lo":0.18,"hi":null,"n":274}]},"hf":{"overall":[{"lo":0,"hi":0.005,"n":3759},{"lo":0.005,"hi":0.01,"n":2508},{"lo":0.01,"hi":0.015,"n":1628},{"lo":0.015,"hi":0.02,"n":1198},{"lo":0.02,"hi":0.025,"n":930},{"lo":0.025,"hi":0.03,"n":824},{"lo":0.03,"hi":0.035,"n":708},{"lo":0.035,"hi":0.04,"n":619},{"lo":0.04,"hi":0.045,"n":582},{"lo":0.045,"hi":0.05,"n":491},{"lo":0.05,"hi":0.055,"n":464},{"lo":0.055,"hi":0.06,"n":428},{"lo":0.06,"hi":0.065,"n":407},{"lo":0.065,"hi":0.07,"n":356},{"lo":0.07,"hi":0.075,"n":362},{"lo":0.075,"hi":0.08,"n":332},{"lo":0.08,"hi":0.085,"n":338},{"lo":0.085,"hi":0.09,"n":291},{"lo":0.09,"hi":0.095,"n":306},{"lo":0.095,"hi":0.1,"n":292},{"lo":0.1,"hi":0.105,"n":286},{"lo":0.105,"hi":0.11,"n":248},{"lo":0.11,"hi":0.115,"n":226},{"lo":0.115,"hi":0.12,"n":229},{"lo":0.12,"hi":0.125,"n":190},{"lo":0.125,"hi":0.13,"n":199},{"lo":0.13,"hi":0.135,"n":202},{"lo":0.135,"hi":0.14,"n":186},{"lo":0.14,"hi":0.145,"n":147},{"lo":0.145,"hi":0.15,"n":151},{"lo":0.15,"hi":0.155,"n":154},{"lo":0.155,"hi":0.16,"n":117},{"lo":0.16,"hi":0.165,"n":109},{"lo":0.165,"hi":0.17,"n":103},{"lo":0.17,"hi":0.175,"n":88},{"lo":0.175,"hi":0.18,"n":72},{"lo":0.18,"hi":0.185,"n":68},{"lo":0.185,"hi":0.19,"n":61},{"lo":0.19,"hi":0.195,"n":54},{"lo":0.195,"hi":0.2,"n":33},{"lo":0.2,"hi":0.21,"n":64},{"lo":0.21,"hi":0.22,"n":53},{"lo":0.22,"hi":null,"n":137}],"female":[{"lo":0,"hi":0.005,"n":2187},{"lo":0.005,"hi":0.01,"n":1297},{"lo":0.01,"hi":0.015,"n":831},{"lo":0.015,"hi":0.02,"n":601},{"lo":0.02,"hi":0.025,"n":460},{"lo":0.025,"hi":0.03,"n":419},{"lo":0.03,"hi":0.035,"n":351},{"lo":0.035,"hi":0.04,"n":315},{"lo":0.04,"hi":0.045,"n":321},{"lo":0.045,"hi":0.05,"n":242},{"lo":0.05,"hi":0.055,"n":230},{"lo":0.055,"hi":0.06,"n":220},{"lo":0.06,"hi":0.065,"n":197},{"lo":0.065,"hi":0.07,"n":168},{"lo":0.07,"hi":0.075,"n":178},{"lo":0.075,"hi":0.08,"n":156},{"lo":0.08,"hi":0.085,"n":180},{"lo":0.085,"hi":0.09,"n":154},{"lo":0.09,"hi":0.095,"n":143},{"lo":0.095,"hi":0.1,"n":151},{"lo":0.1,"hi":0.105,"n":127},{"lo":0.105,"hi":0.11,"n":123},{"lo":0.11,"hi":0.115,"n":95},{"lo":0.115,"hi":0.12,"n":93},{"lo":0.12,"hi":0.125,"n":83},{"lo":0.125,"hi":0.13,"n":86},{"lo":0.13,"hi":0.135,"n":87},{"lo":0.135,"hi":0.14,"n":69},{"lo":0.14,"hi":0.145,"n":50},{"lo":0.145,"hi":0.15,"n":44},{"lo":0.15,"hi":0.155,"n":55},{"lo":0.155,"hi":0.16,"n":42},{"lo":0.16,"hi":0.165,"n":43},{"lo":0.165,"hi":0.17,"n":32},{"lo":0.17,"hi":0.175,"n":29},{"lo":0.175,"hi":0.18,"n":20},{"lo":0.18,"hi":0.185,"n":20},{"lo":0.185,"hi":0.19,"n":20},{"lo":0.19,"hi":0.195,"n":15},{"lo":0.195,"hi":0.2,"n":12},{"lo":0.2,"hi":0.21,"n":13},{"lo":0.21,"hi":0.22,"n":15},{"lo":0.22,"hi":null,"n":20}],"male":[{"lo":0,"hi":0.005,"n":1572},{"lo":0.005,"hi":0.01,"n":1211},{"lo":0.01,"hi":0.015,"n":797},{"lo":0.015,"hi":0.02,"n":597},{"lo":0.02,"hi":0.025,"n":470},{"lo":0.025,"hi":0.03,"n":405},{"lo":0.03,"hi":0.035,"n":357},{"lo":0.035,"hi":0.04,"n":304},{"lo":0.04,"hi":0.045,"n":261},{"lo":0.045,"hi":0.05,"n":249},{"lo":0.05,"hi":0.055,"n":234},{"lo":0.055,"hi":0.06,"n":208},{"lo":0.06,"hi":0.065,"n":210},{"lo":0.065,"hi":0.07,"n":188},{"lo":0.07,"hi":0.075,"n":184},{"lo":0.075,"hi":0.08,"n":176},{"lo":0.08,"hi":0.085,"n":158},{"lo":0.085,"hi":0.09,"n":137},{"lo":0.09,"hi":0.095,"n":163},{"lo":0.095,"hi":0.1,"n":141},{"lo":0.1,"hi":0.105,"n":159},{"lo":0.105,"hi":0.11,"n":125},{"lo":0.11,"hi":0.115,"n":131},{"lo":0.115,"hi":0.12,"n":136},{"lo":0.12,"hi":0.125,"n":107},{"lo":0.125,"hi":0.13,"n":113},{"lo":0.13,"hi":0.135,"n":115},{"lo":0.135,"hi":0.14,"n":117},{"lo":0.14,"hi":0.145,"n":97},{"lo":0.145,"hi":0.15,"n":107},{"lo":0.15,"hi":0.155,"n":99},{"lo":0.155,"hi":0.16,"n":75},{"lo":0.16,"hi":0.165,"n":66},{"lo":0.165,"hi":0.17,"n":71},{"lo":0.17,"hi":0.175,"n":59},{"lo":0.175,"hi":0.18,"n":52},{"lo":0.18,"hi":0.185,"n":48},{"lo":0.185,"hi":0.19,"n":41},{"lo":0.19,"hi":0.195,"n":39},{"lo":0.195,"hi":0.2,"n":21},{"lo":0.2,"hi":0.21,"n":51},{"lo":0.21,"hi":0.22,"n":38},{"lo":0.22,"hi":null,"n":117}]}}}         ;

const OMOP_RULES = "# How to obtain the PREVENT inputs from UCSF OMOP\n\nThese are the rules of the UCSF cardiovascular case study (protocol 1.5, derivation code 0.3). The calculator applies the same rules, so that its results match the validated case study. Database: UCSF de-identified OMOP CDM on SQL Server (schema `omop`), reached only through the UCSFOMOPAgent extension.\n\n## Inputs for one person at an as-of date\n\n| Input | Source | Rule |\n|---|---|---|\n| Age | `person.year_of_birth` | as-of year minus year of birth. **Year of birth 1935 is top-coded** (people aged 90 or more at de-identification): the true age may be higher, so show a warning. |\n| Sex | `person.gender_concept_id` | 8532 female, 8507 male |\n| Systolic BP | measurement, LOINC 8480-6 | closest value on or before the as-of date, within 1 year |\n| Total cholesterol | measurement, LOINC 2093-3 or SNOMED 121868005 | closest within 3 years |\n| HDL cholesterol | measurement, LOINC 2085-9 or SNOMED 28036006 | closest within 3 years |\n| BMI | measurement, LOINC 39156-5 or SNOMED 60621009, 698094009 | closest within 3 years; measured BMI only (weight and height carry no unit at UCSF) |\n| Creatinine (for eGFR) | measurement, LOINC 2160-0 | closest within 1 year |\n| eGFR | computed | CKD-EPI 2021: 142 \u00d7 min(Scr/\u03ba, 1)^\u03b1 \u00d7 max(Scr/\u03ba, 1)^\u22121.200 \u00d7 0.9938^age \u00d7 1.012 if female; \u03ba = 0.7 (female) or 0.9 (male); \u03b1 = \u22120.241 (female) or \u22120.302 (male); Scr in mg/dL |\n| Diabetes | conditions, HbA1c, drugs | any of: an ICD-9-CM 250 or ICD-10-CM E08, E09, E10, E11, E13 code on or before the as-of date; HbA1c \u2265 6.5% within 3 years (LOINC 17856-6, 4548-4 or SNOMED 43396009; mmol/mol converted with 0.0915 \u00d7 value + 2.15); a drug in ATC A10 started within 365 days |\n| BP treatment | drugs | a drug in ATC C02, C03, C07, C08 or C09 started within 365 days before the as-of date |\n| Statin | drugs | a drug in ATC C10AA, C10BA or C10BX started within 365 days |\n| Current smoking | observation | the most recent record within 3 years. Current: observation_concept_id 903657, 903659, 903660, 903661, 903663, 903664, 903654, 2514534, 2514535, 4021160, 40664492, 40664586, 2108525. Not current: 903651, 903653, 42872410. **No record = not current** (UCSF records smoking almost only for smokers); show \"not documented\". |\n\nMeasurements: match `measurement_concept_id` to `concept` by vocabulary and code. When several values share the closest date, use their mean. Units (`unit_concept_id`): cholesterol mg/dL 8840 (mmol/L 8753 \u00d7 38.67); creatinine mg/dL 8840 (\u00b5mol/L 8749 \u00d7 0.011312); SBP mmHg 8876; values without a unit (0) are accepted for these measurements. Drop implausible raw values: SBP 50\u2013300 mmHg, total cholesterol 20\u20131000 mg/dL, HDL 5\u2013250 mg/dL, creatinine 0.1\u201325 mg/dL, BMI 10\u2013100, HbA1c 3\u201320%.\n\nDrugs: ATC classes reach RxNorm drugs through `concept_ancestor` (ancestor = the ATC class concept, descendant = `drug_exposure.drug_concept_id`); use `drug_exposure_start_date`.\n\nConditions: match the ICD code of the source concept (`condition_source_concept_id` \u2192 `concept`, vocabulary ICD9CM or ICD10CM, code with the decimal point); codes match themselves and every longer code that begins with them.\n\n## PREVENT input ranges\n\nPREVENT and `preventr` accept: age 30\u201379, SBP 90\u2013180 mmHg, total cholesterol 130\u2013320 mg/dL, HDL 20\u2013100 mg/dL, BMI 18.5\u201339.9, eGFR 15\u2013140. Outside these ranges, show the value and do not compute.\n\n## Cohort requests in chat (eligibility for a PREVENT distribution)\n\nAge 30\u201379 at the as-of date; no prior cardiovascular disease on or before the as-of date (ICD-9-CM 410, 411, 412, 413, 414, 428, 398.91, 402.01, 402.11, 402.91, 404.01, 404.03, 404.11, 404.13, 404.91, 404.93, 430\u2013438, 440.2, 440.3, 443.9, V45.81, V45.82; ICD-10-CM I20\u2013I25, I50, I09.81, I11.0, I13.0, I13.2, I60\u2013I69, G45 except G45.4, I70.2\u2013I70.7, I73.9, Z95.1, Z95.5, Z98.61; or a condition whose standard SNOMED concept descends from 316139, 4185932, 381591 or 3654996); at least 365 days between the first recorded visit and the as-of date; complete inputs. Report how many were excluded by each rule.\n\n## Writing SQL against UCSF OMOP (lessons from the case study)\n\n- T-SQL (SQL Server). Start from the set of persons, then join the large tables to it on `person_id`; never filter a large table with an expression on `person_id`.\n- Do not write the character `%` in a query (the connector formats queries with Python `%`): use `LEFT(code, n) IN (...)` for code prefixes, or `CHAR(37)` in a LIKE pattern.\n- Use `omop-submit_query_job` for anything that may take more than a minute; wait for completion before reading results.\n- Results that reach the chat or the app for a cohort are aggregates only; counts from 1 to 10 are shown as \"<11\".\n\n## UCSF data facts\n\nDates are shifted per person by up to about one year (the de-identification). There is no emergency department visit type in the database. A study cut-off of 2025-01-31 applies (data after it are incomplete).\n"         ;

/** MOCK calibration parameters (illustrative, not fitted to patient data); see README. */
const UCSF_RECALIBRATION = {"description":"MOCK calibration for privacy purposes: an illustrative rescaling to about 80% of the PREVENT risk. It is NOT the UCSF recalibration of the case study and was not fitted to any patient data.","formula":"x = ln(p / (1 - p)), with p the PREVENT 10-year risk; UCSF-calibrated risk = 1 - s0_10y ^ exp(beta * (x - x_mean))","mock":true,"method":"MOCK: fixed illustrative parameters (beta = 1, x_mean = 0, s0_10y = exp(-0.8)); replace with parameters fitted on a local cohort.","limitations":"Illustrative only; not a calibration.","outcomes":{"total_cvd":{"beta":1.0,"x_mean":0.0,"s0_10y":0.449329,"mock":true,"examples":{"0.02":0.0162,"0.05":0.0412,"0.10":0.0851,"0.20":0.1813}},"ascvd":{"beta":1.0,"x_mean":0.0,"s0_10y":0.449329,"mock":true,"examples":{"0.02":0.0162,"0.05":0.0412,"0.10":0.0851,"0.20":0.1813}},"hf":{"beta":1.0,"x_mean":0.0,"s0_10y":0.449329,"mock":true,"examples":{"0.02":0.0162,"0.05":0.0412,"0.10":0.0851,"0.20":0.1813}}}}         ;

return { PREVENT_COEFFICIENTS, REFERENCE_COHORT, OMOP_RULES, UCSF_RECALIBRATION };
})();

// ---- src/prevent.ts
__mod.prevent = (function () {
const { PREVENT_COEFFICIENTS, REFERENCE_COHORT, UCSF_RECALIBRATION } = __mod.preventData;


                                    
                                                   
                                                                                                                                                                    

                                
                                                                                                                    
                                                                                                                         
 

                                                                                                                                      

const RANGE_LABELS                           = {
  age: "30-79 years", sex: "female or male", sbp: "90-180 mmHg", total_cholesterol_mg_dl: "130-320 mg/dL",
  hdl_mg_dl: "20-100 mg/dL", bmi: "18.5-39.9 kg/m2", egfr: "15-140 mL/min/1.73 m2",
  diabetes: "yes/no", current_smoker: "yes/no", bp_treatment: "yes/no", statin: "yes/no"
};

function toFlag(v                                              )         {
  return v === true || v === 1 || v === "1" || v === "true" || v === "yes" ? 1 : 0;
}

function checkRange(name          , value         , lo        , hi        , problems          ) {
  const n = Number(value);
  if (!Number.isFinite(n)) { problems.push(`${name} must be numeric`); return; }
  if (n < lo || n > hi) problems.push(`${name} ${n} is outside PREVENT range ${RANGE_LABELS[name]}`);
}

function validateInputs(p                        )           {
  const problems           = [];
  const req             = ["age","sex","sbp","total_cholesterol_mg_dl","hdl_mg_dl","bmi","egfr","diabetes","current_smoker","bp_treatment","statin"];
  for (const k of req) if ((p       )[k] === undefined || (p       )[k] === null || (p       )[k] === "") problems.push(`${k} is missing`);
  if (p.sex !== "female" && p.sex !== "male") problems.push("sex must be female or male");
  checkRange("age", p.age, 30, 79, problems); checkRange("sbp", p.sbp, 90, 180, problems);
  checkRange("total_cholesterol_mg_dl", p.total_cholesterol_mg_dl, 130, 320, problems); checkRange("hdl_mg_dl", p.hdl_mg_dl, 20, 100, problems);
  checkRange("bmi", p.bmi, 18.5, 39.9, problems); checkRange("egfr", p.egfr, 15, 140, problems);
  return problems;
}

function terms(p               )                         {
  const age = (Number(p.age) - 55) / 10;
  const nonHdl = (Number(p.total_cholesterol_mg_dl) - Number(p.hdl_mg_dl)) * 0.02586 - 3.5;
  const hdl = (Number(p.hdl_mg_dl) * 0.02586 - 1.3) / 0.3;
  const sbpLow = (Math.min(Number(p.sbp), 110) - 110) / 20;
  const sbpHigh = (Math.max(Number(p.sbp), 110) - 130) / 20;
  const bmiLow = (Math.min(Number(p.bmi), 30) - 25) / 5;
  const bmiHigh = (Math.max(Number(p.bmi), 30) - 30) / 5;
  const egfrLow = (Math.min(Number(p.egfr), 60) - 60) / -15;
  const egfrHigh = (Math.max(Number(p.egfr), 60) - 90) / -15;
  const diabetes = toFlag(p.diabetes), smoker = toFlag(p.current_smoker), bpTx = toFlag(p.bp_treatment), statin = toFlag(p.statin);
  return {
    "Age, per 10 years": age, "non-HDL-C per 1 mmol/L": nonHdl, "HDL-C per 0.3 mmol/L": hdl,
    "SBP <110 per 20 mmHg": sbpLow, "SBP ≥110 per 20 mmHg": sbpHigh, "Diabetes": diabetes, "Current smoking": smoker,
    "BMI <30, per 5 kg/m2": bmiLow, "BMI 30+, per 5 kg/m2": bmiHigh, "eGFR <60, per -15 ml": egfrLow, "eGFR 60+, per -15 ml": egfrHigh,
    "Anti-hypertensive use": bpTx, "Statin use": statin, "Treated SBP ≥110 mm Hg per 20 mm Hg": bpTx * sbpHigh, "Treated non-HDL-C": statin * nonHdl,
    "Age per 10yr * non-HDL-C per 1 mmol/L": age * nonHdl, "Age per 10yr * HDL-C per 1 mml/L": age * hdl,
    "Age per 10yr * SBP ≥110 mm Hg per 20 mmHg": age * sbpHigh, "Age per 10yr * diabetes": age * diabetes, "Age per 10yr * current smoking": age * smoker,
    "Age per 10yr * BMI 30+ per 5 kg/m2": age * bmiHigh, "Age per 10yr * eGFR <60, per -15 ml": age * egfrLow, "Constant": 1
  };
}

function preventRisk(p               , risk         )         {
  const t = terms(p); const coeffs = (PREVENT_COEFFICIENTS       ).coefficients[risk][p.sex]; let lp = 0;
  for (const [name, beta] of Object.entries(coeffs)) lp += Number(beta) * (t[name] ?? 0);
  return 1 / (1 + Math.exp(-lp));
}

function computeRisks(p               )             {
  const problems = validateInputs(p);
  if (problems.length) return { total_cvd: null, ascvd: null, hf: null, valid: false, problems };
  return { total_cvd: preventRisk(p, "total_cvd"), ascvd: preventRisk(p, "ascvd"), hf: preventRisk(p, "hf"), valid: true, problems: [] };
}

/** UCSF-calibrated 10-year risk from the published PREVENT risk: 1 - s0 ^ exp(beta * (logit(p) - x_mean)).
 *  The transformation is monotone, so the percentile in the reference cohort is the same on both scales. */
function ucsfCalibrated(risk         , p                           )                {
  if (p === null || p === undefined || !Number.isFinite(p) || p <= 0 || p >= 1) return null;
  const c = (UCSF_RECALIBRATION       ).outcomes[risk];
  const x = Math.log(p / (1 - p));
  return 1 - Math.pow(c.s0_10y, Math.exp(c.beta * (x - c.x_mean)));
}

function riskPercentile(risk         , value        , sex                  = "overall")         {
  const bins = ((REFERENCE_COHORT       ).distribution[risk] || {})[sex] || [];
  const n = Number((REFERENCE_COHORT       ).n[sex]) || bins.reduce((s        , b     ) => s + Number(b.n || 0), 0);
  if (!bins.length || !Number.isFinite(value) || n <= 0) return NaN;
  let below = 0;
  for (const b of bins) {
    const lo = Number(b.lo), hi = Number(b.hi), count = Number(b.n);
    if (value >= hi) { below += count; continue; }
    if (value < lo) break;
    below += count * Math.min(1, Math.max(0, (value - lo) / Math.max(hi - lo, 1e-9))); break;
  }
  return Math.max(0, Math.min(100, (below / n) * 100));
}

function formatPct(x                           , digits = 1)         {
  return x === null || x === undefined || !Number.isFinite(x) ? "-" : `${(x * 100).toFixed(digits)}%`;
}

function projectAges(p               , years = 10)                                                              {
  const out                                                              = [];
  const start = Math.ceil(Number(p.age)); const last = Math.min(79, start + years);
  for (let age = start; age <= last; age++) { const q = {...p, age}; const r = computeRisks(q); if (r.valid && r.total_cvd !== null && r.ascvd !== null && r.hf !== null) out.push({age,total_cvd:r.total_cvd,ascvd:r.ascvd,hf:r.hf}); }
  return out;
}

function ckdepi2021(scrMgDl        , age        , sex     )         {
  const female = sex === "female"; const kappa = female ? 0.7 : 0.9; const alpha = female ? -0.241 : -0.302; const ratio = scrMgDl / kappa;
  return 142 * Math.pow(Math.min(ratio, 1), alpha) * Math.pow(Math.max(ratio, 1), -1.2) * Math.pow(0.9938, age) * (female ? 1.012 : 1);
}

const referenceCohort = REFERENCE_COHORT;
const ucsfRecalibration = UCSF_RECALIBRATION;
const coefficients = PREVENT_COEFFICIENTS;

return { RANGE_LABELS, toFlag, validateInputs, terms, preventRisk, computeRisks, ucsfCalibrated, riskPercentile, formatPct, projectAges, ckdepi2021, referenceCohort, ucsfRecalibration, coefficients };
})();

// ---- src/cohort.ts
__mod.cohort = (function () {
const { computeRisks, ucsfCalibrated } = __mod.prevent;
// Cohort analysis for the PREVENT calculator: no DOM here, so it can be tested with node.
// A cohort arrives from a Biorouter chat (handoff.mjs) or from the synthetic demo generator.


                                                                                          
                                                                                                                                                 
                                                                                                                                                                   
                                                                           
                                                                                                       
                                                                                                                                                           

const OUTCOMES            = ["total_cvd", "ascvd", "hf"];
const INPUT_KEYS = ["age", "sex", "sbp", "total_cholesterol_mg_dl", "hdl_mg_dl", "bmi", "egfr", "diabetes", "current_smoker", "bp_treatment", "statin"];
const NUMERIC = ["age", "sbp", "total_cholesterol_mg_dl", "hdl_mg_dl", "bmi", "egfr"];
const FLAGS = ["diabetes", "current_smoker", "bp_treatment", "statin"];
const AGE_BANDS                          = [[30, 39], [40, 49], [50, 59], [60, 69], [70, 79]];

function parseSex(v         )                           {
  const s = String(v ?? "").trim().toLowerCase();
  if (["female", "f", "woman", "8532"].includes(s)) return "female";
  if (["male", "m", "man", "8507"].includes(s)) return "male";
  return null;
}
function parseFlag(v         )                 {
  const s = String(v ?? "").trim().toLowerCase();
  if (["1", "true", "yes", "y", "t"].includes(s)) return true;
  if (["0", "false", "no", "n", "f"].includes(s)) return false;
  return null;
}
function flag(v         )          { return v === true || v === 1 || v === "1" || v === "true"; }

/** Normalise one record to PREVENT inputs; returns the inputs and the names of missing or unreadable inputs. */
function normalize(rec                         )                                               {
  const out      = {}; const missing           = [];
  for (const k of NUMERIC) {
    const raw = rec[k]; const n = raw === null || raw === undefined || raw === "" ? NaN : Number(raw);
    if (Number.isFinite(n)) out[k] = n; else { out[k] = ""; missing.push(k); }
  }
  const sex = parseSex(rec.sex); if (sex) out.sex = sex; else { out.sex = "female"; missing.push("sex"); }
  for (const k of FLAGS) { const b = parseFlag(rec[k]); if (b === null) { out[k] = false; missing.push(k); } else out[k] = b; }
  return { inputs: out                 , missing };
}

function scaleOf(r                                                                       )        { return { total_cvd: r.total_cvd, ascvd: r.ascvd, hf: r.hf }; }
function ucsfOf(s       )        { return { total_cvd: ucsfCalibrated("total_cvd", s.total_cvd), ascvd: ucsfCalibrated("ascvd", s.ascvd), hf: ucsfCalibrated("hf", s.hf) }; }

function evaluate(key        , inputs               , missing          )         {
  const none        = { total_cvd: null, ascvd: null, hf: null };
  if (missing.length) return { key, inputs, status: "missing", problems: missing.slice(), prevent: none, ucsf: none };
  const r = computeRisks(inputs);
  if (!r.valid) return { key, inputs, status: "range", problems: r.problems.map((p) => p.split(" ")[0]), prevent: none, ucsf: none };
  const prevent = scaleOf(r);
  return { key, inputs, status: "ok", problems: [], prevent, ucsf: ucsfOf(prevent) };
}

/** Build a cohort from a handoff payload: {kind:"cohort", columns:[...], rows:[[...]], name, purpose, definition, as_of, source, counts}. */
function buildCohort(payload     )         {
  const cols           = Array.isArray(payload.columns) ? payload.columns.map((c         ) => String(c)) : [];
  const idx                         = {}; cols.forEach((c, i) => { idx[c] = i; });
  for (const k of INPUT_KEYS) if (!(k in idx)) throw new Error("the cohort has no column " + k);
  const rows              = Array.isArray(payload.rows) ? payload.rows : [];
  const members = rows.map((row, i) => {
    const rec                          = {}; for (const k of INPUT_KEYS) rec[k] = row[idx[k]];
    const key = "key" in idx && row[idx.key] !== undefined && row[idx.key] !== "" ? String(row[idx.key]) : "C" + String(i + 1).padStart(5, "0");
    const n = normalize(rec); return evaluate(key, n.inputs, n.missing);
  });
  const meta             = {
    name: String(payload.name || "Unnamed cohort"), purpose: String(payload.purpose || ""), definition: String(payload.definition || ""),
    as_of: String(payload.as_of || ""), source: String(payload.source || "Biorouter chat"), received: new Date().toISOString().slice(0, 16).replace("T", " "),
    upstream: payload.counts && typeof payload.counts === "object" ? payload.counts : {},
  };
  return { meta, members, ok: members.filter((m) => m.status === "ok") };
}

function quantile(sorted          , q        )         {
  if (!sorted.length) return NaN; const pos = (sorted.length - 1) * q; const lo = Math.floor(pos), hi = Math.ceil(pos);
  return lo === hi ? sorted[lo] : sorted[lo] * (hi - pos) + sorted[hi] * (pos - lo);
}
function summarize(values          )          {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b); const sum = v.reduce((s, x) => s + x, 0);
  return { n: v.length, mean: v.length ? sum / v.length : NaN, median: quantile(v, 0.5), q1: quantile(v, 0.25), q3: quantile(v, 0.75), sum };
}
function values(members          , k         , scale                    )           { return members.map((m) => m[scale][k]).filter((x)              => x !== null); }

/** Counts of excluded members by reason and input. */
function exclusions(c        )                                                                      {
  const byInput                         = {}; let missing = 0, range = 0;
  for (const m of c.members) {
    if (m.status === "missing") missing++; else if (m.status === "range") range++; else continue;
    for (const p of m.problems) byInput[(m.status === "missing" ? "missing " : "out of range ") + p] = (byInput[(m.status === "missing" ? "missing " : "out of range ") + p] || 0) + 1;
  }
  return { missing, range, byInput };
}

                                                                                                
function breakdown(ok          )             {
  const groups                                          = [
    ["Women", (m) => m.inputs.sex === "female"], ["Men", (m) => m.inputs.sex === "male"],
    ...AGE_BANDS.map(([a, b]) => ["Age " + a + "–" + b, (m        ) => Number(m.inputs.age) >= a && Number(m.inputs.age) <= b]                                    ),
  ];
  return groups.map(([label, pick]) => {
    const g = ok.filter(pick); const medians      = {};
    for (const k of OUTCOMES) medians[k] = [summarize(values(g, k, "prevent")).median, summarize(values(g, k, "ucsf")).median];
    return { label, n: g.length, medians };
  });
}

/** Share of members per bin of width w (risk scale), up to xmax; the last bin collects the tail. */
function histogram(vals          , w        , xmax        )                                               {
  const nb = Math.round(xmax / w); const bins = Array.from({ length: nb }, (_, i) => ({ lo: i * w, hi: i === nb - 1 ? 1 : (i + 1) * w, n: 0 }));
  for (const v of vals) bins[Math.min(nb - 1, Math.max(0, Math.floor(v / w)))].n++;
  return bins;
}

/** Empirical percentile of a value among the cohort's members (mid-rank for ties). */
function cohortPercentile(vals          , value        )         {
  if (!vals.length || !Number.isFinite(value)) return NaN;
  let below = 0, equal = 0; for (const v of vals) { if (v < value) below++; else if (v === value) equal++; }
  return ((below + equal / 2) / vals.length) * 100;
}

// ── what if: PREVENT recomputed with literally changed inputs, only for the members a change applies to ──
// No treatment effects are assumed: PREVENT is a prognostic equation, and statin use is an input describing
// current treatment, so setting it to "yes" recomputes the equation for a treated person with the same values.
const RULES         = [
  { id: "sbp", label: "Systolic blood pressure −10 mmHg", eligibility: "all members (not below 90 mmHg, the PREVENT lower limit)", eligible: (p) => Number(p.sbp) > 90, apply: (p) => ({ ...p, sbp: Math.max(90, Number(p.sbp) - 10) }) },
  { id: "smoking", label: "Stop smoking", eligibility: "current smokers", eligible: (p) => flag(p.current_smoker), apply: (p) => ({ ...p, current_smoker: false }) },
  { id: "statin", label: "Statin use: yes", eligibility: "members not on a statin", eligible: (p) => !flag(p.statin), apply: (p) => ({ ...p, statin: true }) },
];

                              
                                                                   
                                                                                                                                                        
  

/** Recompute PREVENT for every computable member with the chosen input changes applied where they fit.
 *  Reports the sum of predicted 10-year risks (expected events under the model) and the median, before and after. */
function cohortScenario(ok          , ruleIds          )                 {
  const rules = RULES.filter((r) => ruleIds.includes(r.id)); const perRule                         = {};
  for (const r of rules) perRule[r.id] = ok.filter((m) => r.eligible(m.inputs)).length;
  const before                                        = { total_cvd: [[], []], ascvd: [[], []], hf: [[], []] };
  const after                                        = { total_cvd: [[], []], ascvd: [[], []], hf: [[], []] };
  let changed = 0, higher = 0;
  for (const m of ok) {
    let p = m.inputs; let touched = false;
    for (const r of rules) if (r.eligible(m.inputs)) { p = r.apply(p); touched = true; }
    const res = touched ? computeRisks(p) : null; const use = touched && res && res.valid;
    if (use) { changed++; if ((res .total_cvd          ) > (m.prevent.total_cvd          )) higher++; }
    for (const k of OUTCOMES) {
      const b = m.prevent[k]          ; const a = use ? (res [k]          ) : b;
      before[k][0].push(b); before[k][1].push(m.ucsf[k]          );
      after[k][0].push(a); after[k][1].push(use ? (ucsfCalibrated(k, a)          ) : (m.ucsf[k]          ));
    }
  }
  const out      = {};
  for (const k of OUTCOMES) {
    const sb = before[k].map((v) => summarize(v)), sa = after[k].map((v) => summarize(v));
    out[k] = { sumBefore: [sb[0].sum, sb[1].sum], sumAfter: [sa[0].sum, sa[1].sum], medianBefore: [sb[0].median, sb[1].median], medianAfter: [sa[0].median, sa[1].median] };
  }
  return { perRule, changed, higher, outcomes: out };
}

// ── synthetic demo cohort (not patient data), deterministic ──
function rng(seed        )               { let s = seed >>> 0; return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; }; }
function normal(r              , mean        , sd        )         { const u = Math.max(r(), 1e-12), v = r(); return mean + sd * Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v); }
function demoPayload(n = 600, seed = 2026)      {
  const r = rng(seed); const rows              = [];
  for (let i = 0; i < n; i++) {
    const age = Math.round(Math.min(79, Math.max(30, normal(r, 58, 11)))); const female = r() < 0.55;
    const diabetes = r() < 0.08 + 0.004 * (age - 30); const bpTx = r() < 0.1 + 0.009 * (age - 30); const statin = r() < 0.05 + 0.006 * (age - 30) + (diabetes ? 0.25 : 0);
    let sbp                  = Math.round(normal(r, 118 + 0.35 * (age - 30) + (bpTx ? 6 : 0), 15));
    let tc                  = Math.round(normal(r, statin ? 172 : 200, 36)); const hdl = Math.round(normal(r, female ? 60 : 48, 13));
    let bmi                  = Math.round(normal(r, 28.5, 5.5) * 10) / 10; const egfr = Math.round(Math.min(130, normal(r, 108 - 0.75 * (age - 30), 15)));
    if (r() < 0.06) tc = ""; if (r() < 0.03) bmi = ""; if (r() < 0.02) sbp = "";
    rows.push(["D" + String(i + 1).padStart(4, "0"), age, female ? "female" : "male", sbp, tc, hdl, bmi, egfr, diabetes ? 1 : 0, r() < 0.11 ? 1 : 0, bpTx ? 1 : 0, statin ? 1 : 0]);
  }
  return { v: 1, kind: "cohort", name: "Synthetic demo cohort", purpose: "Demonstration only: simulated adults, not patient data.", definition: "600 simulated adults aged 30–79 without cardiovascular disease, with some missing and out-of-range values.", as_of: "2016-01-01", source: "Synthetic (generated in the app)", columns: ["key"].concat(INPUT_KEYS), rows };
}

return { OUTCOMES, INPUT_KEYS, AGE_BANDS, parseSex, parseFlag, normalize, evaluate, buildCohort, quantile, summarize, values, exclusions, breakdown, histogram, cohortPercentile, RULES, cohortScenario, demoPayload };
})();

// ---- src/main.ts
__mod.main = (function () {
const { createApp, mountTimeline } = __mod.sdk;
const { computeRisks, formatPct, projectAges, referenceCohort, riskPercentile, ucsfCalibrated, ucsfRecalibration } = __mod.prevent;
const { OMOP_RULES } = __mod.preventData;
const { RULES, breakdown, buildCohort, cohortPercentile, cohortScenario, demoPayload, exclusions, histogram, normalize, quantile, summarize, values } = __mod.cohort;





                                                                                   
                                             
                                                                       
                                                                                                                                                                                         
                                        
                                                 

const br = createApp({ autoChat: false });
mountTimeline(br, "[data-br-progress]", { maxItems: 12 });

const demo                = { age: 58, sex: "female", sbp: 132, total_cholesterol_mg_dl: 198, hdl_mg_dl: 54, bmi: 28.4, egfr: 82, diabetes: false, current_smoker: false, bp_treatment: true, statin: false };
let patient                = { ...demo };
let scenario                = { ...demo };
let sources            = labelAll("Synthetic demo value");
let missing                          = {};
let scenarios             = [];
let cohort                = null;
let origin                                       = null;
let shownRows = 50;

const RISKS            = ["total_cvd", "ascvd", "hf"];
const LABEL                          = { total_cvd: "Total CVD", ascvd: "ASCVD", hf: "Heart failure" };
const PLAIN                          = { total_cvd: "heart attack, stroke or heart failure", ascvd: "heart attack or stroke", hf: "heart failure" };
// Input ranges are those of PREVENT and of the reference cohort (adults 30-79).
const FIELDS          = [
  { key: "age", label: "Age", short: "Age", kind: "number", unit: "years", min: 30, max: 79, step: 1, modifiable: false },
  { key: "sex", label: "Sex", short: "Sex", kind: "select", unit: "", min: 0, max: 0, step: 1, modifiable: false },
  { key: "sbp", label: "Systolic blood pressure", short: "SBP", kind: "number", unit: "mmHg", min: 90, max: 180, step: 1, modifiable: true },
  { key: "total_cholesterol_mg_dl", label: "Total cholesterol", short: "Total chol.", kind: "number", unit: "mg/dL", min: 130, max: 320, step: 1, modifiable: true },
  { key: "hdl_mg_dl", label: "HDL cholesterol", short: "HDL", kind: "number", unit: "mg/dL", min: 20, max: 100, step: 1, modifiable: true },
  { key: "bmi", label: "Body-mass index", short: "BMI", kind: "number", unit: "kg/m²", min: 18.5, max: 39.9, step: 0.1, modifiable: true },
  { key: "egfr", label: "eGFR", short: "eGFR", kind: "number", unit: "mL/min/1.73 m²", min: 15, max: 140, step: 1, modifiable: true },
  { key: "diabetes", label: "Diabetes", short: "Diabetes", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "current_smoker", label: "Current smoking", short: "Smoking", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "bp_treatment", label: "Blood-pressure treatment", short: "BP treatment", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
  { key: "statin", label: "Statin", short: "Statin", kind: "toggle", unit: "", min: 0, max: 1, step: 1, modifiable: true },
];
// The examples of the brief; each changes inputs literally and PREVENT is recomputed.
const PRESETS                                                    = [
  { label: "SBP −10 mmHg", apply: (s) => { s.sbp = Math.max(90, Number(s.sbp) - 10); } },
  { label: "Stop smoking", apply: (s) => { s.current_smoker = false; } },
  { label: "Statin use: yes", apply: (s) => { s.statin = true; } },
];
const SVG_NS = "http://www.w3.org/2000/svg";

// ── helpers ─────────────────────────────────────────────────────────────────
function el                       (id        )    { return document.getElementById(id)     ; }
function make                                       (tag   , cls = "", text = "")                           {
  const e = document.createElement(tag); if (cls) e.className = cls; if (text) e.textContent = text; return e;
}
function svg(tag        , attrs                                 , text = "")             {
  const e = document.createElementNS(SVG_NS, tag); for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v)); if (text) e.textContent = text; return e;
}
function flag(v         )          { return v === true || v === 1 || v === "1" || v === "true"; }
function labelAll(source        )            { const m            = {}; for (const k of ["age","sex","sbp","total_cholesterol_mg_dl","hdl_mg_dl","bmi","egfr","diabetes","current_smoker","bp_treatment","statin"]) m[k] = { source }; return m; }
function group()        { return (el                   ("sexFilter").value || "overall")         ; }
function groupName(g       )         { return g === "female" ? "women (MOCK reference)" : g === "male" ? "men (MOCK reference)" : g === "cohort" ? "members of this cohort" : "MOCK reference adults"; }
function pct(x                           , d = 1)         { return formatPct(x === undefined ? null : x, d); }
function signed(x        , digits        , unit        )         { return (x > 0 ? "+" : x < 0 ? "−" : "±") + Math.abs(x).toFixed(digits) + unit; }
function showPatients()          { return el                  ("showPatients").checked; }
/** Counts from 1 to 10 are shown as "<11" unless individual patients are shown. */
function cnt(n        )         { return !showPatients() && n >= 1 && n <= 10 ? "<11" : n.toLocaleString("en-US"); }
function fmtValue(f       , v         )         {
  if (f.kind === "toggle") return flag(v) ? "Yes" : "No";
  if (f.kind === "select") return v === "male" ? "Male" : "Female";
  return v === "" || v === undefined || v === null ? "–" : String(v) + (f.unit ? " " + f.unit : "");
}
function fieldLabel(key        )         { const f = FIELDS.find((x) => x.key === key); return f ? f.label : key; }
function fieldProblem(f       , v         )         {
  if (f.kind !== "number") return "";
  const n = Number(v);
  if (v === "" || v === null || v === undefined || !Number.isFinite(n)) return "Enter a value";
  if (n < f.min || n > f.max) return "Outside the PREVENT range (" + f.min + "–" + f.max + "): risk not computed";
  return "";
}
function problems(p               )          { return FIELDS.filter((f) => fieldProblem(f, (p       )[f.key])); }
let toastTimer = 0;
function toast(text        ) { const t = el                ("toast"); t.textContent = text; t.hidden = false; window.clearTimeout(toastTimer); toastTimer = window.setTimeout(() => { t.hidden = true; }, 6000); }

// percentile and distribution for the chosen comparison group
function percentileIn(k         , v               , g       )         {
  if (v === null) return NaN;
  if (g === "cohort") return cohort ? cohortPercentile(values(cohort.ok, k, "prevent"), v) : NaN;
  return riskPercentile(k, v, g);
}
function binsFor(k         , g       )                             {
  if (g === "cohort") {
    if (!cohort) return { bins: [], n: 0 };
    const v = values(cohort.ok, k, "prevent").sort((a, b) => a - b);
    const xmax = Math.min(0.6, Math.max(0.1, Math.ceil(quantile(v, 0.99) * 1.1 * 20) / 20));
    return { bins: histogram(v, 0.005, xmax), n: v.length };
  }
  return { bins: ((referenceCohort       ).distribution[k] || {})[g] || [], n: Number((referenceCohort       ).n[g]) };
}

// ── patient form (built once per patient, so typing never loses focus) ────
function renderForm() {
  const host = el                ("fields"); host.textContent = "";
  for (const f of FIELDS) {
    const wrap = make("label", "br-field field" + (f.kind === "toggle" ? " field-toggle" : ""));
    wrap.appendChild(make("span", "br-label", f.unit ? f.label + " (" + f.unit + ")" : f.label));
    let control             ;
    if (f.kind === "select") {
      const s = make("select", "br-select");
      for (const o of ["female", "male"]) { const opt = make("option", "", o === "female" ? "Female" : "Male"); opt.value = o; s.appendChild(opt); }
      s.value = String(patient.sex); s.onchange = () => setInput(f, s.value); control = s;
    } else if (f.kind === "toggle") {
      const row = make("span", "toggle"); const c = make("input"); c.type = "checkbox"; c.checked = flag((patient       )[f.key]);
      const t = make("span", "toggle-text", c.checked ? "Yes" : "No");
      c.onchange = () => { t.textContent = c.checked ? "Yes" : "No"; setInput(f, c.checked); };
      c.id = "input-" + f.key; row.append(c, t); control = row;
    } else {
      const i = make("input", "br-input"); i.type = "number"; i.inputMode = "decimal";
      i.min = String(f.min); i.max = String(f.max); i.step = String(f.step); i.value = String((patient       )[f.key] ?? "");
      i.oninput = () => setInput(f, i.value === "" ? "" : Number(i.value)); control = i;
    }
    if (f.kind !== "toggle") control.id = "input-" + f.key;
    wrap.appendChild(control);
    const note = make("span", "field-note"); note.id = "note-" + f.key; wrap.appendChild(note);
    host.appendChild(wrap);
  }
  updateFieldNotes();
}

function updateFieldNotes() {
  for (const f of FIELDS) {
    const note = document.getElementById("note-" + f.key); if (!note) continue;
    const prob = fieldProblem(f, (patient       )[f.key]); const s = sources[f.key];
    const where = missing[f.key] ? "Not available in the source: enter by hand" : s && s.source ? s.source + (s.date ? " · " + s.date : "") : "";
    note.textContent = prob || where || (f.kind === "number" ? "Range " + f.min + "–" + f.max : "");
    note.classList.toggle("warn", Boolean(prob) || Boolean(missing[f.key]));
    const ctl = document.getElementById("input-" + f.key); if (ctl) ctl.setAttribute("aria-invalid", prob ? "true" : "false");
  }
}

function setInput(f       , value         ) {
  (patient       )[f.key] = value;
  missing[f.key] = false;
  sources[f.key] = { source: "Entered by hand" };
  scenario = { ...patient };
  br.state.set("/patient", patient);
  br.signals.emit("patient_changed", { changed: f.key, patient });
  updateFieldNotes();
  renderOutputs();
}

// ── risk cards ──────────────────────────────────────────────────────────────
// The twin-scale strip: bar = PREVENT, coral mark = calibrated (MOCK), on a 0-30% scale.
const SCALE_MAX = 0.3;
function at(v        )         { return (Math.min(Math.max(v, 0), SCALE_MAX) / SCALE_MAX) * 100; }
function strip(k         , prevent               , ucsf               )              {
  const s = make("div", "strip"); s.style.setProperty("--series", "var(--c-" + k + ")");
  if (prevent === null) return s;
  const bar = make("span", "strip-bar"); bar.style.width = at(prevent) + "%"; s.appendChild(bar);
  if (ucsf !== null) { const t = make("span", "strip-tick"); t.style.left = at(ucsf) + "%"; s.appendChild(t); }
  s.title = "PREVENT " + pct(prevent) + (ucsf !== null ? ", calibrated (MOCK) " + pct(ucsf) : "") + " (scale 0–30%)";
  return s;
}
function bandStrip(k         , q1        , q3        , median        , medianUcsf        )              {
  const s = make("div", "strip"); s.style.setProperty("--series", "var(--c-" + k + ")");
  if (!Number.isFinite(median)) return s;
  const band = make("span", "strip-band"); band.style.left = at(q1) + "%"; band.style.width = Math.max(0.5, at(q3) - at(q1)) + "%"; s.appendChild(band);
  const med = make("span", "strip-median"); med.style.left = at(median) + "%"; s.appendChild(med);
  const t = make("span", "strip-tick"); t.style.left = at(medianUcsf) + "%"; s.appendChild(t);
  s.title = "PREVENT median " + pct(median) + ", interquartile range " + pct(q1) + "–" + pct(q3) + "; calibrated (MOCK) median " + pct(medianUcsf) + " (scale 0–30%)";
  return s;
}
function valueRow(label        , base               , next                )              {
  const row = make("div", "value-row");
  row.appendChild(make("span", "value-label", label));
  const right = make("span", "value-right");
  if (next === undefined) right.appendChild(make("span", "value", pct(base)));
  else {
    right.appendChild(make("span", "value", pct(base) + " → " + pct(next)));
    if (base !== null && next !== null) {
      const d = next - base; const rel = base > 0 ? (d / base) * 100 : 0;
      right.appendChild(make("span", "delta " + (d < -0.00005 ? "down" : d > 0.00005 ? "up" : ""), signed(d * 100, 1, " pp") + " (" + signed(rel, 0, "%") + ")"));
    }
  }
  row.appendChild(right);
  return row;
}

function riskCard(k         , r       , c        )              {
  const card = make("div", "risk-card"); card.style.setProperty("--series", "var(--c-" + k + ")");
  card.appendChild(make("div", "risk-name", LABEL[k]));
  card.appendChild(make("div", "risk-plain", PLAIN[k]));
  const p = r[k];
  card.appendChild(valueRow("PREVENT", p, c ? c[k] : undefined));
  card.appendChild(valueRow("calibrated (MOCK)", ucsfCalibrated(k, p), c ? ucsfCalibrated(k, c[k]) : undefined));
  if (!c) {
    card.appendChild(strip(k, p, ucsfCalibrated(k, p)));
    const pc = percentileIn(k, p, group());
    card.appendChild(make("div", "risk-pct", Number.isFinite(pc) ? "Higher than " + Math.round(pc) + "% of " + groupName(group()) : "Not computed"));
  }
  return card;
}

function renderRiskCards(target             , base               , compare                ) {
  target.textContent = "";
  const r = computeRisks(base); const c = compare ? computeRisks(compare) : undefined;
  for (const k of RISKS) target.appendChild(riskCard(k, r, c));
}

function renderStatus(id        , p               ) {
  const bad = problems(p); const box = el                (id);
  box.textContent = bad.length ? "Risk not computed. Check: " + bad.map((f) => f.label).join(", ") + "." : "";
  box.hidden = !bad.length;
}

function renderSourceRows() {
  const body = el                         ("sourceRows"); body.textContent = "";
  for (const f of FIELDS) {
    const tr = make("tr"); const s = sources[f.key] || {};
    for (const txt of [f.label, fmtValue(f, (patient       )[f.key]), missing[f.key] ? "Not available; enter by hand" : s.source || "–", s.date || "–"]) tr.appendChild(make("td", "", txt));
    body.appendChild(tr);
  }
}

function renderOrigin() {
  const box = el                ("patientOrigin");
  box.hidden = !origin;
  if (origin) el                 ("originText").textContent = "Member " + origin.key + " of the cohort \"" + origin.name + "\". Values can be edited; the cohort is not changed.";
}

// ── histogram of a distribution (reference cohort or this cohort) with the patient marked ──
function renderHistogram(host               , k         , value               , g       , height        ) {
  host.textContent = "";
  const { bins } = binsFor(k, g);
  const w = Math.max(320, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 640), h = height;
  host.setAttribute("viewBox", "0 0 " + w + " " + h); host.setAttribute("height", String(h));
  if (!bins.length) return;
  const last = bins[bins.length - 1];
  const xmax = Math.min(0.6, Math.max(0.1, Math.ceil((Number(last.lo) + 0.005) * 20) / 20));
  const m = { l: 12, r: 14, t: 18, b: 26 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const X = (v        ) => m.l + (Math.min(Math.max(v, 0), xmax) / xmax) * iw;
  const dens = bins.map((b) => { const hi = Number(b.hi) >= 1 ? xmax : Math.min(Number(b.hi), xmax); return Number(b.n) / Math.max((hi - Number(b.lo)) / 0.005, 1); });
  const dmax = Math.max(...dens, 1e-9);
  const step = xmax <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= xmax + 1e-9; t += step) {
    host.appendChild(svg("line", { x1: X(t), x2: X(t), y1: m.t, y2: m.t + ih, class: "grid" }));
    host.appendChild(svg("text", { x: X(t), y: h - 8, class: "tick", "text-anchor": "middle" }, Math.round(t * 100) + "%"));
  }
  bins.forEach((b, i) => {
    const lo = Number(b.lo), hi = Number(b.hi) >= 1 ? xmax : Math.min(Number(b.hi), xmax);
    if (lo >= xmax || b.n === 0) return;
    const bh = (dens[i] / dmax) * ih; const inBin = value !== null && value >= lo && (value < Number(b.hi) || Number(b.hi) >= 1);
    const rect = svg("rect", { x: X(lo) + 0.5, y: m.t + ih - bh, width: Math.max(1, X(hi) - X(lo) - 1), height: bh, rx: 1.5, class: "bar" + (inBin ? " bar-patient" : "") });
    rect.style.setProperty("--series", "var(--c-" + k + ")");
    const count = g === "cohort" ? cnt(Number(b.n)) : Number(b.n).toLocaleString("en-US");
    rect.appendChild(svg("title", {}, (lo * 100).toFixed(1) + "–" + (Number(b.hi) >= 1 ? "100" : (Number(b.hi) * 100).toFixed(1)) + "%: " + count));
    host.appendChild(rect);
  });
  if (value !== null && Number.isFinite(value)) {
    const x = X(value);
    host.appendChild(svg("line", { x1: x, x2: x, y1: m.t - 4, y2: m.t + ih, class: "marker" }));
    const right = x > w - 120;
    host.appendChild(svg("text", { x: right ? x - 6 : x + 6, y: m.t + 6, class: "marker-label", "text-anchor": right ? "end" : "start" }, "Patient " + pct(value) + (value > xmax ? " (off scale)" : "")));
  }
}

function legend(k         , g       )              {
  const lg = make("div", "legend"); const { n } = binsFor(k, g);
  const name = g === "cohort" && cohort ? "Cohort \"" + cohort.meta.name + "\"" : groupName(g);
  const a = make("span", "lg-bar", name + ", PREVENT risk (n = " + cnt(n) + ")"); a.style.setProperty("--series", "var(--c-" + k + ")");
  lg.append(a, make("span", "lg-marker", "This patient"));
  return lg;
}

function renderMini() {
  const r = computeRisks(patient);
  renderHistogram(el               ("miniPosition")       , "total_cvd", r.total_cvd, group(), 120);
  const host = el                ("miniLegend"); host.textContent = ""; host.appendChild(legend("total_cvd", group()));
  el                    ("miniTitle").textContent = "Total CVD: position among " + groupName(group());
}

function renderPosition() {
  const r = computeRisks(patient); const g = group();
  const host = el                ("histograms"); host.textContent = "";
  for (const k of RISKS) {
    const frame = make("div", "chart-frame");
    const head = make("div", "chart-head");
    head.appendChild(make("h3", "", LABEL[k] + ": " + pct(r[k]) + " PREVENT · " + pct(ucsfCalibrated(k, r[k])) + " calibrated (MOCK)"));
    const pc = percentileIn(k, r[k], g);
    head.appendChild(make("p", "subtle", Number.isFinite(pc) ? "Higher than " + Math.round(pc) + "% of " + groupName(g) + "." : "Not computed: complete the inputs within range."));
    frame.appendChild(head);
    const s = document.createElementNS(SVG_NS, "svg")                 ; s.setAttribute("class", "chart"); s.setAttribute("role", "img");
    s.setAttribute("aria-label", LABEL[k] + " risk distribution of " + groupName(g) + " with the patient marked");
    frame.appendChild(s); frame.appendChild(legend(k, g)); host.appendChild(frame);
    renderHistogram(s, k, r[k], g, 150);
  }
}

// ── what if (one patient) ───────────────────────────────────────────────────
function describe(s               , base               )         {
  const out           = [];
  for (const f of FIELDS) {
    if (!f.modifiable) continue;
    const a = (base       )[f.key], b = (s       )[f.key];
    if (f.kind === "toggle") { if (flag(a) !== flag(b)) out.push(f.short + ": " + (flag(b) ? "yes" : "no")); }
    else if (Number(a) !== Number(b)) out.push(f.short + " " + a + "→" + b);
  }
  return out.length ? out.join(", ") : "No change";
}

function renderScenarioControls() {
  const host = el                ("scenarioControls"); host.textContent = "";
  for (const f of FIELDS) {
    if (!f.modifiable) continue;
    const wrap = make("label", "br-field field" + (f.kind === "toggle" ? " field-toggle" : ""));
    wrap.appendChild(make("span", "br-label", f.unit ? f.label + " (" + f.unit + ")" : f.label));
    const now = (patient       )[f.key];
    if (f.kind === "toggle") {
      const row = make("span", "toggle"); const c = make("input"); c.type = "checkbox"; c.checked = flag((scenario       )[f.key]);
      const t = make("span", "toggle-text", (c.checked ? "Yes" : "No") + " (now " + (flag(now) ? "yes" : "no") + ")");
      c.onchange = () => { (scenario       )[f.key] = c.checked; t.textContent = (c.checked ? "Yes" : "No") + " (now " + (flag(now) ? "yes" : "no") + ")"; renderScenario(); };
      row.append(c, t); wrap.appendChild(row);
    } else {
      const row = make("span", "slider-row");
      const input = make("input", "br-slider"); input.type = "range"; input.min = String(f.min); input.max = String(f.max); input.step = String(f.step);
      input.value = String((scenario       )[f.key]);
      const val = make("span", "slider-val", String((scenario       )[f.key]) + " (now " + now + ")");
      input.oninput = () => { (scenario       )[f.key] = Number(input.value); val.textContent = input.value + " (now " + now + ")"; renderScenario(); };
      row.append(input, val); wrap.appendChild(row);
    }
    host.appendChild(wrap);
  }
  const presets = el                ("presets"); presets.textContent = "";
  for (const p of PRESETS) {
    const b = make("button", "br-btn br-btn--secondary chip-btn", p.label); b.type = "button";
    b.onclick = () => { p.apply(scenario       ); renderScenarioControls(); renderScenario(); };
    presets.appendChild(b);
  }
}

function renderScenario() {
  renderRiskCards(el                ("scenarioRisks"), patient, scenario);
  el                      ("scenarioLabel").textContent = "Scenario: " + describe(scenario, patient);
  renderStatus("whatifStatus", scenario);
  renderSaved();
}

function renderSaved() {
  const table = el                  ("savedTable"); table.textContent = "";
  const cols                                                         = [{ name: "Current", risks: computeRisks(patient), changes: "–" }]
    .concat(scenarios.map((s, i) => ({ name: "Scenario " + (i + 1), risks: s.risks, changes: s.name })));
  const head = make("tr"); head.appendChild(make("th", "", "10-year risk: PREVENT · calibrated (MOCK)")); for (const c of cols) head.appendChild(make("th", "", c.name));
  const thead = make("thead"); thead.appendChild(head); table.appendChild(thead);
  const body = make("tbody");
  for (const k of RISKS) {
    const tr = make("tr"); tr.appendChild(make("td", "", LABEL[k]));
    for (const c of cols) tr.appendChild(make("td", "num", pct(c.risks[k]) + " · " + pct(ucsfCalibrated(k, c.risks[k]))));
    body.appendChild(tr);
  }
  const ch = make("tr", "changes"); ch.appendChild(make("td", "", "Changes")); for (const c of cols) ch.appendChild(make("td", "", c.changes)); body.appendChild(ch);
  table.appendChild(body);
  el                      ("savedHint").textContent = scenarios.length ? "" : "Save up to 3 scenarios to compare them side by side.";
}

// ── projection ──────────────────────────────────────────────────────────────
function renderProjection() {
  const host = el               ("projectionChart")                        ; host.textContent = "";
  const rows = el                         ("projectionRows"); rows.textContent = "";
  const note = el                      ("projectionNote");
  const age = Number(patient.age);
  const pts = Number.isFinite(age) && computeRisks(patient).valid ? projectAges(patient, 79 - Math.ceil(age)) : [];
  if (pts.length < 2) { note.textContent = pts.length === 1 ? "The patient is 79, the upper age limit of PREVENT: no projection." : "Complete the inputs within range to see the projection."; note.hidden = false; return; }
  note.textContent = ""; note.hidden = true;
  const w = Math.max(360, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 860), h = 300;
  host.setAttribute("viewBox", "0 0 " + w + " " + h);
  const m = { l: 44, r: 16, t: 14, b: 34 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const top = Math.max(0.05, Math.ceil(Math.max(...pts.flatMap((p) => [p.total_cvd, p.ascvd, p.hf])) * 1.1 * 20) / 20);
  const a0 = pts[0].age, a1 = pts[pts.length - 1].age;
  const X = (a        ) => m.l + ((a - a0) / Math.max(1, a1 - a0)) * iw;
  const Y = (v        ) => m.t + ih - (v / top) * ih;
  const ystep = top <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= top + 1e-9; t += ystep) {
    host.appendChild(svg("line", { x1: m.l, x2: m.l + iw, y1: Y(t), y2: Y(t), class: "grid" }));
    host.appendChild(svg("text", { x: m.l - 6, y: Y(t) + 4, class: "tick", "text-anchor": "end" }, Math.round(t * 100) + "%"));
  }
  const astep = a1 - a0 > 20 ? 10 : 5;
  for (let a = Math.ceil(a0 / astep) * astep; a <= a1; a += astep) host.appendChild(svg("text", { x: X(a), y: h - 12, class: "tick", "text-anchor": "middle" }, String(a)));
  host.appendChild(svg("text", { x: m.l + iw / 2, y: h - 1, class: "tick", "text-anchor": "middle" }, "Age (years)"));
  for (const k of RISKS) {
    const pre = pts.map((p) => X(p.age) + "," + Y((p       )[k])).join(" ");
    const cal = pts.map((p) => X(p.age) + "," + Y(ucsfCalibrated(k, (p       )[k]) ?? 0)).join(" ");
    for (const [pointsAttr, dashed] of [[pre, false], [cal, true]]                            ) {
      const line = svg("polyline", { points: pointsAttr, class: "series" + (dashed ? " dashed" : "") }); line.style.setProperty("--series", "var(--c-" + k + ")"); host.appendChild(line);
    }
    for (const p of pts) {
      const v = (p       )[k]          ; const dot = svg("circle", { cx: X(p.age), cy: Y(v), r: 6, class: "hit" });
      dot.appendChild(svg("title", {}, "Age " + p.age + ", " + LABEL[k] + ": " + pct(v) + " PREVENT, " + pct(ucsfCalibrated(k, v)) + " calibrated (MOCK)"));
      host.appendChild(dot);
    }
  }
  for (const p of pts) {
    if (p.age !== a0 && p.age !== a1 && (p.age - a0) % 5 !== 0) continue;
    const tr = make("tr"); tr.appendChild(make("td", "", String(p.age) + (p.age === a0 ? " (now)" : "")));
    for (const k of RISKS) tr.appendChild(make("td", "num", pct((p       )[k]) + " · " + pct(ucsfCalibrated(k, (p       )[k]))));
    rows.appendChild(tr);
  }
}

// ── cohort ──────────────────────────────────────────────────────────────────
function loadCohort(c        ) {
  cohort = c; shownRows = 50;
  const opt = el                   ("optCohort"); opt.hidden = false; opt.disabled = false; opt.textContent = "This cohort";
  renderCohort();
}

function tableRows(table                  , head          , rows            , numericFrom = 1) {
  table.textContent = "";
  if (head.some((h) => h)) { const thead = make("thead"); const tr = make("tr"); for (const h of head) tr.appendChild(make("th", "", h)); thead.appendChild(tr); table.appendChild(thead); }
  const body = make("tbody");
  for (const r of rows) { const row = make("tr"); r.forEach((v, i) => row.appendChild(make("td", i >= numericFrom ? "num" : "", v))); body.appendChild(row); }
  table.appendChild(body);
}

function renderCohort() {
  el                ("cohortEmpty").hidden = Boolean(cohort);
  el                ("cohortLoaded").hidden = !cohort;
  if (!cohort) return;
  const c = cohort; const ex = exclusions(c);
  el                    ("cohortName").textContent = c.meta.name;
  el                      ("cohortPurpose").textContent = c.meta.purpose;
  tableRows(el                  ("cohortMeta"), ["", ""], [["Definition", c.meta.definition || "–"], ["As-of date", c.meta.as_of || "–"], ["Source", c.meta.source], ["Received", c.meta.received]], 99);
  const countRows             = [];
  for (const [k, v] of Object.entries(c.meta.upstream)) countRows.push([k.replace(/_/g, " "), typeof v === "number" ? cnt(v) : String(v)]);
  countRows.push(["Members received", cnt(c.members.length)], ["PREVENT computable", cnt(c.ok.length)], ["Excluded: missing input", cnt(ex.missing)], ["Excluded: input outside the PREVENT range", cnt(ex.range)]);
  for (const [k, v] of Object.entries(ex.byInput).sort((a, b) => b[1] - a[1])) { const parts = k.split(" "); const key = parts.pop() || ""; countRows.push(["· " + parts.join(" ") + ": " + fieldLabel(key), cnt(v)]); }
  tableRows(el                  ("cohortCounts"), ["", "n"], countRows);

  // risk summary per outcome
  const grid = el                ("cohortRisk"); grid.textContent = "";
  for (const k of RISKS) {
    const sp = summarize(values(c.ok, k, "prevent")), su = summarize(values(c.ok, k, "ucsf"));
    const card = make("div", "risk-card"); card.style.setProperty("--series", "var(--c-" + k + ")");
    card.appendChild(make("div", "risk-name", LABEL[k])); card.appendChild(make("div", "risk-plain", PLAIN[k]));
    for (const [label, s] of [["PREVENT", sp], ["calibrated (MOCK)", su]]                        ) {
      const row = make("div", "value-row"); row.appendChild(make("span", "value-label", label + ": median (IQR)"));
      const right = make("span", "value-right"); right.appendChild(make("span", "value", pct(s.median)));
      right.appendChild(make("span", "delta", pct(s.q1) + "–" + pct(s.q3))); row.appendChild(right); card.appendChild(row);
    }
    card.appendChild(bandStrip(k, sp.q1, sp.q3, sp.median, su.median));
    card.appendChild(make("div", "risk-pct", "Predicted events in 10 years: " + sp.sum.toFixed(1) + " (PREVENT) · " + su.sum.toFixed(1) + " (calibrated (MOCK)), among " + cnt(sp.n) + " members"));
    grid.appendChild(card);
  }
  renderCohortChart();
  const rows = breakdown(c.ok).map((g) => [g.label, cnt(g.n)].concat(RISKS.map((k) => g.n === 0 || (!showPatients() && g.n <= 10) ? "–" : pct(g.medians[k][0]) + " · " + pct(g.medians[k][1]))));
  tableRows(el                  ("cohortBreakdown"), ["Group", "n", "Total CVD", "ASCVD", "Heart failure"], rows);
  renderCohortRules(); renderCohortScenario(); renderPatientTable();
}

function renderCohortChart() {
  if (!cohort) return;
  const k = el                   ("cohortOutcome").value           ;
  const host = el               ("cohortChart")                        ; host.textContent = "";
  const vals = values(cohort.ok, k, "prevent").sort((a, b) => a - b);
  const w = Math.max(360, host.clientWidth || (host.parentElement ? host.parentElement.clientWidth : 0) || 860), h = 220;
  host.setAttribute("viewBox", "0 0 " + w + " " + h); host.setAttribute("height", String(h));
  const lg = el                ("cohortLegend"); lg.textContent = "";
  if (!vals.length) return;
  const xmax = Math.min(0.6, Math.max(0.1, Math.ceil(quantile(vals, 0.99) * 1.1 * 20) / 20)); const width = 0.01;
  const bins = histogram(vals, width, xmax); const share = bins.map((b) => (b.n / vals.length) * 100);
  // reference: share of reference adults per bin, from the aggregated reference bins (overlap-weighted)
  const ref = ((referenceCohort       ).distribution[k] || {}).overall || []; const nRef = Number((referenceCohort       ).n.overall);
  const refShare = bins.map((b, i) => {
    const lo = b.lo, hi = i === bins.length - 1 ? 1 : b.hi; let s = 0;
    for (const r of ref) { const a = Math.max(lo, Number(r.lo)), z = Math.min(hi, Number(r.hi)); if (z > a) s += Number(r.n) * (z - a) / Math.max(Number(r.hi) - Number(r.lo), 1e-9); }
    return (s / nRef) * 100;
  });
  const peak = Math.max(...share, ...refShare); const ystep = peak > 20 ? 10 : peak > 8 ? 4 : 2; const ymax = Math.ceil((peak * 1.05) / ystep) * ystep;
  const m = { l: 40, r: 14, t: 10, b: 30 }, iw = w - m.l - m.r, ih = h - m.t - m.b;
  const X = (v        ) => m.l + (Math.min(v, xmax) / xmax) * iw; const Y = (v        ) => m.t + ih - (v / ymax) * ih;
  for (let t = 0; t <= ymax; t += ystep) { host.appendChild(svg("line", { x1: m.l, x2: m.l + iw, y1: Y(t), y2: Y(t), class: "grid" })); host.appendChild(svg("text", { x: m.l - 6, y: Y(t) + 4, class: "tick", "text-anchor": "end" }, t + "%")); }
  const xstep = xmax <= 0.2 ? 0.05 : 0.1;
  for (let t = 0; t <= xmax + 1e-9; t += xstep) host.appendChild(svg("text", { x: X(t), y: h - 10, class: "tick", "text-anchor": "middle" }, Math.round(t * 100) + "%"));
  bins.forEach((b, i) => {
    if (!b.n) return;
    const rect = svg("rect", { x: X(b.lo) + 0.5, y: Y(share[i]), width: Math.max(1, X(b.lo + width) - X(b.lo) - 1), height: m.t + ih - Y(share[i]), rx: 1.5, class: "bar" });
    rect.style.setProperty("--series", "var(--c-" + k + ")");
    rect.appendChild(svg("title", {}, (b.lo * 100).toFixed(0) + "–" + (i === bins.length - 1 ? "100" : ((b.lo + width) * 100).toFixed(0)) + "%: " + cnt(b.n) + " members (" + share[i].toFixed(1) + "%); MOCK reference " + refShare[i].toFixed(1) + "%"));
    host.appendChild(rect);
  });
  const step = bins.map((b, i) => X(b.lo) + "," + Y(refShare[i]) + " " + X(b.lo + width) + "," + Y(refShare[i])).join(" ");
  host.appendChild(svg("polyline", { points: step, class: "ref-line" }));
  const a = make("span", "lg-bar", "This cohort, PREVENT risk (n = " + cnt(vals.length) + ")"); a.style.setProperty("--series", "var(--c-" + k + ")");
  lg.append(a, make("span", "lg-ref", "MOCK reference adults, reference (n = " + nRef.toLocaleString("en-US") + ")"));
  const note = make("span", "", "Share of each group per 1-point risk band; last band includes higher risks"); lg.appendChild(note);
}

function chosenRules()           { return RULES.filter((r) => (document.getElementById("rule-" + r.id)                           )?.checked).map((r) => r.id); }

function renderCohortRules() {
  if (!cohort) return;
  const host = el                ("cohortRules"); const keep = chosenRules(); host.textContent = "";
  const counts = cohortScenario(cohort.ok, RULES.map((r) => r.id)).perRule;
  for (const r of RULES) {
    const lab = make("label", "toggle rule"); const c = make("input"); c.type = "checkbox"; c.id = "rule-" + r.id; c.checked = keep.includes(r.id);
    c.onchange = renderCohortScenario;
    lab.append(c, make("span", "", r.label + ": applies to " + cnt(counts[r.id] || 0) + " members (" + r.eligibility + ")"));
    host.appendChild(lab);
  }
}

function renderCohortScenario() {
  if (!cohort) return;
  const ids = chosenRules(); const table = el                  ("cohortScenario"); const note = el                      ("cohortScenarioNote");
  if (!ids.length) { table.textContent = ""; note.textContent = "Choose one or more changes."; return; }
  const s = cohortScenario(cohort.ok, ids);
  const rows = RISKS.map((k) => {
    const o = s.outcomes[k];
    return [LABEL[k], o.sumBefore[0].toFixed(1) + " · " + o.sumBefore[1].toFixed(1), o.sumAfter[0].toFixed(1) + " · " + o.sumAfter[1].toFixed(1),
      pct(o.medianBefore[0]) + " → " + pct(o.medianAfter[0]), pct(o.medianBefore[1]) + " → " + pct(o.medianAfter[1])];
  });
  tableRows(table, ["", "Predicted events now (PREVENT · calibrated)", "With the changes", "Median, PREVENT", "Median, calibrated (MOCK)"], rows);
  const parts = ["Members whose inputs change: " + cnt(s.changed) + " of " + cnt(cohort.ok.length) + "."];
  if (s.higher > 0) parts.push("With these changes PREVENT gives a higher total CVD risk for " + cnt(s.higher) + " of them.");
  if (ids.includes("statin")) parts.push("In PREVENT, statin use describes current treatment: setting it to yes recomputes the equation for a treated person with the same cholesterol; it is not an estimate of the effect of starting a statin.");
  parts.push("Predicted events are the sum of the members' 10-year risks; PREVENT is a prognostic equation, so differences are not treatment effects.");
  note.textContent = parts.join(" ");
}

function memberStatus(m        )         {
  if (m.status === "ok") return "";
  return (m.status === "missing" ? "Missing: " : "Out of range: ") + m.problems.map(fieldLabel).join(", ");
}

function renderPatientTable() {
  const card = el                ("patientListCard"); card.hidden = !cohort || !showPatients();
  if (!cohort || card.hidden) return;
  const sorted = cohort.members.slice().sort((a, b) => ((b.prevent.total_cvd ?? -1) - (a.prevent.total_cvd ?? -1)));
  const table = el                  ("patientTable"); table.textContent = "";
  const thead = make("thead"); const hr = make("tr");
  for (const h of ["Member", "Age", "Sex", "Total CVD (PREVENT · calibrated)", "ASCVD", "Heart failure", "Note"]) hr.appendChild(make("th", "", h));
  thead.appendChild(hr); table.appendChild(thead);
  const body = make("tbody");
  for (const m of sorted.slice(0, shownRows)) {
    const tr = make("tr", "click"); tr.tabIndex = 0;
    const cells = [m.key, String(m.inputs.age ?? "–"), m.inputs.sex === "male" ? "M" : "F"].concat(RISKS.map((k) => m.prevent[k] === null ? "–" : pct(m.prevent[k]) + " · " + pct(m.ucsf[k]))).concat([memberStatus(m)]);
    cells.forEach((v, i) => tr.appendChild(make("td", i >= 3 && i <= 5 ? "num" : "", v)));
    const open = () => openMember(m); tr.onclick = open; tr.onkeydown = (e) => { if (e.key === "Enter") open(); };
    body.appendChild(tr);
  }
  table.appendChild(body);
  const more = el                   ("morePatients"); more.hidden = sorted.length <= shownRows;
  more.textContent = "Show more (" + (sorted.length - shownRows) + " not shown)";
}

function openMember(m        ) {
  if (!cohort) return;
  const p      = { ...m.inputs }; const miss                          = {};
  for (const k of m.problems) if (m.status === "missing") { miss[k] = true; if (k !== "sex") p[k] = typeof p[k] === "boolean" ? false : ""; }
  origin = { name: cohort.meta.name, key: m.key };
  el                   ("sexFilter").value = "cohort";
  newPatient(p, labelAll("Cohort \"" + cohort.meta.name + "\" · " + m.key));
  missing = miss; updateFieldNotes(); renderSourceRows();
}

// ── receiving data from a Biorouter chat (handoff.mjs) ──────────────────────
// The address carries only a pointer to a one-time local WebSocket (allowed by the app's
// content security policy); the data are never in the address or the browser history.
function receive(payload     ) {
  if (!payload || payload.v !== 1) throw new Error("unknown format");
  if (payload.kind === "cohort") {
    const c = buildCohort(payload); loadCohort(c); showTab("cohortview");
    toast("Cohort received: " + c.members.length.toLocaleString("en-US") + " members, PREVENT computed for " + c.ok.length.toLocaleString("en-US") + ".");
  } else if (payload.kind === "patient") {
    const n = normalize(payload.inputs || {}); const src            = labelAll(String(payload.source || "Biorouter chat"));
    if (payload.sources && typeof payload.sources === "object") for (const [k, v] of Object.entries(payload.sources)) src[k] = v       ;
    origin = null; newPatient(n.inputs, src);
    missing = {}; for (const k of n.missing) missing[k] = true; updateFieldNotes(); renderSourceRows();
    toast("Patient received" + (payload.key ? " (" + payload.key + ")" : "") + (n.missing.length ? "; missing: " + n.missing.map(fieldLabel).join(", ") : "") + ".");
  } else throw new Error("unknown kind " + String(payload.kind));
}
function readHandoff() {
  const m = /^#handoff=(ws:\/\/127\.0\.0\.1:\d{2,5}\/[0-9a-f]{32,64})$/.exec(location.hash);
  if (!m) return;
  history.replaceState(null, "", location.pathname + location.search);
  toast("Receiving data from the Biorouter chat…");
  let got = false; const ws = new WebSocket(m[1]);
  ws.onmessage = (ev) => { got = true; try { receive(JSON.parse(String(ev.data))); } catch (e) { toast("Could not read the data: " + String((e       )?.message || e)); } ws.close(); };
  ws.onerror = () => { if (!got) toast("The sender is no longer available: ask the chat to send the data again."); };
}

// ── page ────────────────────────────────────────────────────────────────────
function renderOutputs() {
  renderRiskCards(el                ("riskGrid"), patient);
  renderStatus("inputStatus", patient);
  renderSourceRows();
  renderOrigin();
  renderMini();
  renderPosition();
  renderScenarioControls();
  renderScenario();
  renderProjection();
}
function renderAll() { renderForm(); renderOutputs(); }

const tabs = Array.from(document.querySelectorAll                   ("[data-tab]"));
function showTab(id        ) {
  for (const t of tabs) t.setAttribute("aria-selected", String(t.dataset.tab === id));
  for (const s of Array.from(document.querySelectorAll             (".section"))) s.classList.toggle("active", s.id === id);
  renderOutputs(); if (id === "cohortview") renderCohort();
}
for (const t of tabs) t.onclick = () => showTab(t.dataset.tab || "patient");

function newPatient(p               , src           ) {
  patient = { ...p }; scenario = { ...p }; sources = src; missing = {}; scenarios = [];
  br.state.set("/patient", patient); br.state.set("/scenarios", scenarios);
  renderAll(); showTab("patient");
}

el                   ("sexFilter").onchange = renderOutputs;
el                   ("demoPatient").onclick = () => { origin = null; newPatient(demo, labelAll("Synthetic demo value")); };
el                   ("manualEntry").onclick = () => {
  const blank      = { ...patient };
  for (const f of FIELDS) { if (f.kind === "number") blank[f.key] = ""; if (f.kind === "toggle") blank[f.key] = false; }
  origin = null; newPatient(blank, labelAll("Entered by hand"));
  el                  ("input-age").focus();
};
el                   ("backToCohort").onclick = () => showTab("cohortview");
for (const b of Array.from(document.querySelectorAll                   ("[data-goto]"))) b.onclick = () => { showTab(b.dataset.goto || "about"); window.scrollTo({ top: 0 }); };
el                   ("demoCohort").onclick = () => { loadCohort(buildCohort(demoPayload())); renderCohort(); };
el                   ("demoCohortTop").onclick = () => { loadCohort(buildCohort(demoPayload())); showTab("cohortview"); };
el                  ("showPatients").onchange = () => { renderCohort(); renderOutputs(); };
el                   ("cohortOutcome").onchange = renderCohortChart;
el                   ("morePatients").onclick = () => { shownRows += 100; renderPatientTable(); };
el                   ("resetScenario").onclick = () => { scenario = { ...patient }; renderScenarioControls(); renderScenario(); };
el                   ("saveScenario").onclick = () => {
  if (scenarios.length >= 3) scenarios.shift();
  const saved           = { name: describe(scenario, patient), patient: { ...scenario }, risks: computeRisks(scenario) };
  scenarios.push(saved); br.state.set("/scenarios", scenarios); br.signals.emit("scenario_saved", saved); renderSaved();
};
el                   ("clearScenarios").onclick = () => { scenarios = []; br.state.set("/scenarios", scenarios); renderSaved(); };
el                   ("loadOmop").onclick = async () => {
  const personId = el                  ("personId").value.trim(); const asOf = el                  ("asOfDate").value;
  const msg = el                      ("omopPrivacy");
  if (!personId || !asOf) { msg.textContent = "Enter a person_id and an as-of date. They are sent only to your local app agent."; return; }
  const rules = OMOP_RULES; const outputs = Object.keys(patient);
  const request = { person_id: personId, as_of_date: asOf, omop_rules: rules, required_outputs: outputs };
  msg.textContent = "Loading from OMOP…";
  try { await br.call("load_patient_from_omop", request, { timeoutMs: 180000 }); msg.textContent = "Loaded. Each value shows its source and date."; }
  catch (e) { msg.textContent = "Could not load from OMOP: " + String((e       )?.message || e); }
};
// Cohort selection chat: each message goes to the app's agent; its answer streams into a new bubble.
const COHORT_NOTE = "\n\n(Cohort selection tab. Help define the cohort: purpose, criteria, codes, as-of date. Show the definition and its size from aggregate SQL and wait for confirmation. Aggregates only; counts from 1 to 10 as <11. Do not compute PREVENT: the app computes it. If the OMOP tools are not available in this app session, say so plainly and suggest running the selection in a Biorouter chat with the skill prevent-calculator-handoff.)";
async function sendCohortMessage() {
  const box = el                     ("cohortQuestion"); const q = box.value.trim(); if (!q) return;
  box.value = "";
  const log = el                ("chatLog"); log.hidden = false;
  log.appendChild(make("div", "msg user", q));
  const reply = make("div", "msg agent", "…"); log.appendChild(reply); log.scrollTop = log.scrollHeight;
  try { await br.run(q + COHORT_NOTE, reply, { timeoutMs: 1200000 }); }
  catch (e) { reply.textContent = "The agent could not answer: " + String((e       )?.message || e); }
  log.scrollTop = log.scrollHeight;
}
el                   ("askCohort").onclick = sendCohortMessage;
el                     ("cohortQuestion").onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); sendCohortMessage(); } };
let resizeTimer = 0;
window.addEventListener("resize", () => { window.clearTimeout(resizeTimer); resizeTimer = window.setTimeout(() => { renderOutputs(); renderCohortChart(); }, 150); });
window.addEventListener("hashchange", readHandoff);

br.actions.register("display_loaded_patient", async (args) => {
  const a = (args || {})       ;
  const loaded = a.patient ? { ...patient, ...a.patient } : patient;
  origin = null; newPatient(loaded, a.sources || {});
  missing = a.missing || {}; updateFieldNotes(); renderSourceRows();
  const missingKeys = Object.keys(missing).filter((k) => missing[k]);
  return { ok: true, missing: missingKeys };
});
br.actions.register("display_cohort_result", async (args) => { br.state.set("/cohort", args || null); return { ok: true }; });

el             ("calibrationFacts").textContent = (ucsfRecalibration       ).mock ? "MOCK parameters: illustrative only, not fitted to patient data." : RISKS.map((k) => {
  const cv = (ucsfRecalibration       ).outcomes[k].cross_validated;
  return LABEL[k] + ": observed/expected " + cv.oe_ratio.toFixed(2) + ", calibration slope " + cv.calibration_slope.toFixed(2) + ", C " + cv.c_statistic.toFixed(3);
}).join("; ") + ".";
renderAll();
renderCohort();
readHandoff();

return {  };
})();
})();
