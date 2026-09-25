import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createPortal } from 'react-dom';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_WIDTH_KEYBOARD_STEP,
  SIDEBAR_WIDTH_STORAGE_KEY,
} from './sidebarWidth';

const isMobile = vi.hoisted(() => ({ value: false }));
vi.mock('../../hooks/use-mobile', () => ({
  useIsMobile: () => isMobile.value,
}));

// The REAL app layout is rendered below (T-61, T-66), with its heavy children
// stood in for exactly as `AppLayout.test.tsx` does. The stand-in rail offers
// one destination and one disclosure: the two kinds of control T-66 must tell
// apart.
vi.mock('../BioRouterSidebar/AppSidebar', () => ({
  default: function StandInRail() {
    const navigate = useNavigate();
    return (
      <div>
        <button type="button" onClick={() => navigate('/settings')}>
          Settings
        </button>
        <button type="button" aria-expanded={false}>
          Components
        </button>
      </div>
    );
  },
}));
vi.mock('../DependencySetupModal', () => ({ default: () => null }));
vi.mock('../ExtensionUpdateReporter', () => ({ default: () => null }));
vi.mock('../../hooks/useNavigation', () => ({ useNavigation: () => vi.fn() }));

import { AppLayout } from '../Layout/AppLayout';
import {
  SIDEBAR_OVERLAY_BODY_CLASS,
  Sidebar,
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from './sidebar';

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  isMobile.value = false;
});

describe('responsive Sidebar', () => {
  beforeEach(() => {
    isMobile.value = true;
  });

  it('keeps the mobile drawer at the canonical width instead of sizing to its content', () => {
    render(
      <SidebarProvider>
        <Sidebar>
          <div>A conversation title long enough to exceed the sidebar width</div>
        </Sidebar>
        <SidebarTrigger />
      </SidebarProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Toggle sidebar' }));

    const drawer = screen.getByRole('dialog');
    expect(drawer).toHaveAttribute('data-mobile', 'true');
    expect(drawer).toHaveClass(
      'w-(--sidebar-width)',
      'min-w-(--sidebar-width)',
      'max-w-(--sidebar-width)'
    );
    expect(drawer).not.toHaveClass('!w-fit', '!max-w-none');
    // ⚠ A LITERAL, exactly as this line read `'15rem'` before the sidebar became
    // resizable. Writing `` `${SIDEBAR_DEFAULT_WIDTH}px` `` here reads as the
    // same assertion and is not one: the drawer's width IS that constant, so
    // both sides move together and the expectation can never fail because the
    // shipped width changed. The number is load-bearing outside this file — the
    // OS window's `minWidth` is derived from it (`main.ts`, pinned in
    // `styles/measures.test.ts`) — so moving it must be a deliberate act that
    // trips a test, not a silent one.
    expect(drawer.style.getPropertyValue('--sidebar-width')).toBe('288px');
    expect(SIDEBAR_DEFAULT_WIDTH).toBe(288);
  });

  /**
   * The drawer is a sheet over a narrow window, not a column beside content:
   * there is no edge to drag, and a width chosen on a wide desktop layout is a
   * decision about a layout that is not on screen. So the mobile width stays
   * canonical even after the user has resized the desktop sidebar.
   */
  it('does not follow a width the user chose on the desktop layout', () => {
    localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(SIDEBAR_MAX_WIDTH));

    render(
      <SidebarProvider>
        <Sidebar>
          <div>chats</div>
        </Sidebar>
        <SidebarTrigger />
      </SidebarProvider>
    );
    fireEvent.click(screen.getByRole('button', { name: 'Toggle sidebar' }));

    expect(screen.getByRole('dialog').style.getPropertyValue('--sidebar-width')).toBe(
      `${SIDEBAR_DEFAULT_WIDTH}px`
    );
  });
});

/**
 * The resizable sidebar, tested as a STATE MACHINE.
 *
 * ⚠ jsdom computes no layout, so none of this observes a width — it observes the
 * `--sidebar-width` variable the width is published through, and the persistence
 * behind it. That distinction is the point: the bugs this area produces are
 * state bugs (a handler that never wires up, a drag that never commits, a stored
 * value that escapes the bounds), and those are decidable here. Whether the
 * column actually moves is a browser question, verified by driving the app.
 */
