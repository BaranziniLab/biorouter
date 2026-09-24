import { useId, useRef, useState } from 'react';
import { Button } from '../../ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { PrivacyPopover } from './PrivacyPopover';
import { verifiedPrivacy } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.chip;

/**
 * The privacy chip on the status row (ui-redesign-spec, "Privacy and institution").
 *
 * It states the EFFECTIVE mode — the one the broker enforces for this person here — and the
 * institution in force, as `PrivacyBadge` plus ` · ucsf`. Its accessible name is the copy deck's
 * `Privacy: Private · ucsf` / `Privacy: Public` (the regression tests' anchor).
 *
 * ⚠ **An unverified mode never looks verified.** Until the observer verifies this connection's
 * privacy the chip is plain text with no padlock and nothing to open: the saved connection record
 * and the last verified copy are not evidence of the mode in force now. Which text depends on
 * whether it can resolve (T-06):
 *
 * - a joiner the host has not let in yet reads "Privacy shown after you join" — the workspace
 *   reports nothing about a non-member's privacy, so "Checking…" would never end;
 * - everyone else, while a refresh (re-)verifies, reads "Checking privacy…".
 *
 * Both carry a `title` with the full words and what they wait for, since the row can truncate
 * them (T-68).
 *
 * The popover is named by its own title and opens with focus on itself, never on the first
 * action: that action is "Make my connection public…", and landing on it invites an Enter
 * nobody meant (T-38). It aligns to the chip's start edge, so it stays over the Crew column.
 *
 * `enforcementOff={false}`: the broker enforces Crew mode on its own, whatever this machine's
 * privacy master switch says, so the badge's "(enforcement off)" suffix would be false here.
 * Privacy never animates; the chip swaps in place.
 */
export function PrivacyChip() {
  const crew = useCrew();
  const [open, setOpen] = useState(false);
  const titleId = useId();
  const contentRef = useRef<HTMLDivElement>(null);
  const privacy = verifiedPrivacy(crew);

  if (!privacy) {
    const notJoined = crew.status === 'not-joined';
    return (
      <span
        className="crew-sidebar-chip-text"
        data-crew-privacy={notJoined ? 'not-joined' : 'checking'}
        title={notJoined ? copy.notJoinedHint : copy.checkingHint}
      >
        <span className="crew-sidebar-truncate">{notJoined ? copy.notJoined : copy.checking}</span>
      </span>
    );
  }

  const institution = privacy.effective === 'private' ? privacy.institution : null;
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="crew-sidebar-chip no-drag min-w-0 shrink px-1.5"
          aria-label={copy.name(privacy.effective, institution)}
          data-crew-privacy={privacy.effective}
        >
          <PrivacyBadge tier={privacy.effective} enforcementOff={false} />
          {institution && (
            <span className="crew-sidebar-chip-text">
              {' · '}
              <bdi className="crew-sidebar-truncate" translate="no" title={institution}>
                {institution}
              </bdi>
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        ref={contentRef}
        align="start"
        className="w-72 p-3 focus:outline-none"
        tabIndex={-1}
        aria-labelledby={titleId}
        data-crew-privacy-popover-content=""
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          contentRef.current?.focus();
        }}
      >
        <PrivacyPopover privacy={privacy} titleId={titleId} onClose={() => setOpen(false)} />
      </PopoverContent>
    </Popover>
  );
}
