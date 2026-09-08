import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HostManagedModelNote } from './HostManagedModelNote';
import { HOST_MANAGED_MODEL_REASON, HOST_MANAGED_MODEL_SHORT } from './hostManagedModelCopy';

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

let browserSurface = true;

describe('HostManagedModelNote', () => {
  beforeEach(() => {
    browserSurface = true;
  });
  afterEach(() => {
    browserSurface = true;
  });

  it('renders nothing on the desktop, so a call site can mount it unconditionally', () => {
    browserSurface = false;
    const { container } = render(<HostManagedModelNote />);
    expect(container).toBeEmptyDOMElement();
  });

  it('is a boxed neutral note by default', () => {
    render(<HostManagedModelNote />);
    const note = screen.getByTestId('host-managed-model-note');
    expect(note).toHaveClass('rounded-element', 'bg-background-muted', 'border-border-subtle');
    expect(note).toHaveTextContent(HOST_MANAGED_MODEL_REASON);
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
});
