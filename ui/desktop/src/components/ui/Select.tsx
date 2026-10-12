import React from 'react';
import ReactSelect, {
  components as selectComponents,
  type ClearIndicatorProps,
  type DropdownIndicatorProps,
  type MenuProps,
  type OptionProps,
} from 'react-select';
import { CheckIcon, ChevronDown, X } from '../icons/app-icons';
import {
  MENU_GROUP_LABEL_CLASS_NAME,
  MENU_ROW_CLASS_NAME,
  MENU_ROW_TRAILING_CLASS_NAME,
} from './dropdown-menu';

/**
 * A select option row: the one menu row (spec 2.6) plus a pointer cursor. A 32px FLOOR, never
 * a fixed height (see Select.optionGeometry.test.ts for the two-line spill that rule prevents),
 * with vertical padding so a wrapped row keeps its breathing room.
 */
export const SELECT_OPTION_CLASS_NAME = `${MENU_ROW_CLASS_NAME} cursor-pointer`;

/**
 * The shared select (design.md §4.8).
 *
 * The trigger is visually identical to <Input>: same height, radius, fill and
 * focus edge (the filled field of R-27). The menu is the shared popover surface.
 *
 * Layering: every call site lives inside a modal, and the menu is NOT portalled by
 * default — react-select renders it as an absolutely-positioned sibling. It
 * therefore needs to out-rank the modal surface (--z-modal, 400) without
 * out-ranking a toast (--z-toast, 600). It previously used z-[9999], which painted
 * over the dialog's own close button.
 *
 * ⚠ **z-index does not answer `overflow: hidden`, and that is a different bug with
 * the same symptom.** A call site inside a CLIPPING ancestor — `ModalShell`'s
 * `DialogContent` is `overflow-hidden`, and its scrolling body is `overflow-y-auto`
 * — has its menu cut off at the box's edge however high the menu is stacked; the
 * Scheduler's cron picker showed exactly one of six options. Such a call site
 * passes `menuPortalTarget={document.body}` (with `menuPlacement="auto"`), and the
 * `menuPortal` entry in `styles` below carries the same tier out to the portal so
 * the escaped menu still lands above the dialog and below a toast. It is inert
 * until a call site opts in, so the four non-clipped call sites are unaffected.
 *
 * `unstyled` strips react-select's Emotion base styles, but Emotion still injects
 * `fontSize: inherit` on options AFTER Tailwind's layer, so an option's own
 * `text-secondary` (from the menu row) loses and the option takes the menu's size.
 * The menu therefore carries the menu row's role itself, `text-secondary` (13/18):
 * the inherit chain resolves to the one menu size every other menu has (spec 2.6,
 * vision round 1, V4-11: the select's 14px options sat beside 13px DropdownMenu rows
 * in the same Settings flow), not to whatever the portal target inherits.
 * The chosen option is marked by the trailing check and its wash, never by weight.
 *
 * D-08 selected a full migration to Radix Select + Command. That is staged
 * separately: three of the four call sites are plain selects, but the model
 * picker in SwitchModelModal is a typeahead combobox (onInputChange/inputValue)
 * which Radix Select cannot express without `cmdk`.
 */
// --z-modal-dropdown, deliberately NOT --z-dropdown (200): react-select renders its
// menu into a portal, so it is a SIBLING of any dialog content (--z-modal, 400) that
// hosts it. LeadWorkerSettings and SwitchModelModal both put a Select inside a
// DialogContent — this tier is exactly what stops the menu painting under them.
const MENU_Z = 500;

/**
 * The trigger's motion, the <Input> field's own list on the motion tokens (spec 2.6): colour,
 * background and border over `--dur-fast` on `--ease-out`. It is set in `styles` and not left to
 * the class, because react-select's base control style (emitted even when `unstyled`) carries an
 * unlayered Emotion `transition: all 100ms` that beat the authored
 * `transition-[color,background-color,border-color]`: the trigger animated EVERY property over an
 * off-token 100ms (first run, FR-R3-5). The indicators' base `color 150ms` is restated the same way.
 */
const SELECT_CONTROL_TRANSITION = ['color', 'background-color', 'border-color']
  .map((property) => `${property} var(--dur-fast) var(--ease-out)`)
  .join(', ');
const SELECT_INDICATOR_TRANSITION = 'color var(--dur-fast) var(--ease-out)';

