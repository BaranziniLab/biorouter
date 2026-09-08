import { useEffect, useState, forwardRef } from 'react';
import { SlidersHorizontal } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import PermissionRulesModal from '../permission/PermissionRulesModal';

export interface BioRouterMode {
  key: string;
  label: string;
  description: string;
}

export const all_biorouter_modes: BioRouterMode[] = [
  {
    key: 'auto',
    label: 'Autonomous',
    description: 'Use tools and edit, create, or delete files without asking first.',
  },
  {
    key: 'approve',
    label: 'Manual',
    description: 'Ask before using tools, extensions, or making file changes.',
  },
  {
    key: 'smart_approve',
    label: 'Smart',
    description: 'Ask only when an action’s risk level requires your approval.',
  },
  {
    key: 'chat',
    label: 'Chat only',
    description: 'Chat with the selected model without tools or extensions.',
  },
];

interface ModeSelectionItemProps {
  currentMode: string;
  mode: BioRouterMode;
  showDescription: boolean;
  handleModeChange: (newMode: string) => void;
}

export const ModeSelectionItem = forwardRef<HTMLDivElement, ModeSelectionItemProps>(
  ({ currentMode, mode, showDescription, handleModeChange }, ref) => {
    const [checked, setChecked] = useState(currentMode == mode.key);
    const [isPermissionModalOpen, setIsPermissionModalOpen] = useState(false);

    useEffect(() => {
      setChecked(currentMode === mode.key);
    }, [currentMode, mode.key]);

    // A fragment: the row belongs directly to the list, so the hairlines land
    // between rows rather than around a per-item wrapper. The modal renders no
    // inline DOM while it is closed, so it cannot displace `:last-child`.
    //
    // ⚠ **No state-dependent fill.** This row painted `bg-background-medium/70`
    // while selected; the radio states the selection, and the unlayered
    // `.biorouter-settings-row:hover` beat that utility, so pointing at the
    // selected row visibly un-selected it.
    return (
      <>
        <div
          ref={ref}
          className="biorouter-settings-row group flex min-w-0 cursor-pointer items-center justify-between gap-3 px-3 py-2.5 text-text-default"
          onClick={() => handleModeChange(mode.key)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              handleModeChange(mode.key);
            }
          }}
          role="radio"
          aria-checked={checked}
          tabIndex={0}
        >
          <div className="min-w-0 flex-1">
            <p className="text-label text-text-default break-words">{mode.label}</p>
            {showDescription && (
              <p className="mt-0.5 max-w-md text-supporting text-text-muted break-words [overflow-wrap:anywhere]">
                {mode.description}
              </p>
            )}
          </div>

          <div className="relative flex flex-shrink-0 items-center gap-2">
            {(mode.key == 'approve' || mode.key == 'smart_approve') && (
              <Button
                type="button"
                variant="ghost"
                className="text-text-muted hover:text-text-default"
                onClick={(e) => {
                  e.stopPropagation();
                  setIsPermissionModalOpen(true);
                }}
                aria-label={`Configure ${mode.label} tool permissions`}
              >
                <SlidersHorizontal className="h-4 w-4" />
                Permissions
              </Button>
            )}
            {/* `CustomRadio`'s construction, verbatim (§3.3): a 22px visual ring
                inside a 24px hit target with a 10px inner dot, the input and
                both styled siblings sharing one box so `peer-checked` reaches
                them. The row keeps its own `role="radio"` / `aria-checked` /
                `tabIndex` / keydown semantics, which `CustomRadio`'s `<label>`
                would duplicate — which is why the construction is inlined here
                rather than the component being mounted.

                ⚠ The input must stay INSIDE this span. `peer-checked:` compiles
                to a general SIBLING combinator, so a styled element that is a
                descendant of the peer's sibling is never matched, and the ring
                would simply never fill. */}
            <span className="relative inline-flex h-6 w-6 shrink-0 items-center justify-center">
              <input
                type="radio"
                name="modes"
                value={mode.key}
                checked={checked}
                onChange={() => handleModeChange(mode.key)}
                aria-hidden="true"
                tabIndex={-1}
                className="peer sr-only"
              />
              <span
                className="pointer-events-none absolute inset-[1px] rounded-full border-[1.5px] border-border-emphasized
                           transition-colors
                           peer-checked:border-border-accent"
              />
              <span
                className="pointer-events-none h-2.5 w-2.5 rounded-full bg-background-accent opacity-0
                           transition-opacity
                           peer-checked:opacity-100"
              />
            </span>
          </div>
        </div>

        <PermissionRulesModal
          isOpen={isPermissionModalOpen}
          onClose={() => setIsPermissionModalOpen(false)}
        />
      </>
    );
  }
);

ModeSelectionItem.displayName = 'ModeSelectionItem';
