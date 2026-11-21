import type { BrowserAgentOp, BrowserAgentRequest } from '../../browser/agent/types';
import { BROWSER_AGENT_OPS } from '../../browser/agent/types';
import { leafGroupIds, type ChatGroupsState } from './chatGroupsTypes';
import {
  BROWSER_CHAT_NOT_ON_SCREEN,
  type WorkspaceCommand,
  type WorkspaceCommandResult,
} from './workspaceCommandRegistry';

/**
 * The renderer's half of the Biorouter Browser's agent channel (BB-10, BB-15).
 *
 * The daemon sends one `browser` frame per operation to the window whose layout echo holds the
 * chat. This window checks that the chat is ON SCREEN here (the active tab of one of its
 * groups), and only then forwards the operation to main over preload IPC; main shows the agent
 * tab in the chat's panel and drives it. A chat that is open but behind another tab, or not
 * open here, fails with a sentence the model can act on: the agent drives only what the person
 * can see.
 *
 * A `browser_cancel` frame (the daemon gave up: a timeout, Stop, the turn cancelled) tells main
 * to abandon that operation. Nothing here is ever queued for later.
 */

export const BROWSER_NO_DESKTOP =
  'The Biorouter Browser runs in the Biorouter desktop app, and this window is a web browser. Use webdocuments web_scrape for the plain text of a public page, or tell the person the browser is not available here.';

export async function runBrowserCommand(
  cmd: WorkspaceCommand,
  isOnScreen: (sessionId: string) => boolean
): Promise<WorkspaceCommandResult> {
  const agent = window.electron?.browserAgent;
  if (cmd.cmd === 'browser_cancel') {
    if (agent && cmd.cancel_request_id) await agent.cancel(cmd.cancel_request_id);
    return { ok: true, detail: 'canceled' };
  }
  if (!agent) {
    return { ok: false, detail: BROWSER_NO_DESKTOP, data: { error_kind: 'no_desktop' } };
  }
  const sessionId = cmd.session_id;
  if (!sessionId || !cmd.request_id) {
    return { ok: false, detail: 'the browser operation named no chat' };
  }
  if (!isOnScreen(sessionId)) {
    return {
      ok: false,
      detail: BROWSER_CHAT_NOT_ON_SCREEN,
      data: { error_kind: 'chat_not_on_screen' },
    };
  }
  if (typeof cmd.op !== 'string' || !BROWSER_AGENT_OPS.has(cmd.op)) {
    return { ok: false, detail: `${String(cmd.op)} is not a browser operation` };
  }
  if (cmd.grant !== 'observe' && cmd.grant !== 'act') {
    return { ok: false, detail: 'the browser operation carried no grant' };
  }
  const request: BrowserAgentRequest = {
    requestId: cmd.request_id,
    sessionId,
    op: cmd.op as BrowserAgentOp,
    args: cmd.args && typeof cmd.args === 'object' ? cmd.args : {},
    grant: cmd.grant,
    tier: cmd.tier === 'private' ? 'private' : 'public',
    deadlineMs: Number(cmd.deadline_ms) || 30_000,
    downloadDir: typeof cmd.download_dir === 'string' ? cmd.download_dir : undefined,
  };
  const result = await agent.run(request);
  return {
    ok: result.ok,
    detail: result.detail,
    data: result.data as Record<string, unknown> | undefined,
  };
}

/**
 * Whether a chat is on screen in this window: the active tab of one of its visible groups. A
 * chat behind another tab of its group is open but not on screen (BB-15).
 */
export function isSessionOnScreen(
  state: Pick<ChatGroupsState, 'layout' | 'groups'>,
  sessionId: string
): boolean {
  return leafGroupIds(state.layout).some((groupId) => {
    const group = state.groups[groupId];
    const active = group?.tabs.find((tab) => tab.tabId === group.activeTabId);
    return active?.sessionId === sessionId;
  });
}
