// The app's own set, never `lucide-react` directly: `light()` pins §3.8b's one
// stroke weight (1.5) and `currentColor`. Imported raw, these three shipped at
// lucide's default strokeWidth 2 — a heavier, denser mark than every glyph
// beside them, which is exactly the "different icon language" this badge's
// shape-match with `PrivacyBadge` exists to prevent.
import { LandmarkIcon, LaptopIcon, CircleHelpIcon } from '../icons/app-icons';
import { Badge } from './badge';
import { cn } from '../../utils';
import {
  affiliationPresentation,
  type ProviderAffiliation,
  type ProviderAffiliationKind,
} from '../privacy/providerAffiliation';

export interface AffiliationBadgeProps {
  /**
   * DR-26's third axis for the model bound right now, as
   * `readProviderAffiliation` parsed it. `null`/`undefined` renders NOTHING —
   * a public model has no affiliation, and neither does one the daemon could
   * not resolve.
   */
  affiliation: ProviderAffiliation | null | undefined;
  /**
   * Dense surfaces (the composer chip, a tab strip) where a full pill would
   * crowd the row: render the glyph alone, with the words in its accessible
   * name and title. Mirrors `PrivacyBadge`'s dense dot.
   *
   * ⚠ **The glyph still differs per affiliation.** A single shared dot would say
   * "this chat has an institution" and nothing more, which is the one thing the
   * user already knows; the three affiliations differ in *reach*, and the dense
   * form has to keep them distinguishable or it is decoration.
   */
  dense?: boolean;
  className?: string;
}

/**
 * How each affiliation is marked, as a total map — the same construction, and
 * for the same reason, as `PrivacyBadge`'s `TIER`. A fourth affiliation on the
 * wire is a COMPILE ERROR here rather than a silently-unmarked chip.
 *
 * ⚠ **The glyphs are chosen so `local` cannot be misread as a narrower
 * institution.** A landmark on a local model would put it in the same visual
 * family as UCSF and invite the reading "an institution, but smaller" — the
 * exact inversion DR-26 warns about, since `Local` reaches *every* private
 * extension. A laptop says where the work happens, which is the true statement
 * and the one that does not sort itself into the institution list.
 */
const MARK: Record<ProviderAffiliationKind, { Glyph: typeof LandmarkIcon; ariaPrefix: string }> = {
  local: { Glyph: LaptopIcon, ariaPrefix: 'Model affiliation' },
  institutions: { Glyph: LandmarkIcon, ariaPrefix: 'Model affiliation' },
  unstated: { Glyph: CircleHelpIcon, ariaPrefix: 'Model affiliation' },
};

/**
 * The affiliation indicator (issue #56, DR-26) — the second of the two privacy
 * axes, rendered beside the first.
 *
 * ⚠ **It is deliberately the same SHAPE as `PrivacyBadge`**: the same `Badge`
 * primitive, so the same radius/padding/type scale; the same
 * `bg-background-muted` fill, which is contrast for the ink rather than the
 * signal; a glyph plus a word rather than a colour. The two axes are different
 * questions about one chat, and a user has to read them as one system — an
 * affiliation chip in a different visual language would read as a third,
 * unrelated status.
 *
 * ⚠ **What it never does is imply an ORDER between the three affiliations.**
 * There is no ranking: `local` is the most permissive (it reaches everything
 * private, because nothing leaves the machine), `unstated` the least (it reaches
 * only extensions claiming no institution), and an institution sits between them
 * *for that institution's data only*. No tone, weight or colour is varied across
 * the three, because any variation would be read as a scale, and the scale
 * people would infer — "more specific means more restricted" — is backwards.
 *
 * ⚠ **Never a control.** Nothing here is sent back to the daemon: the grant
 * route reads the institution from its own sample, and a client-supplied one
 * would let a caller record an acceptance for a triple the user was never shown
 * (`utils/crossAffiliation.ts`). This component renders and does nothing else.
 */
export function AffiliationBadge({ affiliation, dense = false, className }: AffiliationBadgeProps) {
  const words = affiliationPresentation(affiliation);
  // Both guards, because `affiliationPresentation` narrowing does not narrow
  // `affiliation` for TypeScript and the map lookup below needs the kind.
  if (!affiliation || !words) return null;

  const { Glyph } = MARK[affiliation.kind];
  const accessibleName = `${MARK[affiliation.kind].ariaPrefix}: ${words.label}`;

  if (dense) {
    return (
      <span
        data-testid="affiliation-badge"
        data-affiliation={affiliation.kind}
        // `role="img"` and an explicit label: a bare span maps to role `generic`,
        // which takes no accessible name, so `title` alone would leave the glyph
        // silent to a screen reader — the same trap `PrivacyBadge`'s dense dot
        // documents.
        role="img"
        aria-label={accessibleName}
        title={`${words.label}. ${words.title}`}
        // `inline-flex` + `shrink-0` for `PrivacyBadge`'s dense-dot reasons: a
        // sized indicator inside a tight flex row is the first thing squeezed
        // away, and an indicator whose whole job is to be seen cannot delegate
        // being visible to its callers.
        className={cn('inline-flex shrink-0 items-center text-text-default', className)}
      >
        <Glyph className="h-3 w-3" aria-hidden="true" />
      </span>
    );
  }

  return (
    <Badge
      data-testid="affiliation-badge"
      data-affiliation={affiliation.kind}
      aria-label={accessibleName}
      title={words.title}
      // `.br-privacy-pill`: the shared rows step it to the strong fill on a washed list row,
      // with the PrivacyBadge beside it (main.css, the rows block).
      className={cn('br-privacy-pill bg-background-muted text-text-default', className)}
    >
      <Glyph className="h-3 w-3 shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate">{words.label}</span>
    </Badge>
  );
}
