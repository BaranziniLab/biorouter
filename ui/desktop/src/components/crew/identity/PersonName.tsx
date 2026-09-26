import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { identityCopy } from './copy';
import { personLayout, resolvePerson, type PersonLabelOptions } from './personLabel';
import type { PeopleDirectory } from './usePeopleDirectory';
import type { PersonContext, PersonRef } from './types';

export interface PersonNameProps extends PersonLabelOptions {
  /** The person, or a principal ID to look up in `dir`. An unknown ID renders "Unknown member". */
  person: PersonRef;
  context: PersonContext;
  /** The workspace's people, for looking up an ID and for current collision and former flags. */
  dir?: PeopleDirectory | null;
  /** A chip's `@username` tooltip. Turn it off when the chip already sits inside a tooltip. */
  tooltip?: boolean;
  /** Layout only. */
  className?: string;
}

/**
 * A parenthesis that stays in the text but takes no space on screen. It is an
 * inline run at `font-size: 0`, not `.sr-only`, and that is measured, not
 * taste: `.sr-only` is `position: absolute`, which makes each paren its own box,
 * and Chromium then joins a name computed from contents with spaces — a
 * checkbox labelled by the row read "Bob Lee ( @bob )". An inline zero-size run
 * reads "Bob Lee (@bob)" there, and as `textContent` and a copied selection.
 *
 * `line-height: 0` is as load-bearing as the zero size. The line-height tokens
 * are fixed pixels (`--text-label--line-height: 20px`), not multiples, so a
 * zero-size run would still inherit the parent's full 20px line box, sitting on
 * the baseline: its half-leading reaches ~10px BELOW the baseline, past the
 * text's own descent. Measured in Chrome with the app's Inter, that grew an Add
 * People row from 28px to 33px and put the name 3px above the avatar's centre;
 * with `line-height: 0` the row is 28px again, the offset −0.5px exactly as a
 * header's, and the checkbox's name still "Bob Lee (@bob)". jsdom has no layout,
 * so the test can only pin the declaration.
 *
 * An inline style, because a newly written utility class can fail to generate
 * (CLAUDE.md, "Desktop shell geometry").
 */
function HiddenParen({ paren }: { paren: '(' | ')' }) {
  return (
    <span data-person-part="paren" style={{ fontSize: 0, lineHeight: 0 }}>
      {paren}
    </span>
  );
}

/**
 * The one component that renders a person (ui-redesign-spec, "Identity and
 * naming display rules"). No other code formats a person; for a string (an
 * `aria-label`, a confirmation title, a toast) use `personLabel`, which makes
 * the same decisions.
 *
 * - Every display name sits in its own `<bdi>`, so a right-to-left or
 *   mixed-direction name cannot reorder the text around it.
 * - `@username` is always its own element, never concatenated into the name.
 * - A display name that only repeats the username — case aside, or with the
 *   username's `@` and `#` removed, as an SSSD account's default name is — is
 *   not repeated: the person reads `@username` once, in every context — never
 *   `bob (@bob)`, and never `bobad.ucsf.edu (@bob@ad.ucsf.edu)`.
 * - An unknown principal is "Unknown member"; an ID is never rendered.
 * - A former member is muted and followed by " · former member".
 * - An authority point (`authority`) is DRAWN the way a member row is: the
 *   display name, then `@username` as its own muted element, with no visible
 *   parentheses — "Carol Nguyen @crew_carol" — so one person reads one way in
 *   every list (Q2-70; naming-design: "people list, member rows"). What an
 *   authority point owes is that `@username` is always shown in full, and it
 *   is. Its TEXT keeps "Carol Nguyen (@crew_carol)", the string
 *   `personLabel(…, 'authority')` gives: the parentheses are still in the
 *   tree, drawn at zero size (see `HiddenParen`), so a checkbox or row named
 *   by its contents, a screen reader and a copied selection all read the
 *   canonical form.
 * - An agent at an authority point names its owner in full BEFORE "'s agent"
 *   (spec rule 5): its text is "Alice Chen (@alice)'s agent", as
 *   `agentLabel(…, 'authority')` gives, drawn "Alice Chen @alice's agent" with
 *   the same muted handle and zero-size parentheses. That is the one place an
 *   authority point is not drawn as a header, whose agent is "Alice Chen's
 *   agent @alice": the header's order would make the text read "Alice Chen's
 *   agent (@alice)", which is not the authority form.
 */