/**
 * A pick leaves focus on the Select (vision round 8, V3-R8-2), as a Radix control returns focus to
 * its trigger. This wrapper used to force react-select's `blurInputOnSelect` ON for every caller
 * (the library's own default is ON only on a touch device), so a pick blurred the field: inside a
 * dialog, Radix's focus trap then caught the escaping focus and parked it on the dialog box, which
 * shows no ring by design (a quiet open, `.biorouter-modal-surface:focus`). In New schedule,
 * ArrowDown and Enter on "AM or PM" picked AM and left focus nowhere a person could see; the next
 * Tab restarted at Name at the top of the form instead of reaching Cancel. A mouse pick did the
 * same. So the blur is OFF unless a caller asks for it with `blurInputOnSelect`, and no caller does.
 * The field keeps its focus edge after the pick and Tab goes on to the next control.
 */
const SELECT_BLUR_INPUT_ON_SELECT = false;

/** The hook on every shared Select's container, for the Escape claim below. */
const SELECT_CONTAINER_CLASS_NAME = 'br-select';

/**
 * Escape closes the innermost open surface only (APG; vision round 2, V4-R2-2).
 *
 * With Switch models' provider menu open, one Escape removed the menu AND the dialog, so the
 * choice in progress was lost, while a Radix menu in another dialog closed only itself. A
 * react-select menu is not a Radix layer, so the dialog is the top layer, and Radix listens for
 * Escape in the CAPTURE phase on the document: it dismisses the dialog before react-select's own
 * handler (bubbling, at React's root) sees the key. Stopping propagation inside the Select would
 * therefore change nothing.
 *
 * So the Select claims the key first: a window capture listener (the window's capture runs before
 * the document's) cancels an Escape whose target is THIS component's combobox while its menu is
 * open (`aria-expanded="true"`, which react-select writes on its input and on its non-searchable
 * dummy input alike). Radix leaves a cancelled Escape alone (`onEscapeKeyDown`, then dismiss only
 * when not `defaultPrevented`), and so does every other surface that checks it; react-select then
 * closes its menu as before and focus stays on the field. A second Escape, with the menu closed,
 * reaches the dialog. The claim is scoped by the container class, because another combobox may say
 * expanded for as long as it is mounted (cmdk's input does), and its Escape must still close a
 * command palette's dialog.
 *
 * ⚠ react-select returns early on a cancelled key when a caller passes `onKeyDown` (it checks
 * `defaultPrevented` after calling it), so a caller with its own `onKeyDown` would keep the menu
 * open on Escape. None passes one; a caller that needs one handles Escape itself.
 *
 * One listener for the document, installed when this module loads, as `Tooltip`'s Tab tracker is.
 */
function installEscapeClaim() {
  if (typeof window === 'undefined') return;
  window.addEventListener(
    'keydown',
    (event) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target.getAttribute('aria-expanded') !== 'true') return;
      if (!target.closest(`.${SELECT_CONTAINER_CLASS_NAME}`)) return;
      event.preventDefault();
    },
    true
  );
}

installEscapeClaim();

/**
 * The chosen option carries a 16px check at the trailing edge, as a selected radio item does in
 * every other menu (spec 2.6, principle 5), so the choice reads without leaning on a wash that a
 * highlighted row shares. Decorative: react-select already sets `aria-selected`. The label sits
 * in a `min-w-0 flex-1` box so a long or two-line label wraps instead of pushing the check out.
 */
function SelectOption(props: OptionProps) {
  return (
    <selectComponents.Option {...props}>
      <div className="min-w-0 flex-1">{props.children}</div>
      {props.isSelected ? (
        <span className={MENU_ROW_TRAILING_CLASS_NAME} data-slot="menu-item-indicator">
          <CheckIcon className="size-4 text-text-default" aria-hidden />
        </span>
      ) : null}
    </selectComponents.Option>
  );
}

/**
 * The chevron and the clear mark are the app's own 16px, stroke-1.5 glyphs, not the library's
 * filled `DownChevron` and `CrossIcon`, which drew a heavier pair than every other control in
 * the app (vision V4-28, V3-20: Switch models' two selects and the workflow Schedule dialog).
 * Passing `children` is react-select's own seam (`children || <DownChevron />`), so the
 * indicator keeps its handlers, its `aria-hidden` and its `isDisabled` state.
 */
function SelectDropdownIndicator(props: DropdownIndicatorProps) {
  return (
    <selectComponents.DropdownIndicator {...props}>
      <ChevronDown className="size-4" aria-hidden />
    </selectComponents.DropdownIndicator>
  );
}