describe('the resizable sidebar', () => {
  const renderSidebar = () =>
    render(
      <SidebarProvider>
        <Sidebar>
          <div>chats</div>
        </Sidebar>
      </SidebarProvider>
    );

  const widthVariable = () => {
    const wrapper = document.querySelector<HTMLElement>('[data-slot="sidebar-wrapper"]');
    if (!wrapper) throw new Error('the sidebar wrapper did not render');
    return wrapper.style.getPropertyValue('--sidebar-width');
  };

  const handle = () => screen.getByRole('separator', { name: 'Resize sidebar' });

  it('opens at the default width', () => {
    renderSidebar();
    expect(widthVariable()).toBe(`${SIDEBAR_DEFAULT_WIDTH}px`);
  });

  it('opens at the width the user last chose', () => {
    localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, '324');
    renderSidebar();
    expect(widthVariable()).toBe('324px');
  });

  /**
   * The read-side clamp, exercised through the component rather than only
   * through the pure module — this is the path a real stale value takes.
   */
  it('clamps a stored width from an earlier build into the current bounds', () => {
    localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, '15');
    renderSidebar();
    expect(widthVariable()).toBe(`${SIDEBAR_MIN_WIDTH}px`);
  });

  it('exposes its bounds on the handle, so the control is reachable without a pointer', () => {
    renderSidebar();
    expect(handle()).toHaveAttribute('aria-valuemin', String(SIDEBAR_MIN_WIDTH));
    expect(handle()).toHaveAttribute('aria-valuemax', String(SIDEBAR_MAX_WIDTH));
    expect(handle()).toHaveAttribute('aria-valuenow', String(SIDEBAR_DEFAULT_WIDTH));
    expect(handle()).toHaveAttribute('tabindex', '0');
  });

  it('moves the edge with the arrow keys and persists each step', () => {
    renderSidebar();

    fireEvent.keyDown(handle(), { key: 'ArrowRight' });
    expect(widthVariable()).toBe(`${SIDEBAR_DEFAULT_WIDTH + SIDEBAR_WIDTH_KEYBOARD_STEP}px`);
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(
      String(SIDEBAR_DEFAULT_WIDTH + SIDEBAR_WIDTH_KEYBOARD_STEP)
    );

    fireEvent.keyDown(handle(), { key: 'ArrowLeft' });
    fireEvent.keyDown(handle(), { key: 'ArrowLeft' });
    expect(widthVariable()).toBe(`${SIDEBAR_DEFAULT_WIDTH - SIDEBAR_WIDTH_KEYBOARD_STEP}px`);
  });

  it('jumps to either bound and cannot be pushed past it', () => {
    renderSidebar();

    fireEvent.keyDown(handle(), { key: 'End' });
    expect(widthVariable()).toBe(`${SIDEBAR_MAX_WIDTH}px`);
    fireEvent.keyDown(handle(), { key: 'ArrowRight' });
    expect(widthVariable()).toBe(`${SIDEBAR_MAX_WIDTH}px`);

    fireEvent.keyDown(handle(), { key: 'Home' });
    expect(widthVariable()).toBe(`${SIDEBAR_MIN_WIDTH}px`);
    fireEvent.keyDown(handle(), { key: 'ArrowLeft' });
    expect(widthVariable()).toBe(`${SIDEBAR_MIN_WIDTH}px`);
  });

  it('restores the default on a double-click', () => {
    localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(SIDEBAR_MAX_WIDTH));
    renderSidebar();
    expect(widthVariable()).toBe(`${SIDEBAR_MAX_WIDTH}px`);

    fireEvent.doubleClick(handle());
    expect(widthVariable()).toBe(`${SIDEBAR_DEFAULT_WIDTH}px`);
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(SIDEBAR_DEFAULT_WIDTH));
  });

  /**
   * The drag itself. Widths land a frame late (the move handler batches through
   * rAF, so a fast pointer cannot queue one setState per event), which is why
   * the assertions here are after the commit on pointerup rather than mid-move.
   */
  it('widens as the pointer moves right and persists once, at the end', () => {
    renderSidebar();

    fireEvent.pointerDown(handle(), { pointerId: 1, clientX: SIDEBAR_DEFAULT_WIDTH });
    // Committing on pointerup uses the latest sampled width directly, so the
    // result does not depend on whether a rAF happened to run first.
    fireEvent.pointerMove(window, { pointerId: 1, clientX: SIDEBAR_DEFAULT_WIDTH + 40 });
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBeNull();

    fireEvent.pointerUp(window, { pointerId: 1, clientX: SIDEBAR_DEFAULT_WIDTH + 40 });
    expect(widthVariable()).toBe(`${SIDEBAR_DEFAULT_WIDTH + 40}px`);
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(
      String(SIDEBAR_DEFAULT_WIDTH + 40)
    );
  });

  it('narrows as the pointer moves left, and stops at the floor', () => {
    renderSidebar();

    fireEvent.pointerDown(handle(), { pointerId: 1, clientX: 500 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 0 });
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 0 });

    expect(widthVariable()).toBe(`${SIDEBAR_MIN_WIDTH}px`);
  });

  /**
   * Every exit path funnels through one `finishResize`, and this is the one that
   * gets forgotten: a pointer released outside the window fires no `pointerup`
   * on the handle. Without the window-level listeners the body would keep
   * `col-resize` painted on it and the move listener would stay live.
   */
  it('ends the drag cleanly when the window loses focus mid-drag', () => {
    renderSidebar();

    // The pointer's DISPLACEMENT is what moves the edge, not its position: the
    // grab point is wherever the user took hold of the handle, so a drag that
    // starts at 300 and ends at 330 widens the sidebar by 30 from whatever it
    // already was.
    const settled = SIDEBAR_DEFAULT_WIDTH + 30;

    fireEvent.pointerDown(handle(), { pointerId: 1, clientX: 300 });
    expect(document.body.classList.contains('biorouter-sidebar-resizing')).toBe(true);
    expect(document.body.style.cursor).toBe('col-resize');

    fireEvent.pointerMove(window, { pointerId: 1, clientX: 330 });
    fireEvent.blur(window);

    expect(document.body.classList.contains('biorouter-sidebar-resizing')).toBe(false);
    expect(document.body.style.cursor).toBe('');
    expect(document.body.style.userSelect).toBe('');
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(settled));

    // The listener is gone: further movement must not move the edge.
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 200 });
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBe(String(settled));
  });

  it('leaves no body styling behind when unmounted mid-drag', () => {
    const view = renderSidebar();

    fireEvent.pointerDown(handle(), { pointerId: 1, clientX: 300 });
    view.unmount();

    expect(document.body.classList.contains('biorouter-sidebar-resizing')).toBe(false);
    expect(document.body.style.cursor).toBe('');
    // A width the user was mid-way through choosing is not a width they chose.
    expect(localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)).toBeNull();
  });
});

