/**
 * Put back a Tab that the dialog's own focus trap cancelled.
 *
 * ## The bug this exists for
 *
 * Measured in Chromium on the **New schedule** dialog: twelve Tab presses
 * visited only Name → Browse → "Repeat every" → the dialog container, over and
 * over. Cancel, "Create schedule", the × and the three time selects were never
 * focused. The trap itself was fine — the actions were simply never reached.
 *
 * The mechanism is an interaction between two libraries, and neither is wrong on
 * its own:
 *
 * 1. `react-select` renders its `aria-live` announcements as children that exist
 *    only while the control is focused (`isFocused && <ScreenReaderText/>`).
 *    Blur unmounts those four `<span>`s **synchronously**, inside the `focusout`
 *    dispatch, because React flushes discrete events without batching.
 * 2. Radix's `FocusScope` watches the dialog subtree with a `MutationObserver`
 *    to catch "the focused element was removed, so the browser dropped focus on
 *    `<body>`" — and answers it by focusing the dialog container.
 *
 * Between a `focusout` and the matching `focusin`, `document.activeElement` is
 * `<body>`, and the JS stack empties — so the observer's microtask runs *inside*
 * that window, sees removals with focus apparently on `<body>`, and parks focus
 * on the container. The Tab the browser was in the middle of delivering is lost,
 * and every control after the first `Select` in the dialog becomes unreachable.
 *
 * This is not specific to the schedule dialog: any dialog holding a `Select`
 * (the model picker, the provider modals, lead/worker settings) has it, which is
 * why the repair lives on the dialog primitive rather than in `CronPicker`.
 *
 * ## The repair
 *
 * The browser told us where the Tab was going — `focusout.relatedTarget`. If the
 * dialog container then takes focus itself, that is the park above, and focus is
 * handed back to the element the browser had chosen.
 *
 * Three details are load-bearing, and each was measured rather than reasoned:
 *
 * - **The restore is deferred to a microtask.** Radix's observer parks focus
 *   *once per mutation record* — react-select's blur produces four — so a
 *   restore performed inside the first park's `focusin` is simply overwritten by
 *   the next three. Deferring puts it after the whole loop, where it sticks; a
 *   later observer batch then sees a focused control and stands down by itself.
 * - **Landing on the intended control does not end the episode**, and neither
 *   does blurring *towards the container*. Both happen while the park and the
 *   browser's own transfer interleave, and treating either as "focus moved, we
 *   are done" makes the repair a no-op for exactly the sequence it exists for.
 * - **A genuine removal is left alone.** When the focused element really is
 *   removed, the blur carries no `relatedTarget` — focus fell to the document —
 *   so nothing is remembered and Radix's park stands, which is what it is for.
 *
 * The intent is dropped at the end of the task, so a container focus arriving
 * later — a click on dialog chrome, say — can never be answered with a stale
 * target.
 */

/** How many times one Tab may be put back before the repair gives up. */
const MAX_RESTORES_PER_EPISODE = 3;

/**
 * Watch `container` for the cancelled-Tab pattern and undo it.
 *
 * Returns the teardown. Safe to call with any element; it touches nothing until
 * focus moves.
 */
export function installDialogTabRepair(container: HTMLElement): () => void {
  let intended: HTMLElement | null = null;
  let forgetTimer: ReturnType<typeof setTimeout> | null = null;
  let restorePending = false;
  let restores = 0;

  const forget = () => {
    intended = null;
    restores = 0;
    if (forgetTimer !== null) {
      clearTimeout(forgetTimer);
      forgetTimer = null;
    }
  };

  const scheduleRestore = () => {
    if (!intended || restorePending) return;
    restorePending = true;
    queueMicrotask(() => {
      restorePending = false;
      const target = intended;
      if (!target) return;
      // Something other than the park won the race; whatever it was, it is a
      // better answer than a focus we inferred.
      if (document.activeElement !== container) return;
      if (!target.isConnected || !container.contains(target)) {
        forget();
        return;
      }
      // A cap, not a rhythm: an endless volley is a bug, not something a user
      // should have to sit through.
      if (restores >= MAX_RESTORES_PER_EPISODE) {
        forget();
        return;
      }
      restores += 1;
      target.focus();
    });
  };

  const handleFocusOut = (event: FocusEvent) => {
    const next = event.relatedTarget;
    // The park taking focus back mid-episode. Keep the destination: answering it
    // is the whole job.
    if (next === container && intended) return;
    if (!(next instanceof HTMLElement) || next === container || !container.contains(next)) {
      forget();
      return;
    }
    intended = next;
    if (forgetTimer === null) forgetTimer = setTimeout(forget, 0);
  };

  const handleFocusIn = (event: FocusEvent) => {
    if (event.target !== container) {
      if (event.target !== intended) forget();
      return;
    }
    // The dialog itself has focus. Nothing in the app focuses the container
    // deliberately except Radix — on open, when there is no destination
    // remembered, and on the park this undoes.
    scheduleRestore();
  };

  container.addEventListener('focusout', handleFocusOut);
  container.addEventListener('focusin', handleFocusIn);

  return () => {
    forget();
    container.removeEventListener('focusout', handleFocusOut);
    container.removeEventListener('focusin', handleFocusIn);
  };
}
