import { useState } from 'react';
import { AlertCircle, ChevronDown, ChevronRight, Info, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../../ui/collapsible';
import { InfoTip } from '../../ui/info-tip';
import { Note } from '../../ui/note';
import { knowledgeCopy } from '../copy';
import type { FileDropWarning } from './fileValidation';

interface Props {
  warnings: FileDropWarning[];
  onDismiss: (id: string) => void;
  onClear: () => void;
}

/**
 * The files the dropzone set aside, folded until wanted (spec 3.12).
 *
 * One header line, "{n} warnings", with the curation advice in an InfoTip, over a list of
 * `Note`s. The header is not a card: the notes are the only boxes, so nothing is a box inside a
 * box, and each note takes the app's one tone recipe (`--wash-*` behind its own ink) instead of
 * hand-mixed alphas.
 */
export function IngestWarnings({ warnings, onDismiss, onClear }: Props) {
  const [open, setOpen] = useState(false);

  if (warnings.length === 0) {
    return null;
  }

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="flex flex-col">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1">
          <CollapsibleTrigger className="biorouter-focus-surface flex min-w-0 items-center gap-2 rounded-element py-1 pr-1 text-left text-label">
            {open ? (
              <ChevronDown
                className="h-icon-row w-icon-row shrink-0 text-text-muted"
                aria-hidden="true"
              />
            ) : (
              <ChevronRight
                className="h-icon-row w-icon-row shrink-0 text-text-muted"
                aria-hidden="true"
              />
            )}
            <span className="tabular-nums">{knowledgeCopy.ingest.warnings(warnings.length)}</span>
          </CollapsibleTrigger>
          <InfoTip
            label={knowledgeCopy.ingest.warningsInfoLabel}
            help={knowledgeCopy.ingest.warningsInfo}
          />
        </div>
        <Button type="button" variant="ghost" size="sm" className="shrink-0" onClick={onClear}>
          Clear
        </Button>
      </div>

      <CollapsibleContent className="flex flex-col gap-2 pt-2">
        {warnings.map((warning) => {
          const isError = warning.level === 'error';
          return (
            <Note
              key={warning.id}
              tone={isError ? 'danger' : 'warning'}
              icon={isError ? AlertCircle : Info}
              action={
                <Button
                  type="button"
                  variant="ghost"
                  shape="round"
                  size="sm"
                  onClick={() => onDismiss(warning.id)}
                  aria-label={`Dismiss "${warning.title}"`}
                >
                  <X aria-hidden="true" />
                </Button>
              }
            >
              <p className="font-semibold">{warning.title}</p>
              <p className="break-words">{warning.message}</p>
            </Note>
          );
        })}
      </CollapsibleContent>
    </Collapsible>
  );
}
