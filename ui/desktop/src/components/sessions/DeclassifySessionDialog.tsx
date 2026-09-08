import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { declassifySession, type Session } from '../../api';
import { toastError, toastSuccess } from '../../toasts';
import { userActionHeaders } from '../../utils/userAction';
import { DangerousConfirmDialog } from '../ui/DangerousConfirmDialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { MODAL_SIZE } from '../ModalShell';

/**
 * Issue #56 §12.4's graded confirmation, mirrored from
 * `requires_typed_confirmation` in `crates/biorouter/src/privacy/declassify.rs`.
 * The daemon re-derives the same grade from the stored provenance and refuses a
 * request that skips the phrase, so this decides which control is SHOWN, never
 * whether the phrase is enforced.
 *
 * Phrased as an exception, not a match, for the reason spelled out on the Rust
 * side: `privacy_reason`'s vocabulary is `turn:*`, `mcp:*`, `inherited:<parent>`,
 * `diverged:<parent>`, `backfill:<provider>` and `imported`, so a chat branched
 * out of an OMOP session carries `diverged:<parent>` and none of the `mcp:`
 * spelling. Reading the absence of `turn:` as "unknown, so protect it" gets the
 * inherited cases right without walking ancestors, and gets an absent or
 * unrecognised reason right too.
 */
export function requiresTypedConfirmation(privacyReason?: string | null): boolean {
  return !(privacyReason ?? '').startsWith('turn:');
}

/**
 * **Why THIS chat is on the strong control**, mirrored clause-for-clause from
 * `strong_confirmation_reason` in `crates/biorouter/src/privacy/declassify.rs`.
 * `null` exactly when the single click applies.
 *
 * ⚠ This exists because the dialog used to say "This chat reached a private data
 * source" for every chat on the strong control, and that is false for two of the
 * provenances that reach it. The one-time migration marks a chat
 * `backfill:<provider>` from the model it was last bound to, having observed
 * nothing it reached, and an `imported` chat arrived already marked — so on day
 * one, on a machine with history, most of the chats shown this dialog were being
 * told a reason that did not happen.
 *
 * The clause completes both "This chat …" and "Session `<id>` …", which is what
 * lets this, the daemon and the CLI share one wording. The Rust test
 * `the_desktop_dialog_carries_the_same_clauses` reads this file and asserts every
 * clause below appears in it verbatim, so a drift on either side turns the build
 * red rather than shipping two different explanations of the same control.
 */
export function strongConfirmationReason(privacyReason?: string | null): string | null {
  if (!requiresTypedConfirmation(privacyReason)) return null;
  const reason = privacyReason ?? '';
  if (reason.startsWith('mcp:')) return 'reached a private data source';
  if (reason.startsWith('inherited:')) return 'was created inside a private chat';
  if (reason.startsWith('diverged:')) return 'was branched out of a private chat';
  if (reason.startsWith('backfill:'))
    return 'was marked private by the one-time migration, from the model it was last using rather than from anything it reached';
  if (reason === 'imported') return 'was imported already marked private';
  // Everything the vocabulary does not name — an absent reason, a future entry.
  // A statement about the RECORD, because nothing about the conversation is
  // known here.
  return 'does not record an observed turn on a private model as the reason it is private';
}

/**
 * The last six characters of the session id — what §12.4 asks the user to
 * retype, and what `confirmation_phrase` derives on the daemon.
 *
 * Characters, not `slice(-6)` on the raw string: a surrogate pair would be cut
 * in half by the latter. NOT the session NAME — `is_default_session_name` shows
 * "New Session", "CLI Session", "Session <N>" and "New session <N>" are all live
 * placeholders, so a name-typed phrase is either a string dozens of rows share
 * (destroying the justification, which is to make the user check WHICH
 * conversation) or a whole sentence to retype.
 */
export function confirmationPhrase(sessionId: string): string {
  return [...sessionId].slice(-6).join('');
}

type Phase = 'confirm' | 'undo' | 'sending';

