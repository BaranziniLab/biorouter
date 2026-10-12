import React, { useEffect, useRef, useState } from 'react';
import { MoreHorizontal } from './icons/app-icons';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';
import { PrivacyBadge } from './ui/PrivacyBadge';
import { AffiliationBadge } from './ui/AffiliationBadge';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import { transcriptCopy } from './transcript/copy';
import type { ProviderAffiliation } from './privacy/providerAffiliation';
import type { SessionClassification } from '../api';

interface Props {
  name: string;
  onRename: (newName: string) => void;
  onDiverge?: () => void | Promise<void>;
  canDiverge?: boolean;
  /**
   * The chat's privacy tier, shown as a pill beside the title (issue #56, R10).
   *
   * Optional and UNDEFINED-SILENT: a chat whose tier is not known yet says
   * nothing rather than claiming Public, because "we have not loaded it" and
   * "it is public" are different facts and only one of them is safe to assert.
   *
   * It is a LABEL here, not a control. Declassification does not belong in this
   * component's overflow menu (§12.1) — that menu is rename/diverge, one
   * careless click from the title, and lowering a chat's tier is a decision
   * that owes its own confirmed surface.
   */
  privacyTier?: SessionClassification;
  /**
   * DR-26's third axis for the model this chat is bound to — *under whose
   * agreements?* (issue #56).
   *
   * ⚠ **It is the BOUND PROVIDER's, not the chat's**, and that is not the same
   * kind of value as {@link privacyTier} beside it. The tier here is the
   * session's ratcheted classification; affiliation is a property of the
   * endpoint the transcript is going to right now, so it changes the moment the
   * model changes and does not ratchet. Rendering them together is the point —
   * they are the two questions a user has about one chat — but they are answered
   * by different objects, which is why this arrives as its own prop rather than
   * being derived from the tier.
   *
   * `null`/`undefined` renders nothing: a public model has no affiliation, and
   * neither does one whose provider row has not loaded.
   */
  affiliation?: ProviderAffiliation | null;
  className?: string;
}

const copy = transcriptCopy.title;

export const SessionNamePill: React.FC<Props> = ({
  name,
  onRename,
  onDiverge,
  canDiverge = false,
  privacyTier,
  affiliation,
  className,
}) => {
  const [editing, setEditing] = useState(false);
  const [diverging, setDiverging] = useState(false);
  const [draft, setDraft] = useState(name);
  const inputRef = useRef<HTMLInputElement>(null);
  const noDragStyle = { WebkitAppRegion: 'no-drag' } as React.CSSProperties;

  useEffect(() => {
    setDraft(name);
  }, [name]);
  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== name) onRename(trimmed);
    setEditing(false);
  };

  const startEditing = (event?: React.SyntheticEvent | Event) => {
    event?.stopPropagation();
    setEditing(true);
  };

  const handleDiverge = (event?: React.SyntheticEvent | Event) => {
    event?.stopPropagation();
    if (!onDiverge || !canDiverge || diverging) return;
    setDiverging(true);
    void Promise.resolve(onDiverge()).finally(() => setDiverging(false));
  };

  return (
    <div
      className={`inline-flex h-10 min-w-0 items-center gap-1 no-drag ${className ?? ''}`}
      style={noDragStyle}
    >
      {editing ? (
        // The shared field, as every other rename in the app (Crew's included).
        <Input
          ref={inputRef}
          value={draft}
          maxLength={200}
          aria-label={copy.renameField}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onPointerDown={(event) => event.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commit();
            if (e.key === 'Escape') {
              setDraft(name);
              setEditing(false);
            }
          }}
          style={noDragStyle}
          className="w-[min(360px,70vw)] min-w-[120px]"
        />
      ) : (
        <>
          <span className="inline-flex h-8 max-w-full min-w-0 items-center pl-1 text-label text-text-default">
            <span className="truncate no-drag">{name}</span>
          </span>
          {privacyTier && <PrivacyBadge tier={privacyTier} className="no-drag" />}
          {/* The two axes, side by side and in the same visual language —
              DR-26's "how sensitive?" and "under whose agreements?". The
              affiliation badge renders nothing at all for a public model, so a
              public chat is unchanged from before this landed. */}
          <AffiliationBadge affiliation={affiliation} className="no-drag min-w-0 max-w-[160px]" />
          <DropdownMenu>
            <Tooltip>
              <TooltipTrigger asChild>
                <DropdownMenuTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    shape="round"
                    aria-label={copy.actions}
                    className="shrink-0 text-text-muted hover:text-text-default"
                    style={noDragStyle}
                    onPointerDown={(event) => event.stopPropagation()}
                  >
                    <MoreHorizontal aria-hidden="true" />
                  </Button>
                </DropdownMenuTrigger>
              </TooltipTrigger>
              <TooltipContent>{transcriptCopy.message.more}</TooltipContent>
            </Tooltip>
            <DropdownMenuContent align="start" side="bottom">
              <DropdownMenuItem onSelect={startEditing}>{copy.rename}</DropdownMenuItem>
              {onDiverge && (
                <DropdownMenuItem disabled={!canDiverge || diverging} onSelect={handleDiverge}>
                  {diverging ? copy.diverging : copy.diverge}
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      )}
    </div>
  );
};
