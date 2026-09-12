/**
 * BR-71 §4.5: the glass-box header on a subagent's tab. Shows the spawned-by
 * link, the child's grants (extensions from GET /sessions/{id}/extensions, KBs
 * from the spawn-context record — both fetched by the mounting container), and the exact spawn
 * context (the provenance:spawn_context first message), expandable. Stop
 * resolves the parent's tool call as Incomplete (the backend path — the button
 * merely posts /agent/cancel via onStop). Closing the tab never kills the
 * child; Stop is the only kill switch here.
 *
 * ⚠ In a browser there is no Stop to offer. The daemon behind `biorouter serve`
 * holds no user-action key, so it refuses `/agent/cancel` for a subagent's chat
 * from every caller (SD-11), and SD-8 wants that said before the click rather
 * than after it. The header therefore puts one line of explanation where the
 * button would be, as `ToolCallConfirmation` does for Allow and Deny. It asks
 * the surface itself rather than taking a prop: every header this component
 * draws is a subagent's, so there is no chat it could be wrong about, and no
 * caller that could forget to pass it.
 */
import { useState } from 'react';
import { Button } from '../ui/button';
import { unwrapGuardrailFrame } from '../../utils/guardrailFrame';
import { SUBAGENT_STOP_NEEDS_DESKTOP, subagentTabReadOnlyReason } from './subagentReadOnly';

export function SubagentTabHeader({
  sessionId,
  parentSessionId,
  parentSessionName,
  spawnContext,
  extensions,
  knowledgeBases,
  running,
  onOpenParent,
  onStop,
}: {
  sessionId: string;
  parentSessionId: string;
  parentSessionName?: string;
  spawnContext?: string;
  extensions: string[];
  knowledgeBases: string[];
  running: boolean;
  onOpenParent: () => void;
  onStop: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const spawnContextId = `subagent-spawn-context-${sessionId}`;
  const stopUnavailable = subagentTabReadOnlyReason();
  return (
    <>
      <div
        /*
         * ⚠ One band's worth of height, on the chat's own ground.
         *
         * This used to be `bg-background-muted` with content-driven height — a
         * THIRD ground beside the header's `bg-sidebar` and the transcript's
         * canvas, growing taller with every grant because the chip row below was
         * an uncapped flex-wrap of one chip per extension and per knowledge base.
         * A subagent tab was visibly taller than an ordinary one, and the taller
         * it got the more it looked like a debug panel rather than a chat.
         *
         * The ordinary chat already settled this question: counts in the
         * composer's row 1, names behind a popover, never a list in a header
         * (`ChatInput.tsx:2668-2670` records the decision). This now follows that
         * convention.
         */
        className="flex h-chrome items-center gap-2 border-b border-border-subtle bg-sidebar px-4 text-secondary"
        data-testid={`subagent-header-${sessionId}`}
      >
        {/* The tab strip already badges this tab as a subagent, so the standalone
          "subagent" pill said the same thing twice on one screen. */}
        <span className="min-w-0 truncate text-text-subtle">
          spawned by{' '}
          <button className="underline" onClick={onOpenParent}>
            {parentSessionName ?? parentSessionId}
          </button>
        </span>
        {/* Counts, not names. The names are one click away and the count is the
          part that survives a glance — the same trade the composer's three
          chips make. `title` carries the full list so it is still reachable
          without opening anything. */}
        {extensions.length > 0 && (
          <span
            className="flex-none rounded bg-background-code px-1.5 py-0.5 text-supporting text-text-subtle"
            title={extensions.join(', ')}
          >
            {extensions.length} extension{extensions.length === 1 ? '' : 's'}
          </span>
        )}
        {knowledgeBases.length > 0 && (
          <span
            className="flex-none rounded bg-background-code px-1.5 py-0.5 text-supporting text-text-subtle"
            title={knowledgeBases.join(', ')}
          >
            {knowledgeBases.length} knowledge base{knowledgeBases.length === 1 ? '' : 's'}
          </span>
        )}
        {/* Only offered when there is something to disclose. The backend's
          `persist_spawn_context` is best-effort (a failure only warns) and
          sessions older than it have no record, so `spawnContext` really can
          be absent while the header itself is still worth showing — and a
          toggle that can only ever open onto nothing is a dead control. */}
        {spawnContext && (
          <button
            className="flex-none underline"
            onClick={() => setExpanded((e) => !e)}
            aria-expanded={expanded}
            aria-controls={spawnContextId}
          >
            spawn context
          </button>
        )}
        {running &&
          (stopUnavailable ? (
            // Text, not a disabled button: a disabled control hides its reason
            // behind a hover it may never get. The full sentence is in `title`,
            // and in the note that takes the composer's place below.
            <span
              className="ml-auto flex-none text-supporting text-text-subtle"
              title={stopUnavailable}
              data-testid="subagent-stop-unavailable"
            >
              {SUBAGENT_STOP_NEEDS_DESKTOP}
            </span>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto flex-none"
              onClick={onStop}
              aria-label="Stop subagent"
            >
              Stop subagent
            </Button>
          ))}
      </div>
      {/* Below the band, not inside it: the band is a fixed `h-chrome` so it
          stays exactly as tall as the ordinary chat header whether this is open
          or shut. */}
      {expanded && spawnContext && (
        <pre
          id={spawnContextId}
          className="max-h-64 overflow-auto whitespace-pre-wrap break-words border-b border-border-subtle bg-background-code p-2 text-supporting"
        >
          {/*
           * ⚠ The last human-facing surface that showed the guardrail's frame
           * raw.
           *
           * `### Task instructions` is free text the PARENT agent wrote, and a
           * parent that quotes what a tool handed it carries
           * `<tool-output untrusted="true" tool="…">` … `</tool-output>` into
           * this record verbatim. Every other panel already unwraps (BaseChat,
           * ToolCallWithResponse); this one printed the delimiter at the reader.
           *
           * Three things this call is careful about:
           *
           * 1. DISPLAY ONLY. `spawnContext` is read off a stored message and
           *    rendered; nothing downstream of here reaches a model. The
           *    `[BIOROUTER GUARDRAIL]` line above a frame is a real warning and
           *    survives, structurally — the helper only rewrites between a
           *    complete opening tag and its matching close.
           * 2. Here and not in `useSubagentSession`, so `extractKnowledgeBases`
           *    keeps parsing the daemon's exact bytes. That parser refuses to
           *    guess when it sees two `### Knowledge bases` headings, which is a
           *    security posture reasoned about against the stored record; it
           *    should not be handed a string some display helper reshaped.
           * 3. `### Rendered system prompt` names the opening tag on its own,
           *    with no close (`subagent_system.md` documents the tag to the
           *    model). A lone tag is left verbatim by design, so the reader
           *    still sees the prompt exactly as the child received it.
           */}
          {unwrapGuardrailFrame(spawnContext)}
        </pre>
      )}
    </>
  );
}
