import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

/**
 * tailwind-merge has to be TAUGHT the design system's own scales.
 *
 * It ships knowing Tailwind's stock scales only. Anything else in the `text-*`
 * namespace it classifies as a text COLOUR — so `cn('text-text-inverse',
 * 'text-supporting')` decided the two were the same class group and dropped the
 * colour, silently rendering a tooltip's ink in the default colour on an inverse
 * fill. Every semantic type role (`text-label`, `text-body`, …) hits this the
 * moment a call site pairs it with a `text-text-*` colour, which is the normal
 * case. Registering them under the `text` theme key (Tailwind v4's font-size
 * namespace) restores the correct two groups: one size, one colour.
 *
 * The radius ladder has the mirror problem in reverse: `rounded-element` and
 * `rounded-container` were both unknown, so both SURVIVED a merge and a call
 * site's override no longer beat a primitive's default — source order decided.
 * The geometry and elevation scales below are the same shape of hazard:
 * tailwind-merge's stock `spacing` scale is `['px', <number>]` and its stock
 * `shadow`/`inset-shadow` scales are t-shirt sizes only, so every semantic name
 * this design system adds falls out of its class group unless it is listed here.
 *
 * The colour tokens are the one family that needs NO entry: tailwind-merge's
 * default `color` scale is `isAny`, so `bg-wash-danger` and `text-text-muted`
 * are classified correctly without being enumerated. That is also why the type
 * ROLES had to be registered — with `color` matching anything, `text-supporting`
 * looked like ink until `text` said otherwise.
 *
 * Keep these lists in step with the `@theme` / `@theme inline` blocks in
 * `src/styles/main.css`.
 */
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      // `heading` and `display` are deprecated aliases of `subheading` and
      // `title` (redesign 2026-10). They stay registered until their last call
      // sites move, then leave this list in the same change as `main.css`.
      text: [
        'display',
        'title',
        'heading',
        'subheading',
        'body',
        'prose',
        'label',
        'secondary',
        'supporting',
        'caps',
        'chip',
        'code',
      ],
      radius: ['inner', 'element', 'container', 'surface'],
      // Geometry: the control ladder, icon boxes, chrome bands, row rhythms,
      // overlay widths and reading measures. Feeds `h-`/`w-`/`size-`/`min-h-`/
      // `max-w-`/`p-`/`gap-` alike — Tailwind v4 runs the whole size family off
      // one `--spacing-*` namespace, so one omission here breaks every one of
      // those utilities for that name at once.
      spacing: [
        'control-sm',
        'control-md',
        'control-lg',
        'control-compact',
        'icon-chip',
        'icon-row',
        'icon-banner',
        'chrome',
        'dock',
        'tab',
        'row',
        'row-rail',
        'row-nav',
        'dialog-sm',
        'dialog-md',
        'dialog-lg',
        'toast',
        'measure-chat',
        'measure-page',
        'measure-graph',
        'knowledge-rail-sources',
        'knowledge-rail-detail',
        'knowledge-filter-height',
      ],
      // All five elevations, not just the new one. The four that shipped before
      // `raised` were never registered, so they fell through to the `shadow-color`
      // group (which matches anything) and merged only by accident. Listing one
      // without the rest would be strictly worse: `shadow-raised` would move into
      // the real `shadow` group while `shadow-popover` stayed a colour, and the
      // two would stop conflicting with each other entirely.
      shadow: ['raised', 'default', 'composer', 'popover', 'modal'],
      'inset-shadow': ['hairline', 'accent'],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function snakeToTitleCase(snake: string): string {
  return snake
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
    .join(' ');
}

export function toolIdentifierToTitleCase(identifier: string): string {
  return snakeToTitleCase(
    identifier.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/[-.]/g, '_')
  ).replace(/\b(Api|Baam|Id|Mcp|Okf|Omop|Sql|Ucsf|Url)\b/g, (word) => word.toUpperCase());
}
