import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HostManagedModelNote } from './HostManagedModelNote';
import {
  HOST_MANAGED_DESTINATION_HELP,
  HOST_MANAGED_DESTINATION_REASON,
  HOST_MANAGED_DESTINATION_SHORT,
  HOST_MANAGED_MODEL_HELP,
  HOST_MANAGED_MODEL_REASON,
  HOST_MANAGED_MODEL_SHORT,
} from './hostManagedModelCopy';
import {
  loadProviderSetupAccess,
  resetProviderSetupAccessForTests,
} from '../../utils/providerSetupAccess';

/**
 * The regression this file exists for is not a rendering bug — it is a prop
 * contract. `className` USED to replace the component's own type classes
 * (`className ?? '…'`), so every call site had to restate them, and five of the
 * six then disagreed about the box: bare prose, two different bordered notes on
 * two different grounds, and a raw `text-[11px] leading-4` divider. A sixth
 * surface gave up and hand-copied the paragraph. A merging `className` is what
 * makes "one definition, seven surfaces" true of the SHAPE as well as of the
 * words.
 */
vi.mock('../../utils/surface', () => ({
  isBrowserSurface: () => browserSurface,
}));

const apiMocks = vi.hoisted(() => ({ getProviderSetupAccess: vi.fn() }));
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  getProviderSetupAccess: apiMocks.getProviderSetupAccess,
}));

let browserSurface = true;

describe('HostManagedModelNote', () => {
  beforeEach(() => {
    browserSurface = true;
    resetProviderSetupAccessForTests();
  });
  afterEach(() => {
    browserSurface = true;
    resetProviderSetupAccessForTests();
  });

  /**
   * SD-17: a browser the daemon admits to provider setup uses the controls as the desktop does,
   * so the note that says the host owns the choice goes, and comes into view the moment it is
   * known, without a remount.
   */
  it('renders nothing in a browser the daemon lets set up providers', async () => {
    apiMocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });
    render(<HostManagedModelNote />);
    // Fail closed: until the daemon has answered, the host owns the choice.
    expect(screen.getByTestId('host-managed-model-note')).toBeInTheDocument();

    await act(async () => {
      await loadProviderSetupAccess();
    });
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();
  });

  it('stays in a browser the daemon refuses', async () => {
    apiMocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: false } });
    render(<HostManagedModelNote />);
    await act(async () => {
      await loadProviderSetupAccess();
    });
    expect(screen.getByTestId('host-managed-model-note')).toHaveTextContent(
      HOST_MANAGED_MODEL_REASON
    );
  });

  it('renders nothing on the desktop, so a call site can mount it unconditionally', () => {
    browserSurface = false;
    const { container } = render(<HostManagedModelNote />);
    expect(container).toBeEmptyDOMElement();
  });

  it('is a boxed neutral note by default', () => {
    render(<HostManagedModelNote />);
    const note = screen.getByTestId('host-managed-model-note');
    expect(note).toHaveClass('rounded-element', 'bg-background-fill', 'border-transparent');
    // The whole reason still reaches the page: the line, then the info icon's description.
    expect(note).toHaveTextContent(HOST_MANAGED_MODEL_REASON);
  });

  // Owner, 2026-11-13: one line up front, the detail on hover. The refusal's reason is the line;
  // what to run on the host and why a browser tab cannot switch are the info icon's.
  it('shows the one-line reason and puts what to do behind the info icon', () => {
    render(<HostManagedModelNote />);
    expect(screen.getByText(HOST_MANAGED_MODEL_SHORT)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'About choosing the model on the host' })
    ).toHaveAccessibleDescription(HOST_MANAGED_MODEL_HELP);
    expect(HOST_MANAGED_MODEL_HELP).toMatch(/biorouter configure/);
    expect(HOST_MANAGED_MODEL_HELP).toMatch(/private conversation/);
  });

  /**
   * `ModelsBottomBar` mounts it INSIDE the dropdown's item list, where a rounded
   * card sitting on a card would be wrong; its own comment says so.
   */
  it('renders flush with a hairline in the `inset` variant', () => {
    render(<HostManagedModelNote variant="inset" />);
    const note = screen.getByTestId('host-managed-model-note');
    expect(note).toHaveClass('border-b', 'border-border-subtle', 'px-3', 'py-2', 'text-supporting');
    expect(note.className).not.toContain('rounded');
  });

  it('MERGES className with the default shape rather than replacing it', () => {
    render(<HostManagedModelNote className="mb-6" />);
    const note = screen.getByTestId('host-managed-model-note');
    expect(note).toHaveClass('mb-6');
    // The half that used to disappear the moment a call site passed anything.
    expect(note).toHaveClass('rounded-element', 'text-supporting');
  });

  it('carries the one-line copy when `short` is set, in either variant', () => {
    const { rerender } = render(<HostManagedModelNote short />);
    expect(screen.getByTestId('host-managed-model-note')).toHaveTextContent(
      HOST_MANAGED_MODEL_SHORT
    );
    // Short means no room for the icon either.
    expect(screen.queryByRole('button')).toBeNull();
    rerender(<HostManagedModelNote short variant="inset" />);
    expect(screen.getByTestId('host-managed-model-note')).toHaveTextContent(
      HOST_MANAGED_MODEL_SHORT
    );
  });

  /**
   * `ConfigSettings` mounts one per frozen config key and its suite queries them
   * individually, so the default id has to be overridable — that is what let the
   * hand-copied seventh implementation be deleted.
   */
  it('takes a testId override', () => {
    render(<HostManagedModelNote short testId="host-managed-config-BIOROUTER_MODEL" />);
    expect(screen.getByTestId('host-managed-config-BIOROUTER_MODEL')).toBeInTheDocument();
    expect(screen.queryByTestId('host-managed-model-note')).toBeNull();
  });
  /**
   * W2-PRV-2, round 4. A setting that decides where a provider sends its key
   * is the host's in a browser too, and its note says that, not that "the
   * model comes from the machine".
   */
  it('names where a provider sends its key when that is what the host owns', () => {
    render(<HostManagedModelNote topic="destination" />);
    const note = screen.getByTestId('host-managed-model-note');
    expect(note).toHaveTextContent(HOST_MANAGED_DESTINATION_REASON);
    expect(note.textContent).not.toContain('the model comes from');
    expect(note.textContent).toContain('biorouter configure');
    expect(
      screen.getByRole('button', { name: 'About changing this on the host' })
    ).toHaveAccessibleDescription(HOST_MANAGED_DESTINATION_HELP);
  });

  it('has a one-line destination form for a settings row', () => {
    render(<HostManagedModelNote short topic="destination" />);
    expect(screen.getByTestId('host-managed-model-note')).toHaveTextContent(
      HOST_MANAGED_DESTINATION_SHORT
    );
  });
});
