// The app's own set, never `lucide-react` directly — see `AffiliationBadge`'s
// import for the defect this closes. The two badges are deliberately one
// system, so they must also draw from one icon set at one stroke weight.
//
// ⚠ **A padlock, and the SAME padlock the chat list draws.** This was a shield
// in the pill and a filled dot in the dense form, which meant the app marked
// Private with three unrelated figures at once: a padlocked speech bubble on a
// private chat (now the lock badge every private chat glyph wears,
// `withPrivateBadge` in `icons/app-icons.tsx`), a shield on a
// private extension, and an anonymous dot on a private model. They are one
// concept — the issue-#56 tier that decides where a transcript may go — so they
// are now one mark, and the two forms below differ only in whether the word
// fits beside it.
import { Lock } from '../icons/app-icons';
import { Badge } from './badge';
import { cn } from '../../utils';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import type { SessionClassification } from '../../api';

export interface PrivacyBadgeProps {
  /**
   * A tier — the generated union the daemon actually sends.
   *
   * ⚠ **Whose** tier is the call site's to know, and the two subjects disagree
   * routinely: a session's ratcheted classification (`SessionNamePill`, the
   * History rows) and a bound model's instance-resolved tier (`ProviderTier`,
   * on the model chip) are the same string union and mean different things. A
   * fresh chat is classified `public` until its first turn, so a model chip
   * that borrowed the chat's value drew a public dot next to a private UCSF
   * model. Structurally identical, semantically not — this badge draws what it
   * is handed and cannot tell them apart, so the caller must.
   */
  tier: SessionClassification;
  /**
   * Dense surfaces — today the composer's model chip — where a full pill would
   * crowd the row: render the padlock alone and nothing at all for Public.
   *
   * ⚠ It used to be a plain dot, and the dot is what made this form unreadable
   * on the one surface that still uses it. The chat lists that the doc-comment
   * here once named moved to `ChatKindIcon`, which folds the tier into a
   * padlocked bubble; what was left beside a model name was a bare mark with no
   * figure to connect it to any of that. Same glyph as the pill, same glyph as
   * the chat list — only the word is dropped.
   */
  dense?: boolean;
  /**
   * The master switch (`BIOROUTER_PRIVACY_TIERS`) is OFF, so nothing enforces
   * this tier (issue #56, DR-15).
   *
   * ⚠ **Leave it undefined and the badge asks.** Every one of this component's
   * nine call sites is a surface — a session row, a tab, a model chip, a KB row
   * — that knows a tier and has no business knowing whether the daemon is
   * enforcing it, and a prop nine callers must remember to pass is a prop that
   * ships unpassed: it did, for one round, and the presentation below was
   * reachable only from its own unit test. So the default comes from
   * `usePrivacyTiersEnabled()`, and this prop exists to OVERRIDE that — which is
   * what the tests below the fold do, and what a surface documenting the two
   * states (a settings preview) would do.
   *
   * The badge stays VISIBLE and says so. The two rejected alternatives, and why:
   *
   * - **Hide it.** It reads as the tidy answer — the feature is off, so its
   *   ornament goes — and it is the worst one. The badge is the only place the
   *   tier is ever stated; removing it makes an *unprotected* machine
   *   indistinguishable from a machine with no private material on it, at
   *   exactly the moment (picking a model, pasting a cohort) when the
   *   distinction matters. The person who needs the reminder is the person who
   *   turned it off and forgot.
   * - **Leave it unchanged.** A pill that reads plain "Private" while nothing
   *   enforces it is not information, it is a false statement, and it is worse
   *   than no badge because the user acts on it.
   *
   * The suffix is on the badge itself rather than only in the settings strip
   * because badges appear on surfaces the strip does not — the session list, the
   * model chip, the extension rows.
   *
   * ⚠ One consequence, stated so it is not rediscovered: a test that
   * `vi.mock`s `ConfigContext` **and** renders a badge must stub
   * `usePrivacyTiersEnabled` too. It fails loudly if it does not — "is not a
   * function", at the line below — which is the right trade against a prop that
   * ships unpassed on nine surfaces and fails nowhere.
   */
  enforcementOff?: boolean;
  className?: string;
}

