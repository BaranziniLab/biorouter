import React from 'react';
import ReactSelect, {
  components as selectComponents,
  type ClearIndicatorProps,
  type DropdownIndicatorProps,
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
 * The trigger is visually identical to <Input>: same height, radius, border and
 * focus treatment. The menu is the shared popover surface.
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
        ...props.components,
      }}
      unstyled
      isSearchable={props.isSearchable !== false}
      closeMenuOnSelect={props.closeMenuOnSelect !== false}
      blurInputOnSelect={props.blurInputOnSelect !== false}
      classNames={{
        container: () => 'w-full cursor-pointer relative',
        indicatorSeparator: () => 'h-0',
        // The trigger IS the <Input> chrome, class for class (§3.2): the 32px md
        // rung, 8px text inset, the `--border-control` edge (set in `styles.control`
        // below, where Emotion writes it unlayered, as `.br-field` does for Input) and the
        // same inset-ring hover whisper. react-select gives us no `:focus` to
        // hang the global rule on — it moves focus to a hidden input — so the
        // focused branch reproduces main.css's surface shift by hand, and stands
        // the whisper down at the same time so the two never stack.
        control: ({ isFocused, isDisabled }) =>
          [
            'flex h-control-md w-full items-center rounded-element border bg-background-default px-2',
            'text-label text-text-default transition-[color,background-color,border-color,box-shadow]',
            isDisabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer',
            isFocused ? 'border-border-focus bg-background-focus' : '',
            !isDisabled && !isFocused
              ? 'hover:inset-ring-2 hover:inset-ring-border-emphasized/30'
              : '',
          ].join(' '),
        placeholder: () => 'text-text-muted',
        singleValue: () => 'text-text-default',
        input: () => 'text-text-default',
        dropdownIndicator: () => 'text-text-muted transition-transform',
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
        // The edge is set here too (spec 2.3): `--border-control` at rest, `--border-focus` while
        // focused. An unlayered declaration, like `.br-field` on Input, so no utility can pull it
        // back to the 1.6:1 `--border-emphasized`.
        control: (base, state) => ({
          ...base,
          minHeight: 0,
          borderColor: state.isFocused ? 'var(--border-focus)' : 'var(--border-control)',
        }),
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
