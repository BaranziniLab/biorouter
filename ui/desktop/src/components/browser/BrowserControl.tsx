import { useCallback, useEffect, useRef, useState } from 'react';
import { modelDisplayName } from '../bottom_menu/modelLabel';
import { Compass } from '../icons/app-icons';
import { Button } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import {
  BrowserNotApplicable,
  allowBrowser,
  browserStatus,
  stopBrowser,
  type BrowserStatus,
} from './browserApi';

/**
 * The Biorouter Browser's approval card and Stop, above the composer (BB-17, BB-18).
 *
 * It sits in the agent control slot beside Biorouter Copilot's panel, wears the same shell and
 * follows the same rules: a pending approval forces the card open with the consent text in
 * full (never folded into an InfoTip), the model and the data destination are named, and Stop
 * stays visible while the browser holds a grant. Stop is one control for both agent
 * capabilities: the daemon's revoke ends the browser's grants and Copilot's, and cancels any
 * browser operation in flight.
 *
 * Two grants (BB-17): reading (once per chat) and acting (once per request, or for the chat
 * from the card's secondary button).
 */

const PANEL_SHELL =
  'br-fill-surface mx-3 mb-2 rounded-container border border-transparent bg-background-fill px-3 py-2.5 text-supporting';

const STOP_CONSEQUENCE = 'Stop cannot undo changes already made.';
const ACT_HELP =
  'Acting covers clicking anything other than a plain link, filling and submitting forms, pressing keys, choosing options, answering dialogs and downloading.';
const READ_HELP =
  'Reading covers opening and navigating pages, reading them, screenshots, scrolling and following plain links. It lasts for this chat until you stop it.';

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object' && 'message' in error) return String(error.message);
  return 'The Biorouter Browser could not confirm this change. Try again.';
}

export function BrowserControl({ sessionId }: { sessionId: string }) {
  const [status, setStatus] = useState<BrowserStatus>();
  const [notApplicable, setNotApplicable] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');
  const mounted = useRef(true);
  const busy = useRef(false);
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    if (busy.current) return;
    const started = generation.current;
    try {
      const next = await browserStatus(sessionId);
      if (mounted.current && started === generation.current && next.session_id === sessionId) {
        setStatus(next);
      }
    } catch (failure) {
      if (!mounted.current) return;
      // Chat mode never runs the browser: no card, and no poll for the life of the chat.
      if (failure instanceof BrowserNotApplicable) setNotApplicable(true);
    }
  }, [sessionId]);

  useEffect(() => {
    mounted.current = true;
    const stop = notApplicable || status?.available === false;
    const timer = stop ? undefined : window.setInterval(() => void refresh(), 2000);
    if (!stop) void refresh();
    return () => {
      mounted.current = false;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [refresh, notApplicable, status?.available]);

  const decide = async (action: () => Promise<BrowserStatus>) => {
    if (busy.current) return;
    busy.current = true;
    generation.current += 1;
    setSending(true);
    setError('');
    try {
      const next = await action();
      if (mounted.current) setStatus(next);
    } catch (failure) {
      if (mounted.current) setError(errorText(failure));
    } finally {
      busy.current = false;
      if (mounted.current) setSending(false);
    }
  };

  if (notApplicable || !status || status.available === false) return null;
  const requested = status.state === 'approval_required' ? status.requested : null;
  const active = status.state === 'active';
  if (!requested && !active) return null;
  if (status.enabled === false && !active) return null;

  const stop = () => decide(() => stopBrowser(sessionId));

  if (!requested) {
    // Active: one line, what is allowed, and Stop.
    const allowed = status.act
      ? status.act === 'chat'
        ? 'reading and acting, for this chat'
        : 'reading, and acting for this request'
      : 'reading, for this chat';
    return (
      <section
        aria-label="Biorouter Browser"
        className={`${PANEL_SHELL} flex min-w-0 items-center gap-2 text-text-default`}
      >
        <Compass className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 break-words" role="status">
          Biorouter Browser active <span className="text-text-muted">· {allowed}</span>
        </span>
        <Button size="sm" variant="destructive" disabled={sending} onClick={() => void stop()}>
          {sending ? 'Stopping…' : 'Stop'}
        </Button>
        {error && (
          <p role="alert" className="basis-full text-text-danger">
            {error}
          </p>
        )}
      </section>
    );
  }

  const acting = requested === 'act';
  return (
    <section aria-label="Biorouter Browser" className={`${PANEL_SHELL} text-text-default`}>
      <div className="flex min-w-0 items-center gap-2">
        <Compass className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 break-words" role="status">
          {acting
            ? 'Biorouter Browser wants to act on a website'
            : 'Biorouter Browser wants to browse'}
        </span>
        <InfoTip
          label={acting ? 'acting on websites' : 'browsing'}
          help={acting ? ACT_HELP : READ_HELP}
        />
      </div>
      <div className="mt-2 space-y-2">
        <p className="break-words">
          Model: {modelDisplayName(status.model)} · {status.provider}
        </p>
        <p className="break-words">Data destination: {status.destination}</p>
        {/* The consent stays whole and in view: it is what the person agrees to. */}
        <p className="whitespace-pre-line">{status.disclosure}</p>
        {acting && <p className="text-text-muted">{STOP_CONSEQUENCE}</p>}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={sending}
            onClick={() =>
              void decide(() => allowBrowser(status, acting ? 'act' : 'observe', 'request'))
            }
          >
            {sending ? 'Allowing…' : acting ? 'Allow for this request' : 'Allow browsing'}
          </Button>
          {acting && (
            <Button
              size="sm"
              variant="secondary"
              disabled={sending}
              onClick={() => void decide(() => allowBrowser(status, 'act', 'chat'))}
            >
              Allow for this chat
            </Button>
          )}
          <Button size="sm" variant="ghost" disabled={sending} onClick={() => void stop()}>
            Cancel
          </Button>
        </div>
      </div>
      {error && (
        <p role="alert" className="mt-2 text-text-danger">
          {error}
        </p>
      )}
    </section>
  );
}