/**
 * ⚠ Asserted AT THE SOURCE, and it has to be.
 *
 * The handle's whole affordance is a hover hairline and `cursor: col-resize`.
 * jsdom applies no stylesheet and never runs Tailwind, so a component test that
 * hovers the handle and reads its style sees nothing either way — and a Tailwind
 * utility that failed to generate would leave an invisible, apparently-dead edge
 * that reads as a missing feature rather than a build one. Same lesson, and the
 * same remedy, as `styles/composerFocus.test.ts`.
 */
describe('the resize handle is styled by authored CSS, not a generated utility', () => {
  const CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
  const SOURCE = readFileSync(join(__dirname, 'sidebar.tsx'), 'utf8');

  it('declares the cursor and the hover hairline in main.css', () => {
    expect(CSS).toMatch(/\.biorouter-sidebar-resize-handle\s*\{[^}]*cursor:\s*col-resize/);
    expect(CSS).toMatch(/\.biorouter-sidebar-resize-handle:hover::after/);
  });

  it('hides the handle while the sidebar is collapsed', () => {
    expect(CSS).toMatch(
      /\[data-slot='sidebar'\]\[data-state='collapsed'\]\s+\.biorouter-sidebar-resize-handle\s*\{\s*display:\s*none/
    );
  });

  /**
   * The drag must not be eased. `sidebar-gap` carries `transition-[width]` at
   * --motion-slow, so without this rule the column trails the pointer by a third
   * of a second and the sidebar feels detached from the hand moving it.
   */
  it('kills the width transition for the duration of a drag', () => {
    expect(CSS).toContain("body.biorouter-sidebar-resizing [data-slot='sidebar-gap']");
    expect(SOURCE).toContain("'biorouter-sidebar-resizing'");
  });
});

const panel = () => {
  const element = document.querySelector<HTMLElement>('[data-slot="sidebar-container"]');
  if (!element) throw new Error('the sidebar panel did not render');
  return element;
};
const sidebarState = () =>
  document.querySelector('[data-slot="sidebar"]')?.getAttribute('data-state');
const toggleWithShortcut = () =>
  act(() => {
    fireEvent.keyDown(window, { key: 'b', metaKey: true });
  });

/**
 * T-20. The collapsed off-canvas panel is only TRANSLATED off-screen, so its
 * seven rows stayed Tab stops at x = -272: focus disappeared for seven presses,
 * and Erin opened Settings by pressing Enter on a row she could not see.
 *
 * jsdom implements no inert behaviour (focus, pointer, a11y tree), so what is
 * decidable here is the attribute; that the browser honours it is the platform's
 * contract. The attribute being present exactly when the panel is away — and
 * never while it is on screen — is the whole bug.
 */
describe('the off-canvas panel leaves the tab order while it is away', () => {
  it('is inert while collapsed and live again once opened', () => {
    render(
      <SidebarProvider defaultOpen={false}>
        <Sidebar>
          <button type="button">Settings</button>
        </Sidebar>
        <SidebarTrigger />
      </SidebarProvider>
    );
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');

    toggleWithShortcut();
    expect(sidebarState()).toBe('expanded');
    expect(panel()).not.toHaveAttribute('inert');

    fireEvent.click(screen.getByRole('button', { name: 'Toggle sidebar' }));
    expect(panel()).toHaveAttribute('inert');
  });

  it('is never inert while expanded', () => {
    render(
      <SidebarProvider>
        <Sidebar>
          <button type="button">Settings</button>
        </Sidebar>
      </SidebarProvider>
    );
    expect(panel()).not.toHaveAttribute('inert');
  });

  // An `icon` sidebar collapses to a rail that is still ON screen and still
  // operable; making it inert would take working controls away.
  it('leaves a collapsed icon rail operable', () => {
    render(
      <SidebarProvider defaultOpen={false}>
        <Sidebar collapsible="icon">
          <button type="button">Settings</button>
        </Sidebar>
      </SidebarProvider>
    );
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).not.toHaveAttribute('inert');
  });

  // Collapsing with focus inside would strand focus on <body>. It goes to the
  // toggle that brings the panel back instead.
  it('hands focus to the toggle when the panel leaves while holding it', () => {
    render(
      <SidebarProvider>
        <Sidebar>
          <button type="button">Settings</button>
        </Sidebar>
        <SidebarTrigger />
      </SidebarProvider>
    );
    const row = screen.getByRole('button', { name: 'Settings' });
    act(() => row.focus());

    toggleWithShortcut();
    expect(panel()).toHaveAttribute('inert');
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Toggle sidebar' }));
  });

  it('leaves focus alone when it was elsewhere', () => {
    render(
      <SidebarProvider>
        <Sidebar>
          <button type="button">Settings</button>
        </Sidebar>
        <SidebarTrigger />
        <textarea aria-label="Composer" />
      </SidebarProvider>
    );
    const composer = screen.getByRole('textbox', { name: 'Composer' });
    act(() => composer.focus());

    toggleWithShortcut();
    expect(document.activeElement).toBe(composer);
  });
});

