/**
 * BR-71 §4.5: the glass-box line on a subagent's tab. Shows the spawned-by
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
 * than after it. The line therefore puts a short state where the button would
 * be, with the reason in an InfoTip, as `ToolCallConfirmation` does for Allow
 * and Deny. It asks
 * the surface itself rather than taking a prop: every header this component
 * draws is a subagent's, so there is no chat it could be wrong about, and no
 * caller that could forget to pass it.
 */
import { useState } from 'react';
import { Button } from '../ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { InfoTip } from '../ui/info-tip';
import { ChevronRight } from '../icons/app-icons';
import { transcriptCopy } from '../transcript/copy';
import { unwrapGuardrailFrame } from '../../utils/guardrailFrame';
import {
  SUBAGENT_STOP_NEEDS_DESKTOP,
  SUBAGENT_STOP_NEEDS_DESKTOP_HELP,
  subagentTabReadOnlyReason,
} from './subagentReadOnly';

const copy = transcriptCopy.subagent;

/**
 * A grant count ("2 extensions") whose names are its description: an InfoTip
 * in its `asChild` form makes the count itself the focusable trigger, so the
 * names open on hover, on Tab and on click, and a screen reader hears them on
 * reaching the count. Never `title=`, which would rename it.
 */
function GrantCount({ label, names }: { label: string; names: string[] }) {
  return (
    <InfoTip label={label} help={names.join(', ')} asChild>
      <span className="flex-none rounded-inner text-supporting text-text-muted tabular-nums">
        {label}
      </span>
    </InfoTip>
  );
}

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
    <Collapsible
      open={expanded && !!spawnContext}
      onOpenChange={setExpanded}
      // The transcript column's own inset (the scroll area's 24px), so the line
      // starts where the chat's words start.
      className="px-6"
    >
      <div
        /*
         * ⚠ One 32px line at the top of the column, on the chat's own ground
         * (spec 3.6, J1). It used to be a second 44px band (`h-chrome
         * bg-sidebar border-b`) under the chat header: 88px of chrome before the
         * first message, and a third ground beside the header's and the
         * transcript's.
         *
         * Counts, not names: the names are behind each count's InfoTip, the
         * same trade the composer's chips make (`ChatInput.tsx` records the
         * decision). A list of one chip per grant made a subagent tab grow
         * taller with every grant.
         */
        className="mx-auto flex h-8 w-full max-w-measure-chat min-w-0 items-center gap-3 text-secondary text-text-muted"
        data-testid={`subagent-header-${sessionId}`}
      >
        {/* The tab strip already badges this tab as a subagent; this line says
            whose. The parent's name is the link back to it. */}
        <span className="flex min-w-0 items-center gap-1">
          <span className="flex-none">{copy.of}</span>
          <Button
            type="button"
            variant="link"
            className="h-auto min-w-0 truncate px-0 text-secondary"
            onClick={onOpenParent}
          >
            {parentSessionName ?? parentSessionId}
          </Button>
        </span>
        {extensions.length > 0 && (
          <GrantCount label={copy.extensions(extensions.length)} names={extensions} />
        )}
        {knowledgeBases.length > 0 && (
          <GrantCount label={copy.knowledgeBases(knowledgeBases.length)} names={knowledgeBases} />
        )}
        {/* Only offered when there is something to disclose. The backend's
          `persist_spawn_context` is best-effort (a failure only warns) and
          sessions older than it have no record, so `spawnContext` really can
          be absent while the line itself is still worth showing, and a toggle
          that can only ever open onto nothing is a dead control. It is the
          shared Disclosure's trigger (ghost sm, a chevron that turns 90°), at
          the line's 13px like the parent link and Stop: a dense control in a
          32px strip (spec 2.1), so the line reads at one size. It takes the
          trigger's -12px start too, so its chevron sits one gap after the count
          before it rather than a gap plus the button's padding. */}
        {spawnContext && (
          <CollapsibleTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="biorouter-disclosure-trigger flex-none text-secondary text-text-muted"
              aria-controls={spawnContextId}
            >
              <ChevronRight aria-hidden="true" className="biorouter-disclosure-chevron" />
              {copy.context}
            </Button>
          </CollapsibleTrigger>
        )}
        {running &&
          (stopUnavailable ? (
            // Text, not a disabled button: a disabled control hides its reason
            // behind a hover it may never get. The reason is in the InfoTip
            // beside it, and in full in the note that takes the composer's place.
            <span className="ml-auto flex flex-none items-center gap-1">
              <span
                className="text-supporting text-text-muted"
                data-testid="subagent-stop-unavailable"
              >
                {SUBAGENT_STOP_NEEDS_DESKTOP}
              </span>
              <InfoTip
                label={SUBAGENT_STOP_NEEDS_DESKTOP}
                help={SUBAGENT_STOP_NEEDS_DESKTOP_HELP}
              />
            </span>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="ml-auto flex-none text-secondary"
              onClick={onStop}
              aria-label={copy.stopLabel}
            >
              {copy.stop}
            </Button>
          ))}
      </div>
      {/* Below the line, not inside it: the line stays 32px whether this is
          open or shut. The shared disclosure height animation, one well. */}
      {spawnContext && (
        <CollapsibleContent className="biorouter-disclosure-panel" id={spawnContextId}>
          <pre className="mx-auto mb-2 max-h-64 w-full max-w-measure-chat overflow-auto whitespace-pre-wrap break-words rounded-element bg-background-well p-3 font-mono text-code text-text-default">
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
        </CollapsibleContent>
      )}
    </Collapsible>
  );
}
