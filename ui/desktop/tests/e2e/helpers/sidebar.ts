/**
 * Navigating the rail.
 *
 * ⚠ **INVARIANT 2 — THE `Components` DISCLOSURE IS COLLAPSED BY DEFAULT.** The
 * rail carries only `Home` and `New chat` (plus `Settings`, which sits outside
 * the group); the other six destinations — Workflows, Scheduler, Extensions,
 * Skills, Knowledge, Built apps — live inside a disclosure that a fresh profile
 * renders **closed** (`AppSidebar.tsx`: `showComponentChildren` gates the group,
 * and the remembered preference is read from `biorouter:sidebar-components-expanded`,
 * which an isolated `--user-data-dir` has never written).
 *
 * Closed means *absent from the DOM*, not merely hidden, so
 * `[data-testid="sidebar-extensions-button"]` does not time out on a slow app —
 * it never matches at all, and neither does a `text=Extensions` fallback, whose
 * span lives inside the same unrendered button. Route through
 * {@link openSidebarEntry} rather than clicking a nav test id directly.
 */

import { expect, type Page } from '@playwright/test';

/** Labels that live behind the `Components` disclosure, verbatim from `AppSidebar.tsx`. */
export const COMPONENT_LABELS = [
  'Workflows',
  'Scheduler',
  'Extensions',
  'Skills',
  'Knowledge',
  'Built apps',
] as const;

export type SidebarLabel = (typeof COMPONENT_LABELS)[number] | 'Home' | 'New chat' | 'Settings';

/** The test id `AppSidebar` derives from a nav label (`New chat` → `sidebar-new-chat-button`). */
export function sidebarTestId(label: string): string {
  return `sidebar-${label.toLowerCase().replace(/\s+/g, '-')}-button`;
}

/** Opens the `Components` group if it is closed. Idempotent. */
export async function expandSidebarComponents(page: Page): Promise<void> {
  const disclosure = page.getByTestId('sidebar-components-disclosure');
  await expect(disclosure).toBeVisible();
  if ((await disclosure.getAttribute('aria-expanded')) === 'true') return;
  await disclosure.click();
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true');
}

/** Clicks a rail destination by its visible label, expanding the disclosure when needed. */
export async function openSidebarEntry(page: Page, label: SidebarLabel): Promise<void> {
  if ((COMPONENT_LABELS as readonly string[]).includes(label)) {
    await expandSidebarComponents(page);
  }
  const button = page.getByTestId(sidebarTestId(label));
  await expect(button).toBeVisible();
  await button.click();
}