/**
 * Q2-51 (live QA round 2). The toggle is a disclosure and never said whether the
 * sidebar was showing, and ⌘B opened the OVERLAY with focus left underneath it:
 * erin's focus stayed on a control now hidden behind the panel, and her next
 * Tabs walked controls she could not see.
 */
describe('the sidebar toggle and ⌘B, for a keyboard user', () => {
  afterEach(() => {
    document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  const toggle = () => screen.getByRole('button', { name: 'Toggle sidebar' });

  const renderSidebar = (defaultOpen = false) =>
    render(
      <SidebarProvider defaultOpen={defaultOpen}>
        <Sidebar>
          <button type="button">New chat</button>
          <button type="button">Settings</button>
        </Sidebar>
        <SidebarTrigger />
        <button type="button">Add channel</button>
      </SidebarProvider>
    );

  it('says whether the sidebar is expanded', () => {
    renderSidebar(false);
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-expanded', 'true');

    toggleWithShortcut();
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
  });

  it('says whether the mobile drawer is open', () => {
    isMobile.value = true;
    renderSidebar(true);
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(toggle());
    expect(screen.getByRole('button', { name: 'Toggle sidebar', hidden: true })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });

  it('moves focus into an overlay ⌘B opens, and back where it was when ⌘B closes it', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    renderSidebar(false);
    const underneath = screen.getByRole('button', { name: 'Add channel' });
    act(() => underneath.focus());

    toggleWithShortcut();
    expect(sidebarState()).toBe('expanded');
    expect(screen.getByRole('button', { name: 'New chat' })).toHaveFocus();

    // Moving around inside the overlay does not change where focus goes back to.
    act(() => screen.getByRole('button', { name: 'Settings' }).focus());
    toggleWithShortcut();
    expect(sidebarState()).toBe('collapsed');
    expect(underneath).toHaveFocus();
  });

  it('falls back to the toggle when what had focus is gone by the time ⌘B closes it', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    const { container } = renderSidebar(false);
    const transient = document.createElement('button');
    transient.textContent = 'Transient';
    container.appendChild(transient);
    act(() => transient.focus());

    toggleWithShortcut();
    expect(screen.getByRole('button', { name: 'New chat' })).toHaveFocus();
    transient.remove();

    toggleWithShortcut();
    expect(toggle()).toHaveFocus();
  });

  it('leaves focus alone when ⌘B opens a docked column, which covers nothing', () => {
    renderSidebar(false);
    const composer = screen.getByRole('button', { name: 'Add channel' });
    act(() => composer.focus());

    toggleWithShortcut();
    expect(sidebarState()).toBe('expanded');
    expect(composer).toHaveFocus();
  });

  it('leaves focus on the toggle when a pointer opens the overlay', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    renderSidebar(false);
    act(() => toggle().focus());

    fireEvent.click(toggle());
    expect(sidebarState()).toBe('expanded');
    expect(toggle()).toHaveFocus();
  });

  // A shortcut that toggled nothing (here a controlled parent that refused it) must not steer a
  // later open that something else caused.
  it('forgets a ⌘B the panel never acted on', async () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    const Controlled = ({ open }: { open: boolean }) => (
      <SidebarProvider open={open} onOpenChange={() => {}}>
        <Sidebar>
          <button type="button">New chat</button>
        </Sidebar>
        <SidebarTrigger />
        <button type="button">Add channel</button>
      </SidebarProvider>
    );
    const { rerender } = render(<Controlled open={false} />);
    const outside = screen.getByRole('button', { name: 'Add channel' });
    act(() => outside.focus());

    toggleWithShortcut();
    expect(sidebarState()).toBe('collapsed');
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)));

    rerender(<Controlled open />);
    expect(sidebarState()).toBe('expanded');
    expect(outside).toHaveFocus();
  });
});

