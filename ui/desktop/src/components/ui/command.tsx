import * as React from 'react';
import { cn } from '../../utils';
import { MENU_GROUP_LABEL_CLASS_NAME, MENU_ROW_CLASS_NAME } from './dropdown-menu';
import { Search } from '../icons/app-icons';

/**
 * The searchable picker — a pinned text field over a filtered list.
 *
 * ⚠ **This is deliberately NOT a `DropdownMenu` with an `<Input>` in it.**
 * Radix's `DropdownMenu` implements the ARIA *menu* pattern, which includes
 * typeahead: printable characters are captured at the menu root and move focus
 * to the matching item. A nested text field then competes for every keystroke —
 * characters are swallowed, or focus jumps out of the field mid-word — and the
 * arrow keys belong to the menu's roving focus, so the field cannot be exited
 * predictably. Five surfaces in the Knowledge section need this shape
 * (knowledge-base/okf-migration/ui-spec.md §4.0), so specifying it as a menu
 * would have replicated the defect five times.
 *
 * What this is instead is the **combobox / listbox** pattern: DOM focus stays in
 * the field, the list is a `role="listbox"` of `role="option"` rows, and the
 * highlighted row is named by `aria-activedescendant` rather than focused. Arrow
 * keys, Home/End and Enter are handled on the field; Escape is left to bubble,
 * so the `Popover` this normally lives inside closes it.
 *
 * ⚠ **No `cmdk`.** The specification names cmdk as the implementation and cmdk
 * is not a dependency of this app (checked: absent from `package.json` and from
 * `node_modules`). Rather than add a runtime dependency the design explicitly
 * budgets elsewhere, the pattern is implemented here against the app's own
 * primitives. The exported shape is deliberately cmdk-compatible —
 * `CommandInput` / `CommandList` / `CommandGroup` / `CommandItem` /
 * `CommandEmpty` / `CommandSeparator` — so adopting cmdk later is a swap of this
 * file and not of its call sites.
 *
 * **Filtering is the caller's job**, but the QUERY lives here. The caller
 * already holds the collection and usually needs the filtered set for other
 * reasons (a count, an empty state, a "create ‹query›" affordance) — a second,
 * hidden filter inside the primitive is how a list ends up disagreeing with the
 * number printed above it. The query is owned here because the highlight has to
 * be repaired whenever it changes, and a repair keyed on state this component
 * cannot see is a repair that does not run.
 */

interface CommandContextValue {
  query: string;
  setQuery: (value: string) => void;
  activeId: string | null;
  setActiveId: (id: string | null) => void;
}

const CommandContext = React.createContext<CommandContextValue | null>(null);

function useCommandContext(component: string): CommandContextValue {
  const ctx = React.useContext(CommandContext);
  if (!ctx) {
    throw new Error(`<${component}> must be rendered inside <Command>`);
  }
  return ctx;
}

export interface CommandProps extends Omit<React.ComponentProps<'div'>, 'onSelect'> {
  /** Announced name for the listbox. */
  label: string;
  /** The search text. */
  query: string;
  onQueryChange: (value: string) => void;
}

export function Command({
  label,
  query,
  onQueryChange,
  className,
  children,
  ...props
}: CommandProps) {
  const [activeId, setActiveId] = React.useState<string | null>(null);
  const rootRef = React.useRef<HTMLDivElement>(null);

  // Whose highlight this is. `false` means nobody has expressed a preference and
  // the component is showing its default — the first row.
  //
  // ⚠ **A default highlight has to FOLLOW the first row, not merely be seeded
  // onto it once.** Both callers open over a collection that is still in flight
  // and a couple of rows that are present from the first paint —
  // `KBSelectorMenu`'s "Manage bases… / Create knowledge base…",
  // `IngestModelPicker`'s groups under a `fetchModelsForProviders`. Seeding
  // against the rows that happened to exist at mount and then keeping that id
  // because it still *exists* parks the highlight on an action row, and Enter as
  // the first keystroke fires that action instead of committing the row the user
  // is looking at. This is not scoped to "the picker opened", because the
  // primitive is unmounted and remounted by the `Popover` that owns it — opening
  // IS mounting, which is exactly the moment the list is at its emptiest.
  const ownership = React.useRef({ userMoved: false });

  /** Move the highlight *on the user's behalf* — arrows, Home/End, the pointer. */
  const chooseActiveId = React.useCallback((id: string | null) => {
    ownership.current.userMoved = true;
    setActiveId(id);
  }, []);

  const visibleIds = React.useCallback((): string[] => {
    const root = rootRef.current;
    if (!root) return [];
    return Array.from(root.querySelectorAll<HTMLElement>('[data-command-item]'))
      .filter((el) => el.getAttribute('aria-disabled') !== 'true')
      .map((el) => el.id);
  }, []);

  // Put the highlight on a row that exists, and on the FIRST row until the user
  // moves it. It reads the DOM the render just produced, because that is the
  // only place that knows what the CALLER's filter — and the caller's
  // still-arriving collection — left behind.
  const repair = React.useCallback(() => {
    const ids = visibleIds();
    setActiveId((prev) => {
      if (ids.length === 0) return null;
      return ownership.current.userMoved && prev !== null && ids.includes(prev) ? prev : ids[0];
    });
  }, [visibleIds]);

  // A new query is a new question, so the answer starts at the top again. This
  // also seeds the highlight on mount.
  React.useEffect(() => {
    ownership.current.userMoved = false;
    repair();
  }, [query, repair]);

  // ⚠ **The set of rows is not a value this component is given**, so it is
  // OBSERVED rather than depended on. It is whatever the caller rendered, and it
  // changes without `query` or `activeId` changing — which is exactly how a
  // highlight seeded against a still-loading list survived the list arriving.
  // `class` is deliberately outside the filter: the highlight itself is painted
  // with a class, so watching it would make every repair schedule another.
  React.useEffect(() => {
    const root = rootRef.current;
    if (!root || typeof MutationObserver !== 'function') return;
    const observer = new MutationObserver(repair);
    observer.observe(root, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['aria-disabled'],
    });
    return () => observer.disconnect();
  }, [repair]);

  /** The highlighted row, resolved against what is actually on screen now. */
  function effectiveActive(ids: string[]): string | null {
    if (ids.length === 0) return null;
    return activeId && ids.includes(activeId) ? activeId : ids[0];
  }

  function move(delta: number | 'first' | 'last') {
    const ids = visibleIds();
    const current = effectiveActive(ids);
    if (current === null) return;
    const index = ids.indexOf(current);
    const next =
      delta === 'first'
        ? 0
        : delta === 'last'
          ? ids.length - 1
          : (index + delta + ids.length) % ids.length;
    const id = ids[next];
    chooseActiveId(id);
    document.getElementById(id)?.scrollIntoView?.({ block: 'nearest' });
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        move(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        move(-1);
        break;
      case 'Home':
        event.preventDefault();
        move('first');
        break;
      case 'End':
        event.preventDefault();
        move('last');
        break;
      case 'Enter': {
        const id = effectiveActive(visibleIds());
        if (!id) return;
        event.preventDefault();
        document.getElementById(id)?.click();
        break;
      }
      // Escape is NOT handled: it belongs to the Popover that owns this.
      default:
        break;
    }
  }

  return (
    <CommandContext.Provider
      value={{ query, setQuery: onQueryChange, activeId, setActiveId: chooseActiveId }}
    >
      <div
        ref={rootRef}
        data-command-root
        data-command-label={label}
        onKeyDown={onKeyDown}
        className={cn('flex min-h-0 flex-col', className)}
        {...props}
      >
        {children}
      </div>
    </CommandContext.Provider>
  );
}