/**
 * How each tier renders, as a total map rather than a chain of ternaries.
 *
 * `Record<SessionClassification, …>` is the whole point: a third tier on the
 * wire is a COMPILE ERROR here, which is what this file used to claim and was
 * not. Widening the generated union to `'public' | 'private' | 'restricted'`
 * typechecked the entire desktop app with zero errors — `tier === 'private' ? a
 * : b` never owes exhaustiveness — and the two branches then disagreed about
 * the new tier: dense mode drew it a Private dot (it is not `'public'`) while
 * the pill labelled it Public (it is not `'private'`). An unrecognised privacy
 * tier rendered as BOTH, and the safer of the two readings was the accident.
 */
const TIER: Record<
  SessionClassification,
  { label: string; ink: string; glyph: boolean; markedWhenDense: boolean }
> = {
  private: {
    label: 'Private',
    ink: 'text-text-default',
    glyph: true,
    markedWhenDense: true,
  },
  public: {
    label: 'Public',
    ink: 'text-text-muted',
    glyph: false,
    markedWhenDense: false,
  },
};

/**
 * The tier indicator (issue #56, R10).
 *
 * Private is the marked state; Public is the quiet state. A badge on absolutely
 * everything trains people to stop seeing badges, which defeats R10's actual
 * goal — knowing which tier you are in BEFORE hitting a wall.
 *
 * Both states use a FILL, not an outline. Measured across all six family × mode
 * scopes with `scripts/lib/theme-tokens.mjs`, against `--background-muted` and
 * `--background-medium`, every NEUTRAL RESTING border token in this system sits
 * between 1.00 and 1.75 — `--border-subtle` 1.00–1.38, `--border-default`
 * 1.00–1.38, `--border-strong` 1.00–1.58, `--border-input` 1.00–1.75 — nowhere
 * near the 3:1 SC 1.4.11 asks of an affordance carrying meaning. A hairline
 * Public pill would ship invisible: parchment:dark measures exactly 1.00, the
 * hairline and its ground being literally the same colour.
 *
 * (Tokens that DO clear 3:1 on those grounds exist — `--border-focus` measures
 * 3.75–5.60, and the status and accent borders 3.55–11.74 — but every one of
 * them already means something: focused, dangerous, successful. Outlining a
 * resting Public chip in one would not say "public".)
 *
 * The chosen pair instead measures 12.91–15.13 (Private) and 5.50–7.26 (Public)
 * and is asserted for every scope in `scripts/check-contrast.mjs`.
 *
 * What the fill is NOT is the carrier of the meaning. `--background-muted`
 * equals `--background-canvas` outright in three scopes and `--sidebar` in
 * parchment:dark, so on those grounds the pill reads as its label rather than as
 * a filled chip. That is why Private is a padlock glyph AND the word, never a
 * colour: the fill is contrast for the ink, not the signal.
 *
 * ⚠ **The padlock is the app's ONE mark for the private tier**, shared with
 * `chatKind.ts`'s padlocked speech bubble on a private conversation. The three
 * subjects this badge is hung on — a conversation, a bound model, an extension
 * — were marked with three different figures (a padlocked bubble, a dot, a
 * shield) for one and the same fact: `ProviderTier`/`SessionClassification`
 * `private`, the issue-#56 tier that decides which models may see the data. A
 * reader had to learn three symbols to read one lattice, and nothing in the
 * three shapes said they were related. Adding a fourth figure for a fourth
 * subject is the failure mode to avoid here; there is one glyph, and its
 * SUBJECT is named in words beside it or in the accessible name.
 *
 * Public gets no glyph, and deliberately no *unlocked* padlock. An open padlock
 * is a mark, and a mark is what Private has to be able to catch the eye with;
 * drawing one on every public row would put the two tiers at the same visual
 * weight and train people past both. Public says its word and nothing more.
 *
 * Geometry comes from `Badge` and only from `Badge` — the radius, padding and
 * type scale are never restated here, which is the drift `badge.tsx`'s own
 * doc-comment exists to prevent.
 */
