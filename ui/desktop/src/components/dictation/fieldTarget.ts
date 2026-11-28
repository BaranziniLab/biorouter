/**
 * Which field an fn dictation goes into, in this window (DD-98, "The fn key" in
 * `docs/dictation/surfaces.md`). Pure DOM rules, with no session state, so the fn
 * client and the session share them and a test can drive them on any document.
 *
 * The order, when fn goes down:
 *
 * 1. The focused field: the composer whose field it is, or any other editable
 *    field (a rename box, a search field, a field in Settings or a dialog,
 *    a `contenteditable` region), never a password and never a disabled or read
 *    only one.
 * 2. The live composer of the active pane (`[data-active-group="true"]`).
 * 3. The composer focused last, if its field is still live.
 * 4. The only live composer.
 *
 * Inside an open modal dialog only its own fields count, so the text never goes
 * into a composer the dialog covers. A field is **live** when it is in the
 * document, outside every `[inert]` subtree, visible (`checkVisibility` with
 * `visibilityProperty`, which also refuses a `visibility: hidden` ancestor, as a
 * compact pane's hidden chat has), and neither disabled nor read only. A message
 * editor that has closed has no field at all, so it is never chosen.
 */
import { dictationFieldKind, type DictationField } from '../../utils/dictatedText';

/** A composer the fn gestures may press, as the fn client knows it. */
export interface FieldCandidate {
  key: string;
  field: () => DictationField | null;
}

/** Fields of the terminal: xterm reads keys from a hidden textarea and draws them itself. */
const NOT_A_FIELD = '.xterm';

/** Whether `el` can be seen: `checkVisibility` where the DOM has it (Chromium), else yes. */
function visible(el: Element): boolean {
  const check = (el as Element & { checkVisibility?: (options?: object) => boolean })
    .checkVisibility;
  if (typeof check !== 'function') return true;
  try {
    return check.call(el, { visibilityProperty: true });
  } catch {
    return true;
  }
}

/**
 * Whether a field may receive dictated text now: present, connected, not inside
 * `[inert]`, visible, an editable kind (never a password), and neither disabled
 * nor read only.
 */
export function isLiveField(el: Element | null | undefined): el is DictationField {
  if (!el || !el.isConnected) return false;
  if (dictationFieldKind(el) === null) return false;
  if (el.closest('[inert]') || el.closest(NOT_A_FIELD)) return false;
  return visible(el);
}

/** An open modal dialog's box: `aria-modal="true"`, or a `<dialog>` opened with `showModal()`. */
function isOpenModal(el: Element): boolean {
  if (!el.isConnected || el.closest('[inert]')) return false;
  // A dialog animating out after it closed covers nothing any more.
  if (el.getAttribute('data-state') === 'closed') return false;
  if (el.getAttribute('aria-modal') === 'true') return visible(el);
  if (el.localName === 'dialog') {
    try {
      return el.matches(':modal');
    } catch {
      return false;
    }
  }
  return false;
}

/**
 * The open modal dialog dictation is confined to: the one that holds focus, else
 * the last one open (the one on top). Null when no modal dialog is open.
 */
export function modalScope(doc: Document = document): Element | null {
  const open = Array.from(doc.querySelectorAll('[aria-modal="true"], dialog')).filter(isOpenModal);
  if (open.length === 0) return null;
  const focused = doc.activeElement;
  const holding = focused ? open.filter((dialog) => dialog.contains(focused)) : [];
  // The innermost dialog that holds focus: a dialog opened from inside another one.
  if (holding.length > 0) return holding[holding.length - 1];
  return open[open.length - 1];
}

/** The focused element, or null when focus is on the page itself. */
function focusedElement(doc: Document): Element | null {
  const active = doc.activeElement;
  if (!active || active === doc.body || active === doc.documentElement) return null;
  return active;
}

/**
 * A field fn could dictate into that no composer owns: the focused element when it
 * is a live, editable field (never a password) inside the modal scope. A
 * `contenteditable` region is named by its editing host.
 */