export function PersonName({
  person,
  context,
  dir,
  agent,
  you,
  tooltip = true,
  className,
}: PersonNameProps) {
  const layout = personLayout(resolvePerson(person, dir), context, { agent, you });

  if (layout.kind === 'unknown') {
    return (
      <span
        className={cn('text-text-muted', context === 'header' && 'text-label', className)}
        data-person-context={context}
        data-person-state="unknown"
      >
        {layout.agent
          ? identityCopy.agentOf(identityCopy.unknownMember)
          : identityCopy.unknownMember}
      </span>
    );
  }

  if (layout.kind === 'joiner') {
    return (
      <span className={className} data-person-context={context} data-person-state="joiner">
        <bdi className="font-mono" data-person-part="username" translate="no">
          {layout.handle}
        </bdi>
        {layout.serverName && (
          <>
            {identityCopy.separator}
            <span data-person-part="server-name">
              <bdi>{layout.serverName}</bdi> ({identityCopy.serverAccountName})
            </span>
          </>
        )}
      </span>
    );
  }

  // An authority point's `Name (@user)` is drawn as the header's muted handle,
  // its parentheses kept in the text at zero size (see the rule above).
  const drawnParen = context === 'authority' && layout.handlePlacement === 'paren';
  const placement = drawnParen ? 'secondary' : layout.handlePlacement;

  const handle = (secondary: boolean) => (
    <bdi
      className={secondary ? 'text-supporting text-text-muted' : undefined}
      data-person-part="username"
      translate="no"
    >
      {layout.handle}
    </bdi>
  );

  const lead =
    layout.lead === 'your-agent' ? (
      identityCopy.yourAgent
    ) : layout.lead === 'handle' ? (
      handle(false)
    ) : (
      <bdi data-person-part="display-name">{layout.displayName}</bdi>
    );

  const nameGroup = (
    <span className={context === 'header' ? 'text-label' : undefined} data-person-part="name">
      {lead}
      {placement === 'paren' && <> ({handle(false)})</>}
      {layout.agentOf && !drawnParen && identityCopy.agentSuffix}
    </span>
  );

  const body = (
    <span
      className={cn(layout.former && 'text-text-muted', className)}
      data-person-context={context}
      data-person-state={layout.former ? 'former' : 'active'}
    >
      {nameGroup}
      {drawnParen ? (
        <>
          {' '}
          <HiddenParen paren="(" />
          {handle(true)}
          <HiddenParen paren=")" />
          {/* Spec rule 5: the owner's handle comes before "'s agent" here. */}
          {layout.agentOf && identityCopy.agentSuffix}
        </>
      ) : (
        placement === 'secondary' && <> {handle(true)}</>
      )}
      {placement === 'tooltip' && (
        <span className="sr-only">
          {' ('}
          {handle(false)}
          {')'}
        </span>
      )}
      {layout.former && (
        <>
          {identityCopy.separator}
          <span data-person-part="former">{identityCopy.formerMember}</span>
        </>
      )}
      {layout.youSuffix && (
        <>
          {identityCopy.separator}
          <span className="text-text-muted" data-person-part="you">
            {identityCopy.you}
          </span>
        </>
      )}
    </span>
  );

  if (placement !== 'tooltip' || !tooltip) return body;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{body}</TooltipTrigger>
      <TooltipContent>
        <bdi translate="no">{layout.handle}</bdi>
      </TooltipContent>
    </Tooltip>
  );
}
