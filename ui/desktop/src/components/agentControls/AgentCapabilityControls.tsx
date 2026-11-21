import { BrowserControl } from '../browser/BrowserControl';
import { CopilotControl } from '../copilot/CopilotControl';

/**
 * The agent control slot above the composer (BB-18): Biorouter Copilot's panel and the Biorouter
 * Browser's card, one chat at a time.
 *
 * ⚠ One element in BaseChat, keyed by the chat. The two controls used to be BaseChat's siblings,
 * both keyed `sessionId`; a duplicate key there makes React orphan one control on every re-render
 * that reconciles the slot by key (whenever an earlier sibling is absent), which stacked identical
 * Copilot approval cards whose Cancel no longer re-rendered them (M2 of the 2026-11-20 fix round).
 * Inside this fragment the two have keys of their own.
 */
export function AgentCapabilityControls({ sessionId }: { sessionId: string }) {
  return (
    <>
      <CopilotControl key="copilot" sessionId={sessionId} />
      <BrowserControl key="browser" sessionId={sessionId} />
    </>
  );
}