export function PrivacyBadge({
  tier,
  dense = false,
  enforcementOff,
  className,
}: PrivacyBadgeProps) {
  // Before the early return below, because a hook must be called
  // unconditionally — and `??`, not `||`, so an explicit `enforcementOff={false}`
  // means what it says instead of falling through to the daemon's answer.
  const enforced = usePrivacyTiersEnabled();
  const off = enforcementOff ?? !enforced;
  const spec = TIER[tier];

  if (dense && !spec.markedWhenDense) return null; // no padlock means public

  if (dense) {
    return (
      <span
        data-testid="privacy-badge"
        data-enforcement={off ? 'off' : 'on'}
        // `{tier}`, never the literal `"private"`. The dense branch is reachable
        // only for a tier with `markedWhenDense`, which today is Private alone —
        // so this is not observably different yet, and no test can discriminate
        // it without widening the union. It is written this way because the
        // TIER map above exists precisely so a third tier is a compile error,
        // and a hardcoded value is the one shape that would compile and then
        // silently mislabel the new tier as Private on every dense surface.
        data-privacy={tier}
        // `role` and `title` on the SPAN, not on the glyph. `title` is an HTML
        // global attribute; on an inline `<svg>` it is not the SVG tooltip
        // mechanism (that is a `<title>` CHILD element), so hanging it there
        // would silently drop the hover text in some engines. And a plain span
        // maps to role `generic`, which does not take an accessible name — so
        // `title` by itself would leave the padlock unannounced as well as
        // untooltipped. Both live here; the glyph is decorative.
        role="img"
        aria-label={off ? 'Private chat, enforcement off' : 'Private chat'}
        title={
          off
            ? 'Private, but privacy tiers are turned off, so nothing enforces this'
            : 'Private. Biorouter only lets a private model open this chat.'
        }
        // `inline-block`, not the default `inline`: this wrapper shrink-wraps a
        // glyph that carries its own box, and an inline box would additionally
        // reserve line-height around it. `shrink-0` because dense surfaces are
        // tight by definition and a flex child with no shrink guard is the first
        // thing squeezed away. Both belong here, not in each caller: an
        // indicator whose whole job is to be seen cannot delegate being visible.
        //
        // The ink is the token the pill's Private state uses, so the two forms
        // cannot drift apart in colour any more than they can in shape.
        className={cn('inline-block shrink-0 text-text-default', className)}
      >
        {/* `block`, so the wrapper is exactly 12×12 — an inline svg sits on the
            text baseline and drags a descender's worth of empty space under it,
            which is how a 12px mark ends up nudging a 28px chip's row height.
            `h-3 w-3` is the size `AffiliationBadge`'s dense glyph already uses,
            and the two sit side by side on the composer's model chip. */}
        <Lock className="block h-3 w-3" aria-hidden="true" />
      </span>
    );
  }

  return (
    <Badge
      data-testid="privacy-badge"
      data-privacy={tier}
      data-enforcement={off ? 'off' : 'on'}
      // Muted ink for BOTH tiers when nothing enforces them, and the suffix
      // below carries the meaning. The fill is unchanged: it is contrast for the
      // ink, not the signal (see this component's doc comment), and swapping it
      // for a status colour would say "danger" on a surface where the honest
      // word is "off".
      className={cn('bg-background-muted', off ? 'text-text-muted' : spec.ink, className)}
    >
      {spec.glyph ? <Lock className="h-3 w-3" aria-hidden="true" /> : null}
      {spec.label}
      {off ? <span className="text-text-muted">&nbsp;(enforcement off)</span> : null}
    </Badge>
  );
}