export function focusedLooseField(
  candidates: readonly FieldCandidate[],
  doc: Document = document
): DictationField | null {
  const focused = focusedElement(doc);
  if (!focused) return null;
  const host = editingHost(focused);
  if (!isLiveField(host)) return null;
  const scope = modalScope(doc);
  if (scope && !scope.contains(host)) return null;
  if (candidates.some((candidate) => candidate.field() === host)) return null;
  return host;
}

/** The element whose content is edited: `el`, or the outermost editable ancestor of a region. */
function editingHost(el: Element): Element {
  if (el.localName === 'textarea' || el.localName === 'input') return el;
  let host = el;
  for (let node = el.parentElement; node; node = node.parentElement) {
    if (dictationFieldKind(node) !== 'contenteditable') break;
    host = node;
  }
  return host;
}

/** A message editor's key (`UserMessage.tsx`): a composer, but the chat's own comes first. */
const isEditorKey = (key: string): boolean => key.startsWith('editor:');

export interface FnTargetInput {
  candidates: readonly FieldCandidate[];
  /** The composer whose field had focus last, or null. */
  lastFocused: string | null;
  doc?: Document;
}

/**
 * Which registered composer a new fn dictation goes into (rules 1 to 4 above, for
 * composers), or null. A focused field no composer owns is
 * {@link focusedLooseField}'s, and the caller asks that first.
 */
export function chooseFnComposer({
  candidates,
  lastFocused,
  doc = document,
}: FnTargetInput): string | null {
  const scope = modalScope(doc);
  const live = candidates.filter((candidate) => {
    const field = candidate.field();
    return isLiveField(field) && (!scope || scope.contains(field));
  });
  if (live.length === 0) return null;

  // 1. The composer whose field has focus.
  const focused = focusedElement(doc);
  if (focused) {
    const owner = live.find((candidate) => {
      const field = candidate.field();
      return field === focused || (field !== null && field.contains(focused));
    });
    if (owner) return owner.key;
  }

  // 2. The active pane's composer: the one focused last when it is there, else the chat's own.
  const pane = doc.querySelector('[data-active-group="true"]');
  if (pane) {
    const inPane = live.filter((candidate) => pane.contains(candidate.field()));
    const last = inPane.find((candidate) => candidate.key === lastFocused);
    if (last) return last.key;
    const chat = inPane.find((candidate) => !isEditorKey(candidate.key));
    if (chat) return chat.key;
    if (inPane.length > 0) return inPane[0].key;
  }

  // 3. The composer focused last, while its field is live.
  if (lastFocused) {
    const last = live.find((candidate) => candidate.key === lastFocused);
    if (last) return last.key;
  }

  // 4. The only live composer.
  return live.length === 1 ? live[0].key : null;
}

/**
 * The composer that shows a notice about a dictation whose own field is gone
 * (MSG-INSERT-LOST, `dictationSession.ts`), or null when none is live: the active
 * pane's chat composer first, as {@link chooseFnComposer} would pick with nothing
 * focused, else any live composer. `prefer` narrows the choice to composers that
 * can show the notice's actions, when any of them is live. An open modal dialog does
 * not narrow it: a notice behind the dialog is there when the dialog closes, where
 * text typed behind it would not be seen.
 */
export function visibleComposer(
  candidates: readonly FieldCandidate[],
  prefer: (key: string) => boolean = () => true,
  doc: Document = document
): string | null {
  const live = candidates.filter((candidate) => isLiveField(candidate.field()));
  const preferred = live.filter((candidate) => prefer(candidate.key));
  const pool = preferred.length > 0 ? preferred : live;
  if (pool.length === 0) return null;
  const pane = doc.querySelector('[data-active-group="true"]');
  const inPane = pane ? pool.filter((candidate) => pane.contains(candidate.field())) : [];
  const ordered = [...inPane, ...pool];
  return (ordered.find((candidate) => !isEditorKey(candidate.key)) ?? ordered[0]).key;
}
