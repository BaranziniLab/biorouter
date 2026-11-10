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
import { extensionDisplayName } from '../settings/extensions/extensionDisplayName';
import { useOptionalKnowledge } from '../knowledge/KnowledgeContext';
import { knowledgeBaseWords } from '../summary/SubagentPeek';
import { unwrapGuardrailFrame } from '../../utils/guardrailFrame';
import {
  SUBAGENT_STOP_NEEDS_DESKTOP,
  SUBAGENT_STOP_NEEDS_DESKTOP_HELP,
  subagentTabReadOnlyReason,
} from './subagentReadOnly';
import { useParentChatName } from './useParentChatName';

const copy = transcriptCopy.subagent;

/** The header's own words (F-15, C-3): what replaced the visible counts and "Context". */
export const SUBAGENT_HEADER_COPY = {
  /** The one InfoTip that names what the child was given. */
  whatItCanUse: 'What it can use',
  /** The disclosure that holds the spawn record (it read "Context"). */
  instructions: 'Instructions',
  extensions: (names: string) => `Extensions: ${names}.`,
  knowledgeBase: (names: string) => `Knowledge base: ${names}.`,
  knowledgeBases: (names: string) => `Knowledge bases: ${names}.`,
};

/** At most this many names per list in the InfoTip, then "and N more". */
const GRANT_NAME_LIMIT = 3;

/** `Developer, Knowledge, Biorouter Copilot and 11 more`. */
function grantNames(names: readonly string[]): string {
  const shown = names.slice(0, GRANT_NAME_LIMIT);
  const more = names.length - shown.length;
  if (more > 0) return `${shown.join(', ')} and ${more} more`;
  if (shown.length <= 1) return shown.join('');
  return `${shown.slice(0, -1).join(', ')} and ${shown[shown.length - 1]}`;
}

/**
 * What the child can use, in at most two sentences of display names (R6): an
 * empty list's sentence is left out, and `null` when both are empty, so no
 * InfoTip is drawn.
 */
export function grantsHelp(extensions: readonly string[], knowledgeBases: readonly string[]) {
  const parts: string[] = [];
  if (extensions.length > 0) parts.push(SUBAGENT_HEADER_COPY.extensions(grantNames(extensions)));
  if (knowledgeBases.length > 0) {
    const names = grantNames(knowledgeBases);
    parts.push(
      knowledgeBases.length === 1
        ? SUBAGENT_HEADER_COPY.knowledgeBase(names)
        : SUBAGENT_HEADER_COPY.knowledgeBases(names)
    );
  }
  return parts.length > 0 ? parts.join(' ') : null;
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
  // Names a person reads (R6): extensions by display name, knowledge bases by the
  // knowledge list's names, an id it does not list read as words.
  const knowledge = useOptionalKnowledge();
  const help = grantsHelp(
    extensions.map((name) => extensionDisplayName(name)),
    knowledgeBases.map(
      (id) => knowledge?.bases.find((base) => base.id === id)?.name || knowledgeBaseWords(id)
    )
  );
  const stopUnavailable = subagentTabReadOnlyReason();
  // ⚠ The parent is named by its NAME, never its id (principle 10, vision
  // V2-R2-2): the line read "Subagent of 20260828_5" because no caller passed
  // a name and the link fell back to the id. A caller may still pass one; when
  // none comes, the line looks it up itself. Until the lookup answers, the
  // link waits rather than showing a stand-in that then changes under the
  // reader; when it answers without a name, the link says "another chat". The
  // id stays reachable in the link's tooltip.
  const lookup = useParentChatName(parentSessionName ? undefined : parentSessionId);
  const parentLabel =
    parentSessionName ?? lookup.name ?? (lookup.settled ? copy.parentFallback : undefined);
  return (
    <Collapsible
      open={expanded && !!spawnContext}
      onOpenChange={setExpanded}
      // The transcript column's own inset (the scroll area's 24px), so the line
      // starts where the chat's words start. ⚠ This is half of a pair: BaseChat
      // wraps this component in `data-preview-area="subheader"` with the scroll
      // root's `px-1`, and the two together are the transcript's 28px a side.
      // Change this padding and that wrapper together (send WS-SUMMARY the
      // diff), or the line and the column part again (vision V2-R2-3).
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
         * No counts and no names on the line (F-15): what the child can use is
         * behind one "What it can use" InfoTip beside the parent link. A list
         * of one chip per grant made a subagent tab grow taller with every
         * grant, and the counts that replaced it were still more than the line
         * needed.
         */
        className="mx-auto flex h-8 w-full max-w-measure-chat min-w-0 items-center gap-3 text-secondary text-text-muted"
        data-testid={`subagent-header-${sessionId}`}
      >
        {/* The tab strip already badges this tab as a subagent; this line says
            whose. The parent's name is the link back to it. Its id is not shown
            here (R6, R9): the tab's context menu has "Copy chat ID". */}
        {parentLabel !== undefined && (
          <span className="flex min-w-0 items-center gap-1">
            <span className="flex-none">{copy.of}</span>
            <Button
              type="button"
              variant="link"
              className="h-auto min-w-0 truncate px-0 text-secondary"
              onClick={onOpenParent}
            >
              {parentLabel}
            </Button>
          </span>
        )}
        {/* What it was given, behind one InfoTip (F-15): the visible counts
            made the line busier than the tab it heads. */}
        {help && (
          <InfoTip
            label={SUBAGENT_HEADER_COPY.whatItCanUse}
            help={help}
            data-testid={`subagent-grants-${sessionId}`}
          />
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
              {SUBAGENT_HEADER_COPY.instructions}
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
          {/* In sans (R9): the record is instructions for a person to read, not
              code. The same well, height cap and scroll as before. */}
          <div className="mx-auto mb-2 max-h-64 w-full max-w-measure-chat overflow-auto whitespace-pre-wrap break-words rounded-element bg-background-well p-3 text-secondary text-text-default">
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
          </div>
        </CollapsibleContent>
      )}
    </Collapsible>
  );
}
