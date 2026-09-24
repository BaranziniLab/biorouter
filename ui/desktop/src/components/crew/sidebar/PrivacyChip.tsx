import { useState } from 'react';
import { Button } from '../../ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { PrivacyPopover } from './PrivacyPopover';
import { verifiedPrivacy } from './sidebarView';
import './crew-sidebar.css';

/**
 * The privacy chip on the status row (ui-redesign-spec, "Privacy and institution").
 *
 * It states the EFFECTIVE mode — the one the broker enforces for this person here — and the
 * institution in force, as `PrivacyBadge` plus ` · ucsf`. Its accessible name is the copy deck's
 * `Privacy: Private · ucsf` / `Privacy: Public` (the regression tests' anchor).
 *
 * ⚠ **An unverified mode never looks verified.** Until the observer verifies this connection's
 * privacy the chip is plain text, "Checking privacy…", with no padlock and nothing to open: the
 * saved connection record and the last verified copy are not evidence of the mode in force now.
 *
 * `enforcementOff={false}`: the broker enforces Crew mode on its own, whatever this machine's
 * privacy master switch says, so the badge's "(enforcement off)" suffix would be false here.
 * Privacy never animates; the chip swaps in place.
 */
export function PrivacyChip() {
  const crew = useCrew();
  const [open, setOpen] = useState(false);
  const privacy = verifiedPrivacy(crew);

  if (!privacy) {
    return (
      <span className="crew-sidebar-chip-text" data-crew-privacy="checking">
        {sidebarCopy.chip.checking}
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
          className="no-drag min-w-0 shrink px-1.5"
          aria-label={sidebarCopy.chip.name(privacy.effective, institution)}
          data-crew-privacy={privacy.effective}
        >
          <PrivacyBadge tier={privacy.effective} enforcementOff={false} />
          {institution && (
            <span className="crew-sidebar-chip-text">
              {' · '}
              <bdi className="crew-sidebar-truncate" translate="no">
                {institution}
              </bdi>
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-3">
        <PrivacyPopover privacy={privacy} onClose={() => setOpen(false)} />
      </PopoverContent>
    </Popover>
  );
}
