/**
 * BR-71 §5: provenance is structural — any message injected across sessions is
 * permanently labeled in the transcript. Renders nothing for ordinary
 * same-session messages. It is the neutral Badge Crew uses, in sentence case,
 * and its full source (with the session id the label may not carry) is the
 * badge's tooltip: the badge takes focus, so hover and Tab both reach it, and a
 * screen reader reads the unclipped label from the badge itself. Never a
 * `title`, which the app's tooltip layer would turn into the badge's name.
 */
import type { ProvenanceKind } from '../api';
import { Badge } from './ui/badge';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';

export type MessageProvenanceView = {
  // Pinned to the generated enum rather than re-spelled here: the Rust
  // `ProvenanceKind` match in `agents/agent.rs` is deliberately exhaustive, and
  // this is its mirror. Adding a fourth variant on the backend regenerates
  // `ProvenanceKind`, which then fails `labelFor`'s `never` check below instead
  // of silently rendering the last arm's label for a kind nobody wrote copy for.
  kind: ProvenanceKind;
  // Nullable, not merely optional: the generated `MessageProvenance` declares
  // these as `string | null` (serde `Option<String>` → `| null`), so a view type
  // with `?: string` would not accept `message.metadata.provenance` directly.
  fromSessionId?: string | null;
  fromSessionName?: string | null;
};

function labelFor(provenance: MessageProvenanceView): string {
  switch (provenance.kind) {
    case 'agent_injection':
      return `Injected by ${provenance.fromSessionName ?? provenance.fromSessionId ?? 'another agent'}`;
    case 'user_direct':
      return 'Direct user message';
    case 'spawn_context':
      return 'Spawn context';
    default: {
      // Unreachable while the switch stays exhaustive — this line is the type
      // error a new variant trips. It deliberately does not throw: a transcript
      // that crashes is worse than one labeled generically, should a session
      // persisted by a newer daemon ever be replayed here.
      const unhandled: never = provenance.kind;
      return `Cross-session message (${String(unhandled)})`;
    }
  }
}

export function ProvenanceChip({ provenance }: { provenance?: MessageProvenanceView }) {
  if (!provenance) return null;
  const label = labelFor(provenance);
  // The session name comes from another session, so its length is not ours to
  // assume: cap the badge and clip the label rather than let it widen the row.
  // The full text (plus the id, which the label may not carry) is the badge's
  // tooltip, so nothing is lost to the clip.
  const full = provenance.fromSessionId ? `${label} (${provenance.fromSessionId})` : label;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge
          tone="neutral"
          tabIndex={0}
          className="min-w-0 max-w-xs"
          data-provenance-kind={provenance.kind}
        >
          <span className="min-w-0 truncate">{label}</span>
        </Badge>
      </TooltipTrigger>
      <TooltipContent>{full}</TooltipContent>
    </Tooltip>
  );
}
