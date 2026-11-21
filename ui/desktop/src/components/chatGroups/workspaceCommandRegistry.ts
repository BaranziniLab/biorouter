/**
 * The daemon→ChatGroups hand-off for BR-71 workspace command frames.
 *
 * The same seam, and deliberately the same shape, as newTabRegistry /
 * closeActiveTabRegistry — for the same reason: frames arrive at the app root
 * (the workspace WebSocket lives beside ChatGroupsProvider, but a frame can
 * arrive while the user is on Settings, where no provider is mounted). A live
 * provider registers a claim; frames with no claimant are QUEUED, not dropped,
 * and the next mounting provider drains them — the workspace analogue of
 * pendingNewTab (issue #38 taught us the redirect-vs-commit race; consume-once
 * protects against StrictMode double-mounts).
 */
export type WorkspaceCommand = {
  type: 'workspace';
  cmd:
    | 'open_tab'
    | 'activate_tab'
    | 'close_tab'
    | 'open_window'
    | 'notify'
    | 'annotate_tab'
    | 'observe'
    | 'read_panel'
    | 'capture_panel'
    // BB-10: one Biorouter Browser operation, and the daemon abandoning one.
    | 'browser'
    | 'browser_cancel';
  session_id?: string;
  placement?: 'tab' | 'split' | 'window';
  focus?: boolean;
  level?: string;
  message?: string;
  badge?: string;
  parent_session_id?: string;
  request_id?: string;
  /** `read_panel`: how much text to return. */
  max_chars?: number;
  /** `browser`: the operation, its arguments, and what the daemon admitted it under. */
  op?: string;
  args?: Record<string, unknown>;
  grant?: string;
  tier?: string;
  deadline_ms?: number;
  download_dir?: string;
  /** `browser_cancel`: the request id of the operation to abandon. */
  cancel_request_id?: string;
};

/**
 * `data` carries anything richer than a sentence — a panel's text, a capture's
 * path.
 *
 * The daemon side needed no change to accept it: `apply_inbound_frame` resolves
 * the parked tool call with the **whole** frame, so extra fields have always
 * survived; the renderer simply never sent any.
 */
export type WorkspaceCommandResult = {
  ok: boolean;
  detail?: string;
  data?: Record<string, unknown>;
};

/**
 * Handlers may be async.
 *
 * They could not be before, and that was the blocker for anything involving a
 * capture: `capturePage` is inherently asynchronous, as is reading a live
 * page's text out of a separate `WebContents`.
 */
export type WorkspaceCommandHandler = (
  cmd: WorkspaceCommand
) => WorkspaceCommandResult | Promise<WorkspaceCommandResult>;

let handler: WorkspaceCommandHandler | null = null;
let pending: WorkspaceCommand[] = [];

export function registerWorkspaceCommands(next: WorkspaceCommandHandler): () => void {
  handler = next;
  return () => {
    if (handler === next) handler = null;
  };
}

/** A browser operation is never queued: replayed later it would act on a page nobody watched. */
export const BROWSER_CHAT_NOT_ON_SCREEN =
  "Biorouter's browser runs in this chat's side panel, and the chat is not on screen. Ask the person to open the chat.";

/** Apply now if a provider is mounted; otherwise queue and report deferral. */
export function applyWorkspaceCommand(
  cmd: WorkspaceCommand
): WorkspaceCommandResult | Promise<WorkspaceCommandResult> {
  if (handler) return handler(cmd);
  // BB-15: no chat surface is mounted (Settings is in front), so the chat is not on screen.
  // Queueing would replay the operation, possibly a click, whenever a chat next mounts.
  if (cmd.cmd === 'browser' || cmd.cmd === 'browser_cancel') {
    return {
      ok: false,
      detail: BROWSER_CHAT_NOT_ON_SCREEN,
      data: { error_kind: 'chat_not_on_screen' },
    };
  }
  pending.push(cmd);
  return { ok: false, detail: 'no chat surface mounted; queued' };
}

/** Consume-once drain, by the provider on mount. */
export function drainPendingWorkspaceCommands(): WorkspaceCommand[] {
  const drained = pending;
  pending = [];
  return drained;
}

export function hasPendingWorkspaceCommands(): boolean {
  return pending.length > 0;
}

/** Tests only — the singleton must not leak across cases. */
export function resetWorkspaceCommandRegistry(): void {
  handler = null;
  pending = [];
}
