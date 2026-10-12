import { useRef } from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { disclosureTitle, type DisclosureCopy } from './disclosureCopy';
import { DisclosureProse } from './DisclosureProse';
import { NotificationSurface } from '../alerts/NotificationSurface';

export interface NonPrivateModelDisclosureProps {
  open: boolean;
  /** The provider the user is about to talk to, as a person would name it. */
  providerDisplayName: string;
  /**
   * The served copy. **Required** — there is no fallback string in this file,
   * and that is the point: a default here would be a second definition of the
   * sentence, and it would be the one that shipped stale.
   */
  copy: DisclosureCopy;
  onAcknowledge: () => void;
  busy?: boolean;
  /**
   * Why the acknowledgement was not recorded, when it was not.
   *
   * ⚠ Shown, and the button then says so. A daemon holding no user-action key
   * refuses this and there is nothing the person at the keyboard can present to
   * satisfy it, so the honest sequence is: tell them it was not saved, and let
   * them past — this dialog conveys a fact, it does not grant a permission.
   * Silently closing as though it had been saved is the failure that turns a
   * once-per-install disclosure into a modal they see on every launch and stop
   * reading.
   */
  acknowledgeError?: string | null;
}

/**
 * The one-time blocking disclosure (issue #56, DR-17 requirement 3).
 *
 * ⚠ **It cannot be dismissed.** No Escape, no overlay click, no close button —
 * `purpose="required"` on `ModalShell` is what says so: the shell passes
 * `dismissible={false}` to `DialogContent`, which prevents the first two and
 * suppresses the third. A disclosure a user can flick away without reading is a
 * disclosure that did not happen, and this one has no *action* to gate: there is
 * no safer alternative on offer at this moment, only a fact to convey.
 * `NonPrivateModelDisclosure.required.test.tsx` fails the day the purpose changes.
 *
 * ⚠ **No key acknowledges it.** Task 29's `DangerousConfirmDialog` established
 * the discipline and the reason is the same here: "type, Enter" ends every other
 * dialog in this app, so a dialog whose whole job is to be *read* must not be
 * dismissible by the muscle memory that skips reading. There is no `<form>`, the
 * button is `type="button"`, and initial focus is put on the dialog's body
 * rather than on the button — so the first Enter after it opens lands on
 * nothing.
 *
 * ⚠ **Every word comes from {@link copy}.** This component contains no product
 * prose. See `disclosureCopy.ts`.
 */
export function NonPrivateModelDisclosure({
  open,
  providerDisplayName,
  copy,
  onAcknowledge,
  busy = false,
  acknowledgeError = null,
}: NonPrivateModelDisclosureProps) {
  const bodyRef = useRef<HTMLDivElement>(null);

  return (
    // The shared dialog (spec 2.6, ruling 23): top anchor, header rule, a scrolling body and the
    // footer row, like every other dialog on the first run path. Privacy UI presentation only: the
    // copy, the props, the test id (`testId` puts it on the `role="dialog"` node, where it was), the
    // focus rule and the one way out are what they were.
    <ModalShell
      open={open}
      // Nothing can call this: a `required` shell draws no ×, and it refuses Escape and a press on
      // the backdrop. The dialog closes when the gate stops passing `open`, once the
      // acknowledgement settles.
      onOpenChange={() => {}}
      size="lg"
      purpose="required"
      testId="non-private-model-disclosure"
      title={disclosureTitle(copy, providerDisplayName)}
      scrollBody
      // Focus lands on the prose, not on the way out of it. Radix's default
      // would focus the first focusable node, which here is the only button —
      // and an acknowledgement one keystroke away from the dialog opening is
      // the receipt this task exists to avoid.
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        bodyRef.current?.focus();
      }}
      footer={
        <>
          {/* A failure notice in the app's notice tier, not a quieter paragraph.
              This was a bare `<p>` in `text-text-muted` — LOWER contrast than the
              explanatory body directly above it — with no icon, no colour and no
              separation, while the button silently changed from "I understand" to
              "Continue". A failure rendered less prominently than the prose it
              follows is read as more boilerplate, which is exactly the outcome
              the button relabelling is trying to avoid. `NotificationSurface` is
              the shared banner primitive; `warning`, not `error`, because the
              person is not blocked — the sentence's whole job is to say what was
              not recorded before they go on.

              It sits in the footer, on a row of its own above the button, so it
              stays outside the scrolling body, as it sat outside the old
              scroller. At the end of the scrolled prose, in a short window or at
              a large text size, it would arrive out of view while the button
              beside it changed its word. */}
          {acknowledgeError && (
            <div data-testid="disclosure-ack-error" className="w-full min-w-0">
              <NotificationSurface
                role="status"
                status="warning"
                message={acknowledgeError}
                className="min-w-0 [overflow-wrap:anywhere]"
              />
            </div>
          )}
          <Button type="button" variant="default" disabled={busy} onClick={onAcknowledge}>
            {acknowledgeError ? 'Continue' : 'I understand'}
          </Button>
        </>
      }
    >
      <div
        ref={bodyRef}
        tabIndex={-1}
        className="space-y-3 py-3 text-sm text-text-default outline-none"
      >
        <DisclosureProse text={copy.long} paragraphClassName="min-w-0 [overflow-wrap:anywhere]" />
      </div>
    </ModalShell>
  );
}