const pressEscape = (target: Element = document.activeElement ?? document.body) =>
  act(() => {
    fireEvent.keyDown(target, { key: 'Escape' });
  });

/**
 * Q3-60 (live QA round 3). The overlay is a floating surface, and every other one in the app
 * steps aside on Escape. This one ignored it: Erin opened it with the titlebar toggle (focus
 * rightly stays on the toggle), pressed Escape, and it went on covering Crew's rail until she
 * found the toggle again. Escape now closes it however it was opened, and focus lands on the
 * toggle.
 */
describe('Escape closes the overlay sidebar', () => {
  beforeEach(() => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  afterEach(() => {
    document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  const toggle = () => screen.getByRole('button', { name: 'Toggle sidebar' });

  const renderSidebar = (extra?: React.ReactNode) =>
    render(
      <SidebarProvider defaultOpen={false}>
        <Sidebar>
          <button type="button">New chat</button>
          <button type="button">Settings</button>
          {extra}
        </Sidebar>
        <SidebarTrigger />
        <input aria-label="Page field" />
      </SidebarProvider>
    );

  it('closes an overlay the toggle opened, from the toggle, and keeps focus there', () => {
    renderSidebar();
    act(() => toggle().focus());
    fireEvent.click(toggle());
    expect(sidebarState()).toBe('expanded');
    expect(toggle()).toHaveFocus();

    pressEscape();
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');
    expect(toggle()).toHaveFocus();
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
  });

  it('closes an overlay ⌘B opened, from inside it, and hands focus to the toggle', () => {
    renderSidebar();
    act(() => screen.getByRole('textbox', { name: 'Page field' }).focus());
    toggleWithShortcut();
    expect(screen.getByRole('button', { name: 'New chat' })).toHaveFocus();

    pressEscape();
    expect(sidebarState()).toBe('collapsed');
    expect(toggle()).toHaveFocus();
  });

  it('closes it with nothing focused, and puts focus on the toggle', () => {
    renderSidebar();
    fireEvent.click(toggle());
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    expect(document.activeElement).toBe(document.body);

    pressEscape(document.body);
    expect(sidebarState()).toBe('collapsed');
    expect(toggle()).toHaveFocus();
  });

  // A menu or dialog opened from a row answers its own Escape first (Radix prevents the default
  // when it dismisses); only the next Escape reaches the panel.
  it('leaves it open when something else already answered the Escape', () => {
    renderSidebar(
      <button
        type="button"
        onKeyDown={(event) => {
          if (event.key === 'Escape') event.preventDefault();
        }}
      >
        Row menu
      </button>
    );
    fireEvent.click(toggle());
    const row = screen.getByRole('button', { name: 'Row menu' });
    act(() => row.focus());

    pressEscape(row);
    expect(sidebarState()).toBe('expanded');

    // Something that stops the event before it reaches the window, the same.
    const stop = (event: KeyboardEvent) => event.stopPropagation();
    document.addEventListener('keydown', stop);
    try {
      pressEscape(screen.getByRole('button', { name: 'Settings' }));
      expect(sidebarState()).toBe('expanded');
    } finally {
      document.removeEventListener('keydown', stop);
    }

    pressEscape(screen.getByRole('button', { name: 'Settings' }));
    expect(sidebarState()).toBe('collapsed');
  });

  // The page behind the overlay keeps its own Escape (clear a field, cancel an edit).
  it('leaves it open for an Escape pressed on the page behind it', () => {
    renderSidebar();
    fireEvent.click(toggle());
    const field = screen.getByRole('textbox', { name: 'Page field' });
    act(() => field.focus());

    pressEscape(field);
    expect(sidebarState()).toBe('expanded');
    expect(field).toHaveFocus();
  });

  it('leaves a docked column alone, which covers nothing', () => {
    document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS);
    renderSidebar();
    fireEvent.click(toggle());
    act(() => toggle().focus());

    pressEscape();
    expect(sidebarState()).toBe('expanded');
  });

  it('stops listening once the overlay is closed', () => {
    renderSidebar();
    fireEvent.click(toggle());
    pressEscape(toggle());
    expect(sidebarState()).toBe('collapsed');

    // A later Escape on the toggle must not reopen, re-close or steal anything.
    const field = screen.getByRole('textbox', { name: 'Page field' });
    act(() => field.focus());
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    act(() => {
      field.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(false);
    expect(field).toHaveFocus();
  });
});

/**
 * T-66. Below rung 1 an open sidebar floats OVER the page (AppLayout puts
 * SIDEBAR_OVERLAY_BODY_CLASS on <body> and main.css draws the overlay from it),
 * so an overlay left open after a choice kept covering the page the user had
 * just asked for.
 */
describe('an overlay sidebar steps aside once something is chosen', () => {
  afterEach(() => {
    document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS);
  });

  const renderOpenSidebar = (extra?: React.ReactNode) =>
    render(
      <SidebarProvider>
        <Sidebar>
          <button type="button">Settings</button>
          <button type="button" aria-expanded={false}>
            Components
          </button>
          <button type="button" aria-haspopup="menu">
            More
          </button>
          {extra}
        </Sidebar>
      </SidebarProvider>
    );

  it('closes after a destination is chosen in the overlay', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    renderOpenSidebar();

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');
  });

  // A disclosure or a menu opener changes what the rail SHOWS. The user is
  // still choosing, so closing then would take the menu they just opened away.
  it('stays open for a disclosure or a menu opener', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    renderOpenSidebar();

    fireEvent.click(screen.getByRole('button', { name: 'Components' }));
    fireEvent.click(screen.getByRole('button', { name: 'More' }));
    expect(sidebarState()).toBe('expanded');
  });

  // A row's context menu or confirmation is portalled to <body>, but React
  // still bubbles its clicks through the row. Acting there is not choosing a
  // destination in the rail.
  it('ignores clicks that only bubble in from a portal', () => {
    document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS);
    renderOpenSidebar(createPortal(<button type="button">Delete chat</button>, document.body));

    fireEvent.click(screen.getByRole('button', { name: 'Delete chat' }));
    expect(sidebarState()).toBe('expanded');
  });

  // A docked column sits beside the content, not over it: it stays as the user
  // left it, which is what it has always done.
  it('leaves a docked sidebar open', () => {
    renderOpenSidebar();

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(sidebarState()).toBe('expanded');
  });
});