export interface DeclassifySessionDialogProps {
  session: Session;
  /**
   * Defaults to open. This dialog is mounted by the control that opens it
   * (History's row menu, the session page's action bar) and unmounted when it
   * closes, so there is no state to keep for a closed one.
   */
  open?: boolean;
  onClose?: () => void;
  onDeclassified?: (sessionId: string) => void;
  /** The undo window on the single-click path. A seam for tests, not a setting. */
  undoMs?: number;
}

/**
 * Issue #56 §12.4/§12.5 — the one surface that marks a private chat public.
 *
 * Shared by both entry points (History's row menu and the session page) so the
 * two cannot come to disagree about what confirmation this takes. It is
 * deliberately NOT reachable from `SessionNamePill`'s title menu: that menu is
 * rename/diverge, one careless click from the chat title, and §12.1 puts this
 * on the History row instead.
 *
 * Two controls, graded on provenance:
 *
 *  * Every other provenance — `mcp:*`, but also `inherited:*`, `diverged:*`,
 *    `backfill:*`, `imported` and anything unrecognised — asks the user to
 *    retype the last six characters of its id, shown beside the field, above a
 *    sentence naming the reason that is actually true of that chat
 *    (`strongConfirmationReason`). It is NOT one sentence about reaching a data
 *    source: most of these chats did not.
 *  * A chat that merely ran turns against a private **model** gets a single
 *    click, with an undo window — and the request is HELD for that window
 *    rather than sent and reversed, because declassification is one-way at the
 *    daemon: a re-raise would write a second ledger row under a different
 *    provenance, leaving an audit trail that says something happened which the
 *    user believes did not.
 */