/**
 * The open menu names itself the way the Radix menus do: `data-slot="select-content"` with
 * `data-state="open"` (the name a Radix Select would carry, D-08). react-select mounts the menu
 * only while it is open, so the state never reads anything else. The toast layer reads these two
 * attributes ("toast yields to an open menu" in main.css): a menu the person opened stands above
 * a standing toast, as a dropdown, context menu or popover does (WS-MOTION, V1-R7-1).
 */
export const SELECT_MENU_SLOT = 'select-content';

function SelectMenu(props: MenuProps) {
  const slot: Record<string, string> = { 'data-slot': SELECT_MENU_SLOT, 'data-state': 'open' };
  return <selectComponents.Menu {...props} innerProps={{ ...props.innerProps, ...slot }} />;
}

function SelectClearIndicator(props: ClearIndicatorProps) {
  return (
    <selectComponents.ClearIndicator {...props}>
      <X className="size-4" aria-hidden />
    </selectComponents.ClearIndicator>
  );
}

export const Select = (props: React.ComponentProps<typeof ReactSelect>) => {
  return (
    <ReactSelect
      {...props}
      components={{
        Option: SelectOption,
        DropdownIndicator: SelectDropdownIndicator,
        ClearIndicator: SelectClearIndicator,
        Menu: SelectMenu,
        ...props.components,
      }}
      unstyled
      isSearchable={props.isSearchable !== false}
      closeMenuOnSelect={props.closeMenuOnSelect !== false}
      blurInputOnSelect={props.blurInputOnSelect ?? SELECT_BLUR_INPUT_ON_SELECT}
      classNames={{
        container: () => `${SELECT_CONTAINER_CLASS_NAME} w-full cursor-pointer relative`,
        indicatorSeparator: () => 'h-0',
        // The trigger IS the <Input> chrome, class for class (§3.2): the 32px md
        // rung, 8px text inset, and the filled field of R-27 (design.md 9.11).
        // react-select gives us no `:focus` to hang `.br-field` on — it moves
        // focus to a hidden input — so the fill and the edge are set by state in
        // `styles.control` below.
        control: ({ isDisabled }) =>
          [
            'flex h-control-md w-full items-center rounded-element border px-2',
            'text-label text-text-default transition-[color,background-color,border-color]',
            isDisabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
          ].join(' '),
        placeholder: () => 'text-text-muted',
        singleValue: () => 'text-text-default',
        input: () => 'text-text-default',
        dropdownIndicator: () => 'text-text-muted',
        // `unstyled` leaves the clear mark with no padding; 4px each side keeps a 24px target
        // beside the chevron, and it warms to the default ink under the pointer like a ghost icon.
        clearIndicator: () => 'cursor-pointer px-1 text-text-muted hover:text-text-default',
        // --radius-container (12px), matching every other floating surface (§4.5). It opens the
        // way every other menu does (`.br-select-menu` in main.css: opacity, scale .97 and 4px
        // toward the side it hangs from, over `--dur-fast-max`); react-select unmounts it on
        // close, so it has no exit.
        menu: ({ placement }) =>
          `biorouter-popover-surface mt-1.5 bg-background-default rounded-container text-secondary select__menu absolute br-select-menu${
            placement === 'top' ? ' br-select-menu--top' : ''
          }`,
        menuList: () => 'max-h-60 overflow-y-auto p-1',
        // `text-caps` carries the uppercase transform itself — a second
        // `uppercase` beside it is the redundancy the type roles exist to end.
        // A menu group label: 12/16 at 500, muted, sentence case (spec 2.6).
        groupHeading: () => MENU_GROUP_LABEL_CLASS_NAME,
        noOptionsMessage: () => 'px-3 py-1.5 text-secondary text-text-muted',
        option: ({ isFocused, isSelected, isDisabled }) => {
          // 32px rows at 8px inset — the shared menu-row geometry (§3.8), which
          // is also what makes the option row and the trigger above it agree.
          //
          // `min-h-control-md`, NOT `h-control-md`. A fixed height is only correct while every option
          // is one line. `formatOptionLabel` call sites (the model picker) render a
          // title plus a wrapped detail line — ~65px of content — and a fixed 32px
          // box does not clip it, because the option is `overflow: visible`: it
          // SPILLS over the rows beneath. The spilled content still hit-tests, so
          // `elementFromPoint` at row N's centre returns row N-1 and the user
          // silently selects the model above the one they clicked. `min-h` keeps
          // the 32px rung for single-line rows and lets a two-line row own its
          // real height. Guarded by Select.optionGeometry.test.ts.
          //
          // Spec 2.6: this is the one menu row every menu shares (`MENU_ROW_CLASS_NAME`).
          const base = SELECT_OPTION_CLASS_NAME;
          if (isDisabled) return `${base} opacity-50 cursor-not-allowed pointer-events-none`;
          if (isSelected) {
            // A selected row is emphasised, not inverted. It used to fill with
            // bg-background-accent — a near-black block in a menu of neutral rows.
            //
            // `tint-selected tint-interactive` is the pair, not two independent
            // choices: on its own the selected wash (14%) would be REPLACED by a
            // plain hover (5%) and the row would visibly un-select under the
            // pointer. The pair rule in main.css resolves it to 19% instead.
            return `${base} tint-selected tint-interactive text-text-default pointer-events-auto`;
          }
          // `isFocused` is react-select's keyboard/pointer highlight — a state
          // class, not a pseudo — so it cannot come from `tint-interactive`.
          // The row has no fill of its own, so a translucent background COLOUR is
          // safe here: there is nothing for it to replace.
          if (isFocused) return `${base} bg-overlay-hover text-text-default pointer-events-auto`;
          return `${base} text-text-default tint-interactive pointer-events-auto`;
        },
      }}
      menuShouldBlockScroll={false}
      menuShouldScrollIntoView={false}
      tabSelectsValue={true}
      openMenuOnFocus={false}
      styles={{
        // `unstyled` does NOT strip `minHeight: spacing.controlHeight` (38px) from
        // react-select's own control CSS — that line sits *outside* the `unstyled ? {}`
        // branch in the library's `control` style function, so it is emitted either way.
        // Emotion injects it unlayered, Tailwind's `h-control-md` lives in `@layer utilities`, and
        // min-height beats height regardless: the trigger measured 38px against a 32px
        // class, six pixels off the <Input> rung it is class-for-class identical to.
        //
        // Standing the floor down is the whole fix — the `h-control-md` above is then the single
        // source of truth for the rung, exactly as the classNames block already claims.
        //
        // ⚠ jsdom cannot catch a regression here: it has no layout engine, so a test that
        // renders this and reads `height` sees nothing either way. `Select.test.ts`
        // asserts the override AT THE SOURCE instead of pretending to measure.
        //
        // The filled field (R-27), in Emotion's unlayered rules so no utility can put a resting
        // line back: the control ground at rest (`--background-fill`, or the page ground inside a
        // filled box, as `.br-field` resolves it), a 1px border kept TRANSPARENT (forced colours
        // still paint it in the system ink), `--border-focus` while focused (a field firms an edge
        // under D-15), and `--background-fill-strong` under the pointer unless the trigger is
        // focused or disabled, as a focused field keeps its fill while the person types.
        control: (base, state) => ({
          ...base,
          minHeight: 0,
          transition: SELECT_CONTROL_TRANSITION,
          borderColor: state.isFocused ? 'var(--border-focus)' : 'transparent',
          backgroundColor: 'var(--br-control-ground, var(--background-fill))',
          ...(state.isFocused || state.isDisabled
            ? {}
            : { '&:hover': { backgroundColor: 'var(--background-fill-strong)' } }),
        }),
        // The chevron never moves, so its only motion is colour; the clear mark warms under the
        // pointer. Both on the tokens, over react-select's base `color 150ms`.
        dropdownIndicator: (base) => ({ ...base, transition: SELECT_INDICATOR_TRANSITION }),
        clearIndicator: (base) => ({ ...base, transition: SELECT_INDICATOR_TRANSITION }),
        menu: (base) => ({ ...base, pointerEvents: 'auto', zIndex: MENU_Z }),
        // The same tier, carried out to a portalled menu. react-select's own
        // `menuPortal` default is `zIndex: 1`, which at `document.body` sits
        // under the modal scrim — so a menu that escaped its clipping ancestor
        // would then be painted over by the dialog it escaped.
        menuPortal: (base) => ({ ...base, zIndex: MENU_Z }),
        menuList: (base) => ({
          ...base,
          maxHeight: '240px',
          overflowY: 'auto',
          pointerEvents: 'auto',
        }),
        // react-select's own option rule is `display: block`, unlayered, and would beat the
        // row's `flex`: the label and the trailing check would stack instead of sharing a line.
        option: (base) => ({ ...base, display: 'flex', pointerEvents: 'auto', cursor: 'pointer' }),
      }}
    />
  );
};
