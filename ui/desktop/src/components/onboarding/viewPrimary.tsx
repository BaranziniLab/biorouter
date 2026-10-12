import { createContext, useContext, type ReactNode } from 'react';

/**
 * One coral button per view (principle 7, the section 0 decision) across panels that open on
 * their own (vision FR-R5-2).
 *
 * The provider catalog's rows open independently (FR-R4-2), and every first-run panel ends in its
 * primary action: "Download and start" and "Use Ollama" on Local, "Use Claude Code" and "Use
 * Codex" on Public, a Versa form's "Save", the key box's "Continue". Two open panels drew two coral
 * buttons in one view. Now one participant per view holds the primary: the one the person turned
 * to last, by opening its row or by moving focus into it (typing in a field, pressing a control).
 * The others draw the same action as a secondary button, still enabled and still one click away.
 *
 * ⚠ **Decided by the catalog, at render, from what it already knows**: which rows are open, which
 * tab each is in, and the order the person turned to them. Panels never report their state up in
 * an effect. An effect-reported answer draws one commit with the old owner, and ProviderCard's
 * layout read (it measures its line) turns that commit into a coral to grey fade on a button the
 * person did not touch. So the owner can be a panel with no primary to offer right now (Ollama not
 * installed, an agent not signed in), and that view then draws no coral button: the principle asks
 * for at most one, and a colour only ever changes on the person's own open, close or focus.
 *
 * Outside a catalog (a dialog, the recovery modal, a test that mounts a card alone) a panel is its
 * own view and keeps its primary.
 */

/** The participant that holds a view's primary: the latest engaged one present, else the first. */
export function viewPrimaryOwner(
  engaged: readonly string[],
  present: readonly string[]
): string | null {
  for (let index = engaged.length - 1; index >= 0; index -= 1) {
    if (present.includes(engaged[index])) return engaged[index];
  }
  return present[0] ?? null;
}

/** `engaged` with `key` moved to the end (most recent); the same array when it already is. */
export function engageViewPrimary(engaged: readonly string[], key: string): readonly string[] {
  if (engaged[engaged.length - 1] === key) return engaged;
  return [...engaged.filter((entry) => entry !== key), key];
}

const HoldsViewPrimaryContext = createContext<boolean | null>(null);

/**
 * One panel that may hold its view's primary. The wrapper draws no box (`display: contents`); it
 * is there to hear focus move into the panel, which is the person turning to it.
 */
export function ViewPrimaryParticipant({
  holds,
  onEngage,
  children,
}: {
  holds: boolean;
  onEngage: () => void;
  children: ReactNode;
}) {
  return (
    <HoldsViewPrimaryContext.Provider value={holds}>
      <div style={{ display: 'contents' }} onFocus={onEngage}>
        {children}
      </div>
    </HoldsViewPrimaryContext.Provider>
  );
}

/**
 * The variant of a panel's primary action: coral when the action is ready and the panel holds its
 * view's primary, secondary otherwise. `ready` is the panel's own rule (a Versa form's Save is
 * coral only once its keys are in, FR-R3-3).
 */
export function useViewPrimaryVariant(ready = true): 'default' | 'secondary' {
  const holds = useContext(HoldsViewPrimaryContext) ?? true;
  return ready && holds ? 'default' : 'secondary';
}
