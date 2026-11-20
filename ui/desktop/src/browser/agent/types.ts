/**
 * The shapes that cross the Biorouter Browser's agent channel (BB-10): the daemon's `browser`
 * frame, forwarded by the renderer over preload IPC (`biorouter-browser:agent`), and main's
 * answer, which the renderer sends back as the frame's `workspace_result`.
 *
 * Kept free of Electron so the renderer, main and the Vitest suites can all import it.
 */

export type BrowserAgentOp =
  | 'open'
  | 'navigate'
  | 'tabs'
  | 'snapshot'
  | 'read'
  | 'screenshot'
  | 'scroll'
  | 'hover'
  | 'wait'
  | 'click'
  | 'fill'
  | 'type'
  | 'press'
  | 'select'
  | 'dialog';

export const BROWSER_AGENT_OPS: ReadonlySet<string> = new Set<BrowserAgentOp>([
  'open',
  'navigate',
  'tabs',
  'snapshot',
  'read',
  'screenshot',
  'scroll',
  'hover',
  'wait',
  'click',
  'fill',
  'type',
  'press',
  'select',
  'dialog',
]);

export type BrowserAgentGrant = 'observe' | 'act';
export type BrowserAgentTier = 'public' | 'private';

export type BrowserAgentRequest = {
  /** The daemon's request id, so a `browser_cancel` can name the operation. */
  requestId: string;
  sessionId: string;
  op: BrowserAgentOp;
  args: Record<string, unknown>;
  /** The grant the daemon admitted this call under (BB-17). */
  grant: BrowserAgentGrant;
  /** The call's sampled capability tier (BB-20), never re-read. */
  tier: BrowserAgentTier;
  deadlineMs: number;
  /** Where an agent download is saved, under the act grant only (BB-05). */
  downloadDir?: string;
};

export type BrowserAgentTabInfo = {
  tab: string;
  title: string;
  url: string;
  active: boolean;
  opened_by: 'agent' | 'person';
};

export type BrowserAgentDownload = {
  state: 'completed' | 'cancelled' | 'interrupted' | 'progressing';
  filename: string;
  bytes: number;
  mime: string;
  origin: string;
  path?: string;
  reason?: string;
};

export type BrowserAgentResult = {
  ok: boolean;
  detail: string;
  data?: {
    tab?: string;
    url?: string;
    title?: string;
    navigated?: boolean;
    refs_invalidated?: boolean;
    opened_tabs?: Array<{ tab: string; url: string }>;
    dialog?: { type: string; message: string; default_prompt?: string } | null;
    download?: BrowserAgentDownload | null;
    snapshot?: string;
    content?: string;
    truncated?: boolean;
    screenshot_path?: string;
    tabs?: BrowserAgentTabInfo[];
    person_interacted?: boolean;
    /** A click admitted under observe whose target is not a plain link: nothing happened. */
    needs_act_grant?: boolean;
    error_kind?: string;
  };
};

/** Main tells the chat's panel to show an agent tab (BB-15). */
export type BrowserAgentShowTab = {
  sessionId: string;
  viewId: string;
  url: string;
  title: string;
};

/** Main reports that an agent operation started or ended on a tab, for the bar and outline. */
export type BrowserAgentActivity = {
  sessionId: string;
  viewId: string;
  active: boolean;
};
