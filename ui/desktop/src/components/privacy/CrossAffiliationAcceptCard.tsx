import { useCallback, useEffect, useState } from 'react';
import { Button } from '../ui/button';
import {
  acceptCrossAffiliationFlow,
  GRANT_SCOPE_COPY,
  readMixingMode,
  type MixingMode,
} from '../../utils/crossAffiliation';

export interface CrossAffiliationAcceptCardProps {
  /**
   * The chat the refusal happened in. A grant is keyed on it, so there is
   * nothing to record without one and the card renders nothing.
   */
  sessionId?: string;
  /** The extension key, read out of the refusal's accept frame. */
  extension: string;
}

/**
 * The user's accept control for one stated cross-institutional data flow
 * (issue #56, DR-26 / Task 57).
 *
 * ⚠ **It renders where the refusal lands** — inside the failed tool call in the
 * transcript, immediately under the daemon's own words. That placement is the
 * whole fix: the mechanism has existed since Task 49, and what was missing was
 * an affordance on the surface where the person meets the refusal. A control
 * somewhere else (a settings screen, a separate dialog they must go find) is the
 * same hard block with an extra step.
 *
 * ⚠ **The agent cannot press it.** DR-19: the model may ask, only the user may
 * answer. Two things hold that, and neither is this component's good intentions
 * — the daemon requires `X-User-Action`, which no tool-call path can carry; and
 * exactly one renderer module may name the grant call, audited by
 * `utils/crossAffiliation.test.ts`. The third thing this comment used to claim —
 * that the call has exactly one caller, and that it is an `onClick` — is true
 * today but is **not** what that audit pins; see the note on
 * {@link acceptCrossAffiliationFlow}'s module.
 *
 * ⚠ **What is deliberately NOT here: the risk statement.** The refusal directly
 * above this card already carries the warning verbatim, naming both
 * institutions, composed once in `privacy::affiliation` so every surface states
 * it in the same words. Restating it here would be a second account of one
 * boundary. What this adds is the half the refusal cannot state — how far a yes
 * reaches — and that comes from {@link GRANT_SCOPE_COPY}, mirrored from the
 * daemon rather than written again.
 *
 * ⚠ **DR-27's three modes are a difference in what the press COSTS, never in
 * whether there is one** (issue #56, DR-27). `standard` and `strict` render
 * the same control, post the same body to the same route, and differ in one
 * sentence of copy — because the extra proof `strict` demands is raised by the
 * DAEMON, not here: `agent_cross_affiliation_grant` calls
 * `strict_mode_authorization`, which reads `privacy::mixing::policy()` itself
 * and puts the operating system's dialog between the resolution and the write.
 * A renderer that branched further would be a second reader of the policy, and
 * the two would eventually disagree about a control the user has just used.
 *
 * ⚠ **Do not restore the dead end this used to have.** Between Task 57 and Task
 * 52 the strict branch rendered "this build cannot ask for a system password, so
 * the flow cannot be approved" — correct when it was written, and left standing
 * after DR-24's prompters and the daemon's strict layer landed. What it left
 * behind was a mode in which a cross-institutional flow could not be accepted at
 * all: the hard block DR-26 exists to prevent, restored for exactly the
 * deployments careful enough to choose `strict`. If a future build genuinely
 * cannot prompt, the honest refusal is the daemon's — `strict_mode_authorization`
 * answers 403 for an `Unavailable` prompter, naming the platform — and it
 * arrives through {@link approve}'s error path, after a press, rather than as a
 * renderer's guess before one.
 */
