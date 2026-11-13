import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { DependencyEvent, DependencyInfo } from '../utils/dependencyChecker';

vi.mock('./ModalShell', () => ({
  ModalShell: ({
    children,
    footer,
    title,
    titleInfo,
    subtitle,
  }: {
    children: React.ReactNode;
    footer?: React.ReactNode;
    title?: React.ReactNode;
    titleInfo?: React.ReactNode;
    subtitle?: React.ReactNode;
  }) => (
    <div>
      <h2>{title}</h2>
      {titleInfo}
      <p data-testid="dependency-subtitle">{subtitle}</p>
      {children}
      {footer}
    </div>
  ),
}));

import DependencySetupModal, { DEPENDENCY_COPY } from './DependencySetupModal';

let emit: (e: DependencyEvent) => void = () => {};

function dep(name: string, required: boolean): DependencyInfo {
  return {
    name,
    displayName: name,
    version: null,
    installed: false,
    installCmd: `install ${name}`,
    requiresSudo: false,
    downloadUrl: '',
    required,
  };
}

beforeEach(() => {
  window.electron = {
    on: (_channel: string, handler: (e: unknown, ...args: unknown[]) => void) => {
      emit = (payload: DependencyEvent) => handler({}, payload);
      return () => {};
    },
    cliStatus: async () => null,
    installDependency: vi.fn(),
    openExternal: vi.fn(),
    createChatWindow: vi.fn(),
    dependencyEnvironment: async () => ({}),
  } as unknown as typeof window.electron;
});

// W2-PRV-11: with only the Rust toolchain (required: false) missing, the modal
// said "required ... Install them to continue", while `biorouter doctor`
// called it optional.
describe('DependencySetupModal required copy', () => {
  it('does not call optional tools required', async () => {
    render(<DependencySetupModal />);
    emit({ type: 'check-results', deps: [dep('Rust toolchain (rustc)', false)] });

    const subtitle = await screen.findByTestId('dependency-subtitle');
    expect(subtitle).not.toHaveTextContent(/required/i);
    expect(subtitle).not.toHaveTextContent(/to continue/i);
    expect(subtitle).toHaveTextContent('These optional tools add Biorouter features.');
    // "Now or later" is the title's InfoTip, and says nothing about continuing either.
    const tip = screen.getByRole('button', { name: `About ${DEPENDENCY_COPY.tipLabel}` });
    expect(tip).toHaveAccessibleDescription(DEPENDENCY_COPY.optionalTip);
    expect(tip).not.toHaveAccessibleDescription(/to continue/i);
    expect(screen.getByText('(optional)')).toBeInTheDocument();
  });

  it('still says required when a required tool is missing', async () => {
    render(<DependencySetupModal />);
    emit({
      type: 'check-results',
      deps: [dep('git', true), dep('Rust toolchain (rustc)', false)],
    });

    const subtitle = await screen.findByTestId('dependency-subtitle');
    expect(subtitle).toHaveTextContent('These tools are required for Biorouter features.');
    // "Install them to continue" is the title's InfoTip (owner, 2026-11-13).
    expect(
      screen.getByRole('button', { name: `About ${DEPENDENCY_COPY.tipLabel}` })
    ).toHaveAccessibleDescription(DEPENDENCY_COPY.requiredTip);
    expect(DEPENDENCY_COPY.requiredTip).toBe('Install them to continue.');
    expect(screen.getAllByText('(optional)')).toHaveLength(1);
  });
});