/**
 * The same three fixes, measured on the REAL app shell rather than on the
 * primitive alone: the body class comes from AppLayout's own width watcher, the
 * toggle from its own titlebar, and the landmarks from its own route container.
 *
 * jsdom's window is 1024 px wide, which is below rung 1 (1120 px), so the shell
 * starts exactly where Erin did: sidebar auto-collapsed, overlay on demand.
 */
describe('the app shell, as a keyboard and screen-reader user meets it', () => {
  let innerWidth: PropertyDescriptor | undefined;

  const setWindowWidth = (width: number) =>
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });

  beforeEach(() => {
    innerWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth');
  });

  afterEach(() => {
    if (innerWidth) Object.defineProperty(window, 'innerWidth', innerWidth);
  });

  const renderShell = () =>
    render(
      <MemoryRouter initialEntries={['/']}>
        <Routes>
          <Route element={<AppLayout />}>
            <Route path="/" element={<p>Home page</p>} />
            <Route path="/settings" element={<p>Settings page</p>} />
          </Route>
        </Routes>
      </MemoryRouter>
    );

  // T-61: SidebarInset rendered a <main> around the route container's own
  // <main>, so the landmark list offered "main" twice for one page. The inset
  // is a layout box and contributes no landmark; the route supplies the one.
  it('lets the inset add no landmark of its own', () => {
    render(
      <SidebarProvider>
        <SidebarInset>
          <p>page</p>
        </SidebarInset>
      </SidebarProvider>
    );
    expect(screen.queryByRole('main')).toBeNull();
  });

  it('has exactly one main landmark, and it holds the route', () => {
    setWindowWidth(1400);
    renderShell();

    const mains = screen.getAllByRole('main');
    expect(mains).toHaveLength(1);
    expect(mains[0]).toHaveTextContent('Home page');
    expect(document.querySelector('[data-slot="sidebar-inset"]')?.tagName).toBe('DIV');
  });

  it('keeps the auto-collapsed rail out of the tab order, and closes the overlay after a choice', () => {
    setWindowWidth(1024);
    renderShell();

    expect(document.body).toHaveClass(SIDEBAR_OVERLAY_BODY_CLASS);
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');

    // ⌘B opens it as an overlay, and an overlay is operable — and holds focus (Q2-51).
    toggleWithShortcut();
    expect(sidebarState()).toBe('expanded');
    expect(panel()).not.toHaveAttribute('inert');
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveFocus();
    expect(screen.getByTestId('titlebar-sidebar-toggle')).toHaveAttribute('aria-expanded', 'true');

    // Opening the disclosure is not a choice: the overlay stays.
    fireEvent.click(screen.getByRole('button', { name: 'Components' }));
    expect(sidebarState()).toBe('expanded');

    // Choosing a destination by keyboard: Settings opens AND the overlay gets
    // out of its way, handing focus to the titlebar toggle rather than <body>.
    const settings = screen.getByRole('button', { name: 'Settings' });
    act(() => settings.focus());
    act(() => {
      fireEvent.click(settings);
    });
    expect(screen.getByText('Settings page')).toBeInTheDocument();
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');
    expect(document.activeElement).toBe(screen.getByTestId('titlebar-sidebar-toggle'));
  });

  // Q3-60 in the real shell: the titlebar toggle opens the overlay, Escape closes it.
  it('closes the overlay the titlebar toggle opened on Escape', () => {
    setWindowWidth(1024);
    renderShell();
    const titlebarToggle = screen.getByTestId('titlebar-sidebar-toggle');
    act(() => titlebarToggle.focus());
    fireEvent.click(titlebarToggle);
    expect(sidebarState()).toBe('expanded');

    pressEscape(titlebarToggle);
    expect(sidebarState()).toBe('collapsed');
    expect(panel()).toHaveAttribute('inert');
    expect(titlebarToggle).toHaveFocus();
  });

  it('leaves the docked sidebar open after a choice on a wide window', () => {
    setWindowWidth(1400);
    renderShell();

    expect(document.body).not.toHaveClass(SIDEBAR_OVERLAY_BODY_CLASS);
    expect(sidebarState()).toBe('expanded');
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    });
    expect(screen.getByText('Settings page')).toBeInTheDocument();
    expect(sidebarState()).toBe('expanded');
  });
});

