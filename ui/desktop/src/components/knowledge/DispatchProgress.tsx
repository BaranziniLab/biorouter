import { AlertCircle, CheckCircle2, ChevronDown, ChevronRight } from '../icons/app-icons';
import { useMemo, useState } from 'react';
import { Spinner } from '../ui/spinner';
import type { StreamState, SubAgentEvent } from './hooks/useIngestStream';

function prettyToolName(name: string): string {
  const raw = name.startsWith('kb_') ? name.slice(3) : name;
  return raw
    .split('_')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function summarizeArgs(args: unknown): string {
  if (!args || typeof args !== 'object' || Array.isArray(args)) {
    return 'Using the current staged source context.';
  }

  const entries = Object.entries(args as Record<string, unknown>)
    .filter(([, value]) => value !== undefined && value !== null && value !== '')
    .slice(0, 3)
    .map(([key, value]) => {
      const rendered =
        typeof value === 'string'
          ? value
          : typeof value === 'number' || typeof value === 'boolean'
            ? String(value)
            : JSON.stringify(value);
      return `${key}: ${rendered}`;
    });

  return entries.length > 0 ? entries.join(' • ') : 'Using the current staged source context.';
}

function renderEventLine(ev: SubAgentEvent): { tone: string; text: string } {
  if (ev.kind === 'step') {
    return {
      tone: 'text-text-default',
      text: `Step ${ev.index + 1}: ${ev.assistant_text.trim() || 'Preparing the next knowledge-base action.'}`,
    };
  }

  if (ev.kind === 'tool_call') {
    return {
      tone: 'text-text-muted',
      text: `Tool call · ${prettyToolName(ev.name)} · ${summarizeArgs(ev.args)}`,
    };
  }

  if (ev.kind === 'tool_result') {
    return {
      tone: ev.ok ? 'text-text-success' : 'text-text-danger',
      text: `${ev.ok ? 'Completed' : 'Issue'} · ${prettyToolName(ev.name)} · ${
        ev.summary.trim() || (ev.ok ? 'Completed.' : 'Returned an error.')
      }`,
    };
  }

  return {
    tone: 'text-text-muted',
    text:
      ev.reason === 'cancelled'
        ? 'Digest stopped before the current item completed.'
        : `Digest finished · ${ev.reason}`,
  };
}

interface Props {
  state: StreamState;
}

/**
 * The per-item sub-agent log (ui-spec §4.4 state 3).
 *
 * ⚠ **It no longer carries a Stop control.** The one Stop in this rail is the
 * primary button in the footer, which turns into it while a digest runs. This
 * component used to draw a second, bare-text one inside the log — and the two
 * spelled the same word differently (`Stopping...` here, `Stopping…` in the
 * panel) on screen at the same time.
 *
 * ⚠ **The status label must stay a DIRECT text child of one element.**
 * `IngestPanel.streamFailure.test.tsx` selects it with `getByText(/Digest
 * error/i)`, and testing-library matches on an element's own text nodes: split
 * the label across two spans and the query finds nothing; hoist it so an
 * ancestor also owns it directly and the query finds two.
 */
export function DispatchProgress({ state }: Props) {
  const [open, setOpen] = useState(true);

  const statusLabel =
    state.status === 'starting'
      ? 'Preparing digest…'
      : state.status === 'streaming'
        ? 'Digesting…'
        : state.status === 'stopping'
          ? 'Stopping…'
          : state.status === 'done'
            ? 'Digest complete'
            : state.status === 'error'
              ? `Digest error: ${state.error}`
              : '';

  const lines = useMemo(() => state.events.map(renderEventLine), [state.events]);

  if (state.status === 'idle') return null;

  return (
    // A header line over the log, not a card around it: the log's well is the
    // one box, so nothing is a box inside a box (spec 3.12, principle 4).
    <div className="flex flex-col">
      <div className="flex items-center justify-between gap-3">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="biorouter-focus-surface flex min-w-0 flex-1 items-center gap-2 rounded-element py-1 pr-1 text-left text-label"
        >
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
          <span className="min-w-0 break-words">{statusLabel}</span>
        </button>
        <span className="shrink-0 text-supporting tabular-nums text-text-muted">
          {state.events.length} events
        </span>
      </div>

      {open && (
        <div className="pt-2">
          <div className="max-h-[260px] space-y-2 overflow-y-auto rounded-element bg-background-well px-3 py-3 text-supporting">
            {lines.length === 0 && (
              <div className="flex items-start gap-2 text-text-muted">
                <Spinner className="mt-0.5 shrink-0" />
                <p className="min-w-0 break-words whitespace-pre-wrap">
                  {state.status === 'starting'
                    ? 'Checking the staged source and opening the digest pipeline.'
                    : state.status === 'stopping'
                      ? 'Waiting for the current request to stop cleanly.'
                      : 'Waiting for tool-call updates.'}
                </p>
              </div>
            )}

            {lines.map((line, index) => (
              <div key={index} className={`flex items-start gap-2 ${line.tone}`}>
                {line.text.startsWith('Completed') ? (
                  <CheckCircle2
                    className="mt-0.5 h-icon-row w-icon-row shrink-0"
                    aria-hidden="true"
                  />
                ) : line.text.startsWith('Issue') || line.text.startsWith('Digest error') ? (
                  <AlertCircle
                    className="mt-0.5 h-icon-row w-icon-row shrink-0"
                    aria-hidden="true"
                  />
                ) : (
                  <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-current opacity-60" />
                )}
                <p className="min-w-0 break-words whitespace-pre-wrap">{line.text}</p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