export function DeclassifySessionDialog({
  session,
  open = true,
  onClose,
  onDeclassified,
  undoMs = 5000,
}: DeclassifySessionDialogProps) {
  const [phase, setPhase] = useState<Phase>('confirm');
  // Set when a request that carried no confirmation came back refused. The
  // grade below is read off a row from the session list's CACHE, so it can be
  // stale; the daemon's is read off the row inside the writing transaction and
  // is the one that decides. See the catch in `send`.
  const [escalated, setEscalated] = useState(false);
  const gradedStrong = requiresTypedConfirmation(session.privacy_reason);
  // Non-null exactly when `gradedStrong` is true, by construction — one
  // predicate decides both — so the strong copy cannot be shown to a chat
  // getting the single click.
  const strongReason = strongConfirmationReason(session.privacy_reason);
  const needsPhrase = gradedStrong || escalated;
  const phrase = confirmationPhrase(session.id);
  const undoSeconds = Math.max(1, Math.round(undoMs / 1000));

  // The callbacks are read through a ref so that `send` — and therefore the undo
  // timer's effect — has an identity that does not change when the parent
  // re-renders. Both entry points pass INLINE arrows (`SessionListView`:
  // `onClose={() => setDeclassifyTarget(null)}`), so a `send` that closed over
  // them directly would be a new function on every parent render; an effect
  // keyed on it clears the pending timeout and arms a fresh FULL-LENGTH one, so
  // one re-render doubles the advertised window and a re-render source faster
  // than the window — the session list's own change subscription, a search
  // debounce — means the request is never sent at all. Pinned by "the undo
  // window is a deadline, not a countdown a re-render restarts".
  const latest = useRef({ onClose, onDeclassified });
  useLayoutEffect(() => {
    latest.current = { onClose, onDeclassified };
  });

  const send = useCallback(
    async (confirmation: string | null) => {
      setPhase('sending');
      try {
        await declassifySession({
          path: { session_id: session.id },
          body: { confirmation },
          // DR-16's proof-of-user. Without it the daemon refuses, correctly:
          // the server secret alone is reachable from any developer-enabled
          // agent shell (§9.3 A1) and is not evidence of a human.
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        toastSuccess({
          title: 'Chat marked public',
          msg: 'It no longer carries a private marker. The change is recorded.',
        });
        latest.current.onDeclassified?.(session.id);
        latest.current.onClose?.();
      } catch (error) {
        toastError({
          title: 'Could not mark this chat public',
          msg: error instanceof Error ? error.message : String(error),
        });
        // A request that carried no confirmation was refused, so the weak
        // control this dialog rendered was the wrong one — most likely because
        // the cached row's `turn:*` provenance has since been displaced by an
        // `mcp:*` one. Returning to the same control would re-render it from
        // the same stale prop and fail identically, forever. Escalating is the
        // only recovery, and it is never the wrong answer to a refusal.
        if (confirmation === null) setEscalated(true);
        setPhase('confirm');
      }
    },
    [session.id]
  );

  // The undo window. The request has NOT been made while this is pending —
  // that is the whole difference between an undo and an apology. Unmounting
  // therefore sends nothing, which is the safe direction: the chat stays
  // private rather than turning public after the user left the surface that
  // announced it.
  useEffect(() => {
    if (phase !== 'undo') return;
    const timer = setTimeout(() => void send(null), undoMs);
    return () => clearTimeout(timer);
  }, [phase, send, undoMs]);

  if (phase === 'undo') {
    return (
      <Dialog open={open} onOpenChange={(next) => !next && setPhase('confirm')}>
        {/* V8 — the ladder's form rung by name; the literal it replaces was
            already 480px, so the width is unchanged and only the spelling
            moves onto the one ladder `ModalShell.tsx` documents. */}
        <DialogContent className={MODAL_SIZE.md}>
          <DialogHeader>
            <DialogTitle>Marking this chat public…</DialogTitle>
            <DialogDescription>
              Nothing has been sent yet. This chat becomes public in {undoSeconds} seconds unless
              you undo.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="pt-2">
            <Button type="button" variant="outline" onClick={() => setPhase('confirm')}>
              Undo
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }

  return (
    <DangerousConfirmDialog
      open={open}
      busy={phase === 'sending'}
      title="Make this chat public?"
      description={
        gradedStrong
          ? `This chat ${strongReason}. Marking it public removes its private ` +
            'marker and lets it be opened by a public model. Its contents are unchanged, and ' +
            'this cannot be undone.'
          : escalated
            ? // Not the sentence above: nothing here established that this chat
              // reached a private data source, and saying so would be a claim
              // about the conversation rather than about the refusal.
              "That request was refused. This chat's record has changed since this list was " +
              'loaded, so it now takes the typed confirmation. Marking it public cannot be undone.'
            : `This chat only ran turns against a private model. Marking it public removes its ` +
              `private marker; you will have ${undoSeconds} seconds to undo.`
      }
      phrase={needsPhrase ? phrase : undefined}
      fieldLabel="Type the last 6 characters of this chat's ID"
      confirmLabel="Make public"
      onConfirm={() => (needsPhrase ? void send(phrase) : setPhase('undo'))}
      onCancel={() => onClose?.()}
    >
      {/* The ResetPanel precedent: show what is being acted on, so a user who
          opened the menu on the wrong row sees it before confirming.

          V4/V6/V8 — the `Note` primitive, not a fourth hand-rolled panel. What
          this replaced carried all three of the bans in one element:
          `rounded-lg` (the deprecated alias of the element radius),
          `bg-background-muted/40` (a hand-mixed alpha, so the block sat at a
          different depth from every other neutral surface in the app) and
          `text-sm`/`text-xs` (sizes where the roles `text-label` and
          `text-supporting` say the same thing and carry their own weight and
          line-height). `Note`'s neutral tone IS `border border-border-subtle
          bg-background-muted` — the same construction, at full strength, from
          one definition. */}
      <Note>
        <p className="truncate text-label text-text-default" title={session.name}>
          {session.name}
        </p>
        <p className="mt-0.5 font-mono text-supporting text-text-muted">{session.id}</p>
      </Note>
    </DangerousConfirmDialog>
  );
}