export type CommandInputProps = Omit<React.ComponentProps<'input'>, 'value' | 'onChange'>;

export function CommandInput({ className, placeholder, ...props }: CommandInputProps) {
  const { query, setQuery, activeId } = useCommandContext('CommandInput');

  return (
    // The field's own 32px, from the Input primitive's geometry. Sub-32px fields
    // do not exist in this app and adding a `size` variant to `Input` is a
    // design-system change this surface does not own (ui-spec §4.0).
    <div className="flex h-control-md flex-none items-center gap-2 border-b border-border-subtle px-2">
      <Search className="h-icon-row w-icon-row shrink-0 text-text-muted" aria-hidden="true" />
      <input
        type="text"
        role="combobox"
        aria-expanded
        aria-autocomplete="list"
        aria-activedescendant={activeId ?? undefined}
        value={query}
        placeholder={placeholder}
        onChange={(event) => setQuery(event.target.value)}
        // Focus is global (D-15) and this field's ground IS the popover, so a
        // bordered box would be a second field edge inside a surface that
        // already has one.
        className={cn(
          'min-w-0 flex-1 bg-transparent text-label text-text-default outline-none placeholder:text-text-muted',
          className
        )}
        {...props}
      />
    </div>
  );
}

export function CommandList({ className, children, ...props }: React.ComponentProps<'div'>) {
  return (
    <div role="listbox" className={cn('min-h-0 flex-1 overflow-y-auto p-1', className)} {...props}>
      {children}
    </div>
  );
}

export function CommandGroup({
  heading,
  className,
  children,
  ...props
}: React.ComponentProps<'div'> & { heading?: React.ReactNode }) {
  const headingId = React.useId();
  return (
    <div role="group" aria-labelledby={heading ? headingId : undefined} {...props}>
      {heading ? (
        <div id={headingId} className={MENU_GROUP_LABEL_CLASS_NAME}>
          {heading}
        </div>
      ) : null}
      <div className={cn('flex flex-col', className)}>{children}</div>
    </div>
  );
}

export function CommandSeparator({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div role="separator" className={cn('my-1 h-px bg-border-subtle', className)} {...props} />
  );
}

export interface CommandItemProps extends Omit<React.ComponentProps<'div'>, 'onSelect'> {
  onSelect: () => void;
  /** Marks the row as the current VALUE (a check), not as the highlight. */
  selected?: boolean;
  disabled?: boolean;
}

export function CommandItem({
  onSelect,
  selected = false,
  disabled = false,
  className,
  children,
  ...props
}: CommandItemProps) {
  const id = React.useId();
  const { activeId, setActiveId } = useCommandContext('CommandItem');
  const active = activeId === id;

  return (
    <div
      id={id}
      role="option"
      data-command-item=""
      aria-selected={selected}
      aria-disabled={disabled || undefined}
      // The pointer moves the highlight rather than painting a separate hover
      // state, so the keyboard's notion of "current row" and the pointer's are
      // one thing.
      onMouseMove={() => {
        if (!disabled && !active) setActiveId(id);
      }}
      onClick={() => {
        if (!disabled) onSelect();
      }}
      className={cn(
        // The one menu row (spec 2.6): 32px minimum, 12px inset, radius 8 (12 − 4 of the
        // popover's padding), 13/18, the `--overlay-hover` highlight. A minimum, not a fixed
        // height, so a row with a second muted line owns its height instead of spilling.
        MENU_ROW_CLASS_NAME,
        'cursor-pointer',
        disabled && 'cursor-not-allowed opacity-50',
        active && !disabled && 'bg-overlay-hover text-text-default',
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export function CommandEmpty({ className, children, ...props }: React.ComponentProps<'div'>) {
  return (
    <div className={cn('px-2 py-6 text-center', className)} {...props}>
      {children}
    </div>
  );
}
