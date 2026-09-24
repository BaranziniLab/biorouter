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
 * The one component that renders a person (ui-redesign-spec, "Identity and
 * naming display rules"). No other code formats a person; for a string (an
 * `aria-label`, a confirmation title, a toast) use `personLabel`, which makes
 * the same decisions.
 *
 * - Every display name sits in its own `<bdi>`, so a right-to-left or
 *   mixed-direction name cannot reorder the text around it.
 * - `@username` is always its own element, never concatenated into the name.
 * - A display name that is only the username (case aside) is not repeated: the
 *   person reads `@username` once, in every context — never `bob (@bob)`.
 * - An unknown principal is "Unknown member"; an ID is never rendered.
 * - A former member is muted and followed by " · former member".
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
      {layout.handlePlacement === 'paren' && <> ({handle(false)})</>}
      {layout.agentOf && identityCopy.agentSuffix}
    </span>
  );

  const body = (
    <span
      className={cn(layout.former && 'text-text-muted', className)}
      data-person-context={context}
      data-person-state={layout.former ? 'former' : 'active'}
    >
      {nameGroup}
      {layout.handlePlacement === 'secondary' && <> {handle(true)}</>}
      {layout.handlePlacement === 'tooltip' && (
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

  if (layout.handlePlacement !== 'tooltip' || !tooltip) return body;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{body}</TooltipTrigger>
      <TooltipContent>
        <bdi translate="no">{layout.handle}</bdi>
      </TooltipContent>
    </Tooltip>
  );
}
