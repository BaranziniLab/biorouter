'use client';

import * as React from 'react';

import { cn } from '../../utils';
import { Search } from '../icons/app-icons';
import { isMacPlatform, uiCopy } from './copy';
import { Input } from './input';
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';

/**
 * FilterInput: the one list filter for every view band (spec 2.6): Workflows, Built apps,
 * Extensions, Skills, History (and Scheduler if it ever needs one).
 *
 * A 28px `Input` with a leading 16px search glyph, placeholder "Filter" (History passes
 * `placeholder={uiCopy.searchHistory}`), a Tooltip "Filter · ⌘F", and ⌘F / Ctrl+F focusing it
 * while the view is mounted (`useFindShortcut`). It is `no-drag`, because it sits in a band that
 * is a window-drag region. Escape clears the text, and a second Escape leaves the field.
 *
 * It replaces the views' use of `conversation/SearchView`, which becomes the transcript's find
 * overlay and is no longer a list filter.
 */

/**
 * Focus (and select) `ref`'s field on ⌘F / Ctrl+F while the calling view is mounted and
 * `enabled`. A keydown another handler already took (`defaultPrevented`) is left alone.
 */
export function useFindShortcut(
  ref: React.RefObject<HTMLInputElement | null>,
  enabled: boolean = true
): void {
  React.useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const mac = isMacPlatform();
      const modifier = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
      if (!modifier || event.shiftKey || event.altKey) return;
      if (event.key.toLowerCase() !== 'f') return;
      const field = ref.current;
      if (!field || !field.isConnected || field.disabled) return;
      event.preventDefault();
      field.focus();
      field.select();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [ref, enabled]);
}

export interface FilterInputProps extends Omit<
  React.ComponentProps<'input'>,
  'type' | 'value' | 'onChange' | 'size'
> {
  value: string;
  onValueChange: (value: string) => void;
  /** The field's accessible name and the start of its tooltip. Defaults to "Filter". */
  label?: string;
  /** Wire ⌘F / Ctrl+F to this field while it is mounted. On by default. */
  findShortcut?: boolean;
  /** Layout only, on the wrapper. */
  className?: string;
  /** Classes for the input itself. */
  inputClassName?: string;
}

export const FilterInput = React.forwardRef<HTMLInputElement, FilterInputProps>(
  function FilterInput(
    {
      value,
      onValueChange,
      label = uiCopy.filter,
      placeholder,
      findShortcut = true,
      className,
      inputClassName,
      onKeyDown,
      ...props
    },
    forwardedRef
  ) {
    const innerRef = React.useRef<HTMLInputElement | null>(null);
    const setRefs = React.useCallback(
      (node: HTMLInputElement | null) => {
        innerRef.current = node;
        if (typeof forwardedRef === 'function') forwardedRef(node);
        else if (forwardedRef) forwardedRef.current = node;
      },
      [forwardedRef]
    );
    useFindShortcut(innerRef, findShortcut);

    const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
      onKeyDown?.(event);
      if (event.defaultPrevented || event.key !== 'Escape') return;
      event.preventDefault();
      if (value) onValueChange('');
      else event.currentTarget.blur();
    };

    return (
      <div className={cn('br-filter-input no-drag', className)} data-slot="filter-input">
        <Search aria-hidden="true" focusable="false" size={16} className="br-filter-input-icon" />
        <Tooltip>
          <TooltipTrigger asChild>
            <Input
              ref={setRefs}
              type="search"
              role="searchbox"
              aria-label={label}
              placeholder={placeholder ?? uiCopy.filter}
              value={value}
              onChange={(event) => onValueChange(event.target.value)}
              onKeyDown={handleKeyDown}
              spellCheck={false}
              autoComplete="off"
              className={cn('br-filter-input-field', inputClassName)}
              {...props}
            />
          </TooltipTrigger>
          <TooltipContent>{uiCopy.filterTooltip(label)}</TooltipContent>
        </Tooltip>
      </div>
    );
  }
);
