import { useId, useRef, useState } from 'react';
import { Button } from '../../ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { useCrew } from '../state/CrewControllerContext';
import type { ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { PrivacyPopover } from './PrivacyPopover';
import { useKnownInstitutions, verifiedPrivacy } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.chip;

/**
 * Statuses in which nothing is checking this connection's privacy, so the chip says nothing at
 * all (Q2-17, Q2-01, Q2-43): "Offline · Checking privacy…" put a claim of work in progress beside
 * a status that says none is possible. The status word is the whole story there — a connection
 * that is down, reconnecting, waiting for sign-in or refused, a view whose updates stopped, and a
 * joiner the host has not let in yet (the workspace reports nothing about a non-member).
 *
 * Every other unverified state (connecting, checking, updating) reads "Checking privacy…": the
 * next snapshot verifies it.
 */
export const CHIP_SILENT_STATUSES: ReadonlySet<ConnectionStatusKey> = new Set<ConnectionStatusKey>([
  'offline',
  'reconnecting',
  'sign-in-needed',
  'cant-connect',
  'cant-verify',
  'not-set-up',
  'not-joined',
  'updates-unavailable',
]);

/**
 * The privacy chip on the status row (ui-redesign-spec, "Privacy and institution").
 *
 * It states the EFFECTIVE mode — the one the broker enforces for this person here — and the
 * institution in force as ONE badge, `🔒 Private · UCSF` (Q2-45): the institution sits inside the
 * badge's fill, never beside it as a second chip. The institution is worded by the name a
 * configured provider publishes for its ID, else the ID itself (Q2-38). Its accessible name is
 * the copy deck's `Privacy: Private · ucsf` / `Privacy: Public` (the regression tests' anchor).
 *
 * ⚠ **An unverified mode never looks verified.** Until the observer verifies this connection's
 * privacy the chip has no padlock and nothing to open: the saved connection record and the last
 * verified copy are not evidence of the mode in force now. While a connect or a refresh is
 * verifying it, it reads "Checking privacy…" with a tooltip saying what it waits for (T-06,
 * T-68); in a status where nothing is verifying it ({@link CHIP_SILENT_STATUSES}) it renders
 * nothing.
 *
 * The popover is named by its own title and opens with focus on itself, never on the first
 * action: that action is "Make my connection public…", and landing on it invites an Enter
 * nobody meant (T-38). It aligns to the chip's start edge, so it stays over the Crew column.
 *
 * `enforcementOff={false}`: the broker enforces Crew mode on its own, whatever this machine's
 * privacy master switch says, so the badge's "(enforcement off)" suffix would be false here.
 * Privacy never animates: the chip swaps in place and takes no press scale.
 */
export function PrivacyChip() {
  const crew = useCrew();
  const [open, setOpen] = useState(false);
  const titleId = useId();
  const contentRef = useRef<HTMLDivElement>(null);
  const known = useKnownInstitutions();
  const privacy = verifiedPrivacy(crew, known);

  if (!privacy) {
    if (crew.status === null || CHIP_SILENT_STATUSES.has(crew.status)) return null;
    return (
      <span
        className="crew-sidebar-chip-text"
        data-crew-privacy="checking"
        title={copy.checkingHint}
      >
        <span className="crew-sidebar-truncate">{copy.checking}</span>
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
          className="crew-sidebar-chip no-drag min-w-0 shrink px-1 active:scale-100"
          aria-label={copy.name(privacy.effective, institution)}
          data-crew-privacy={privacy.effective}
        >
          <span className="crew-sidebar-chip-badge" data-crew-privacy-badge={privacy.effective}>
            <PrivacyBadge tier={privacy.effective} enforcementOff={false} />
            {institution && (
              <span className="crew-sidebar-chip-institution">
                {' · '}
                <bdi className="crew-sidebar-truncate" translate="no" title={institution}>
                  {institution}
                </bdi>
              </span>
            )}
          </span>
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