export function CrossAffiliationAcceptCard({
  sessionId,
  extension,
}: CrossAffiliationAcceptCardProps) {
  // `null` while unknown. Rendering the control before the policy is read would
  // draw it on a machine whose policy is `open` — where nothing was refused and
  // there is nothing to approve — and a control that appears and then withdraws
  // is worse than one that arrives a beat late. It would also, under `strict`,
  // put the button on screen a frame before the sentence saying what pressing it
  // costs, which is the one ordering that surface may not have.
  const [mode, setMode] = useState<MixingMode | null>(null);
  const [busy, setBusy] = useState(false);
  // ⚠ Component-local, so a remount (scrolling a long transcript, reopening the
  // chat) shows the button again for a flow that is already granted. Harmless —
  // `grant::record` is idempotent by triple and refreshes the timestamp rather
  // than erroring — but it is not the same as knowing. There is no route that
  // answers "is this triple granted", and inventing one to dim a button would
  // add a second reader of the grant store beside the gate that decides.
  const [accepted, setAccepted] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!sessionId) return;
    let live = true;
    void readMixingMode().then((next) => {
      if (live) setMode(next);
    });
    return () => {
      live = false;
    };
  }, [sessionId]);

  const approve = useCallback(async () => {
    if (!sessionId) return;
    setBusy(true);
    setError(null);
    try {
      setAccepted(await acceptCrossAffiliationFlow(sessionId, extension));
    } catch (e) {
      // Under `throwOnError` the generated client throws the PARSED BODY, so a
      // refusal arrives as a plain string and says more than any fallback could.
      // Two realistic ones: a daemon the app did not start, holding no
      // user-action key (open question 23); and — under `strict` — the 403 the
      // daemon answers when the operating system did not confirm, which already
      // says "Nothing was recorded" and, for an unavailable prompter, names the
      // platform. ⚠ Printing it rather than paraphrasing is what keeps a denied
      // password prompt distinguishable from a broken button: both leave the
      // control pressable, and only the daemon's sentence says which happened.
      setError(e instanceof Error ? e.message : typeof e === 'string' && e.trim() ? e : null);
    } finally {
      setBusy(false);
    }
  }, [sessionId, extension]);

  // A grant is keyed on (session, extension, model affiliation). No session, no
  // key, nothing to offer.
  if (!sessionId) return null;

  if (accepted) {
    return (
      <div
        data-testid="cross-affiliation-accepted"
        role="status"
        className="mt-3 rounded-element border border-transparent bg-background-default px-3 py-3 text-body text-text-default"
      >
        {/*
          The daemon's own composition (`grant::accepted_statement`), echoed
          back and printed verbatim. The sentence recorded and the sentence read
          must not differ by a word — a paraphrase here would leave the audit
          row saying one thing and the user remembering another.
        */}
        <p className="min-w-0 [overflow-wrap:anywhere]">{accepted}</p>
        {/*
          ⚠ The one thing the daemon's sentence cannot say: what happens next.
          The refusal told the model *do not retry — the same call will be
          refused again*, and it was right when it said it, so the model will
          not try on its own. Without this line the user is left holding a
          recorded acceptance and a conversation that has stopped, which is the
          same dead end one press further along.
        */}
        <p className="min-w-0 [overflow-wrap:anywhere] mt-2 text-text-muted">
          Ask the assistant to try that step again. It was told not to retry on its own.
        </p>
      </div>
    );
  }

  // DR-27's policy, read in ONE place. `open` raises no mismatch in the daemon,
  // so in practice no refusal reaches this component at all; the branch is here
  // because a control must not depend on an upstream gate staying silent.
  if (mode === null || mode === 'open') return null;

  return (
    <div
      data-testid="cross-affiliation-accept"
      className="mt-3 rounded-element border border-transparent bg-background-default px-3 py-3 text-body text-text-default space-y-3"
    >
      {/*
        ⚠ **The one thing `strict` changes on this surface: the user is told the
        press costs more, BEFORE they make it.** DR-20 point 4's premise is that
        a system dialog says honestly what it authorises; a dialog that arrives
        with no warning is one people dismiss as spurious — and dismissing it
        leaves the flow refused with nothing recorded, which reads as a broken
        button rather than as their own answer.

        It is a sentence, not a branch: the proof itself is the daemon's to
        demand (`strict_mode_authorization`), and this component never decides
        whether the prompt is raised. Above the scope copy because it is the
        first cost, and because it is the half a `standard` user does not have.
      */}
      {mode === 'strict' && (
        <p
          data-testid="cross-affiliation-strict-notice"
          className="min-w-0 [overflow-wrap:anywhere]"
        >
          {/* ⚠ "may", not "will". F-13: macOS has been observed approving
              `evaluatePolicy` instantly with no dialog off a recent
              authentication, and whether an explicit reuse duration of 0 defeats
              that is untested. Promising a dialog that does not appear teaches
              the user that a silent approval is normal, which is exactly the
              wrong lesson for the prompt that gates enforcement. */}
          This machine&rsquo;s privacy policy is set to{' '}
          <strong className="font-semibold">strict</strong>, so your operating system may ask you to
          confirm it is you before this is recorded.
        </p>
      )}
      {/*
        The scope, BEFORE the press. "How far does my yes reach" is part of what
        is being decided: a user who believes they are approving one call behaves
        differently from one who knows they are approving this connector for the
        rest of the chat. It is also what makes the narrowness legible.
      */}
      <p className="min-w-0 [overflow-wrap:anywhere]">{GRANT_SCOPE_COPY}</p>
      {error && (
        <p role="alert" className="min-w-0 [overflow-wrap:anywhere] text-text-muted">
          Nothing was recorded. {error}
        </p>
      )}
      <Button
        type="button"
        variant="default"
        size="sm"
        disabled={busy}
        onClick={() => void approve()}
      >
        {/*
          Names the connector, so a transcript with two refused calls in it does
          not present two identical buttons.
        */}
        Approve this flow for {extension}
      </Button>
    </div>
  );
}