/**
 * ⚠ The overlay is ONE class in three files, and nothing but this ties them.
 * `AppLayout` sets it, `main.css` draws the overlay from it, and the sidebar
 * asks it whether a choice should close the panel. A rename in any one of them
 * would leave the overlay drawn but never dismissed — or dismissed while docked.
 */
describe('the overlay class is shared, not re-spelled', () => {
  const CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
  const LAYOUT = readFileSync(join(__dirname, '../Layout/AppLayout.tsx'), 'utf8');

  it('is the class AppLayout toggles on <body> and main.css draws the overlay from', () => {
    expect(LAYOUT).toContain(`classList.toggle('${SIDEBAR_OVERLAY_BODY_CLASS}'`);
    expect(CSS).toContain(`body.${SIDEBAR_OVERLAY_BODY_CLASS} [data-slot='sidebar-container']`);
    expect(CSS).toContain(`body.${SIDEBAR_OVERLAY_BODY_CLASS} [data-slot='sidebar-inset']`);
  });

  /**
   * Q3-60. The overlay's rule swaps the docked hairline for the popover shadow, and a black
   * shadow on a near-black page is no edge. A 1px `--border-subtle` border on the side that meets
   * the page — a border, because forced colours keep it and strip the shadow.
   */
  it('draws the overlay’s edge as a --border-subtle border on the side facing the page', () => {
    const rule = (selector: string) => {
      const at = CSS.indexOf(`${selector} {`);
      expect(at, selector).toBeGreaterThanOrEqual(0);
      return CSS.slice(at, CSS.indexOf('}', at)).replace(/\s+/g, ' ');
    };
    const body = `body.${SIDEBAR_OVERLAY_BODY_CLASS}`;
    expect(rule(`${body} [data-side='left'] > [data-slot='sidebar-container']`)).toContain(
      'border-right: 1px solid var(--border-subtle)'
    );
    expect(rule(`${body} [data-side='right'] > [data-slot='sidebar-container']`)).toContain(
      'border-left: 1px solid var(--border-subtle)'
    );
  });

  it('keys that edge on the attribute and nesting the panel really renders', () => {
    render(
      <SidebarProvider>
        <Sidebar>
          <p>rail</p>
        </Sidebar>
      </SidebarProvider>
    );
    expect(panel().parentElement).toHaveAttribute('data-side', 'left');
  });
});
