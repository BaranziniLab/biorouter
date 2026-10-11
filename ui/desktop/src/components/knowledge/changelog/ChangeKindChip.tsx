import type { ComponentType } from 'react';
import type { LucideProps } from 'lucide-react';
import type { ChangeKind } from '../../../api/types.gen';
import {
  ClipboardList,
  Flag,
  Link as LinkIcon,
  MessageSquare,
  Pencil,
  RefreshCw,
  Sparkles,
} from '../../icons/app-icons';
import { Badge, type BadgeTone } from '../../ui/badge';
import { knowledgeCopy } from '../copy';

/**
 * What KIND of change an entry is (ui-spec §4.10).
 *
 * ⚠ **Status hues are not a taxonomy.** This used to paint `flag` danger-red,
 * `query` success-green, `lint`/`restore` warning-amber and `ingest`/`link`
 * info-blue — so a routine "queried the base" entry read as a success and a
 * routine "linted" entry read as a warning, in a log where nothing had gone
 * wrong. The six tones are SEMANTIC: accent, info, success, warning and danger
 * mean something about state, and "this commit came from an ingest" is none of
 * them.
 *
 * All seven kinds are therefore `tone="neutral"` with a 14px leading glyph — the
 * glyph is what distinguishes them, and it survives monochrome. `flag` keeps
 * `danger`, and only `flag`, because a flag genuinely IS a problem marker.
 *
 * The words are sentence case (spec 3.12), and `lint` draws `ClipboardList`, the
 * glyph of the band's "Check for problems" action that makes those commits: the
 * wrench now means "a tool we cannot name" app-wide (spec 3.3).
 */
const GLYPH: Record<ChangeKind, ComponentType<LucideProps>> = {
  ingest: Sparkles,
  link: LinkIcon,
  flag: Flag,
  query: MessageSquare,
  lint: ClipboardList,
  restore: RefreshCw,
  manual: Pencil,
};

const TONE: Record<ChangeKind, BadgeTone> = {
  ingest: 'neutral',
  link: 'neutral',
  flag: 'danger',
  query: 'neutral',
  lint: 'neutral',
  restore: 'neutral',
  manual: 'neutral',
};

export function ChangeKindChip({ kind }: { kind: ChangeKind }) {
  const Glyph = GLYPH[kind];
  return (
    // `br-wash-chip`: the neutral chip steps back to the page ground while its row is washed
    // (main.css, "washed-row chips"; V3-R2-5). The danger tone is a tint and keeps its own.
    <Badge
      data-testid="change-kind-chip"
      data-kind={kind}
      tone={TONE[kind]}
      className={TONE[kind] === 'neutral' ? 'br-wash-chip' : undefined}
    >
      <Glyph className="h-icon-chip w-icon-chip" aria-hidden="true" />
      {knowledgeCopy.changeKind[kind]}
    </Badge>
  );
}
