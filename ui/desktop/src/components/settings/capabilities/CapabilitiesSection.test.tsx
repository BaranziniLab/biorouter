import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { copilotSetupCopy } from '../../copilot/copy';
import { CapabilitiesSection } from './CapabilitiesSection';
import { CAPABILITIES } from './capabilities';

const mocks = vi.hoisted(() => ({
  extensionsList: [] as Array<Record<string, unknown>>,
  addExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  toggleExtensionDefault: vi.fn(async () => undefined),
  copilotSetup: vi.fn(),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: mocks.extensionsList,
    addExtension: mocks.addExtension,
    getExtensions: mocks.getExtensions,
  }),
}));

vi.mock('../extensions', () => ({
  toggleExtensionDefault: mocks.toggleExtensionDefault,
}));

vi.mock('../../copilot/copilotApi', () => ({ copilotSetup: mocks.copilotSetup }));

const labelOf = (key: string) => CAPABILITIES.find((capability) => capability.key === key)!.label;
const helpOf = (key: string) =>
  CAPABILITIES.find((capability) => capability.key === key)!.description;

describe('CapabilitiesSection', () => {
  beforeEach(() => {
    mocks.extensionsList = [];
    vi.clearAllMocks();
    Object.assign(window, { electron: { platform: 'darwin', getConfig: () => ({}) } });
  });

  it('shows all capabilities and uses their declared defaults while config loads', () => {
    render(<CapabilitiesSection />);

    expect(screen.getAllByRole('switch')).toHaveLength(14);
    expect(screen.getByRole('switch', { name: 'Auto Visualiser' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Code Execution' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Biorouter Copilot' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Agent Drafter' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Chat Recall' })).not.toBeChecked();
    expect(screen.getByRole('switch', { name: 'Crew' })).toBeChecked();
  });

  // Spec 2.6: a switch is named by its row's visible label, never by "Toggle …" and never by its
  // state. The explanation is not a line under the label: it is the row's InfoTip, which the
  // switch hears through aria-describedby.
  it('names every switch by its row label and describes it with the row help', () => {
    render(<CapabilitiesSection />);

    for (const capability of CAPABILITIES) {
      const control = screen.getByRole('switch', { name: capability.label });
      expect(control).toHaveAccessibleDescription(capability.description);
    }
    expect(screen.queryByRole('switch', { name: /toggle/i })).toBeNull();
  });

  it('prints one line per row: the description is help, not a paragraph', () => {
    const { container } = render(<CapabilitiesSection />);

    expect(container.querySelectorAll('.biorouter-settings-row')).toHaveLength(14);
    expect(container.querySelectorAll('p')).toHaveLength(0);
    // Each row carries its explanation as an InfoTip, named after the row.
    expect(screen.getByRole('button', { name: 'About Developer' })).toBeInTheDocument();
  });

  it('toggles the persisted capability state through the shared extension flow', async () => {
    mocks.extensionsList = [
      {
        type: 'platform',
        name: 'chatrecall',
        description: 'Recall chats',
        enabled: false,
      },
    ];
    render(<CapabilitiesSection />);

    fireEvent.click(screen.getByRole('switch', { name: 'Chat Recall' }));

    await waitFor(() =>
      expect(mocks.toggleExtensionDefault).toHaveBeenCalledWith(
        expect.objectContaining({
          toggle: 'toggleOn',
          extensionConfig: expect.objectContaining({ name: 'chatrecall', enabled: false }),
          addToConfig: mocks.addExtension,
          itemKind: 'capability',
        })
      )
    );
    expect(mocks.getExtensions).toHaveBeenCalledWith(true);
  });

  // Crew left the Extensions page for this list. Its switch reads and writes the same `crew`
  // entry the Extensions page did, so a person who had switched it off finds it off here, and
  // turning it back on is the same config write.
  it('keeps a saved Crew off and turns it on through the same config entry', async () => {
    mocks.extensionsList = [
      {
        type: 'platform',
        name: 'crew',
        description: 'Use saved Crew connections',
        bundled: true,
        enabled: false,
      },
    ];
    render(<CapabilitiesSection />);

    const crew = screen.getByRole('switch', { name: 'Crew' });
    expect(crew).not.toBeChecked();
    expect(crew).toBeEnabled();
    expect(crew).toHaveAccessibleDescription(helpOf('crew'));

    fireEvent.click(crew);
    await waitFor(() =>
      expect(mocks.toggleExtensionDefault).toHaveBeenCalledWith(
        expect.objectContaining({
          toggle: 'toggleOn',
          extensionConfig: expect.objectContaining({
            type: 'platform',
            name: 'crew',
            enabled: false,
          }),
          addToConfig: mocks.addExtension,
          itemKind: 'capability',
        })
      )
    );
  });

  it('disables a switch whose config entry has not loaded yet', () => {
    render(<CapabilitiesSection />);
    // No entry yet (config still loading): shown at its default, and not togglable.
    expect(screen.getByRole('switch', { name: 'Developer' })).toBeDisabled();
  });
});

// CA2: Biorouter Copilot's setup check is the row's trailing button, and its result is a block
// AFTER the row, so a result of several lines can no longer swell the row or take its hover wash.
describe('Biorouter Copilot row', () => {
  beforeEach(() => {
    mocks.extensionsList = [];
    vi.clearAllMocks();
    Object.assign(window, { electron: { platform: 'darwin', getConfig: () => ({}) } });
  });

  const copilotRow = () =>
    screen
      .getByRole('switch', { name: labelOf('computercontroller') })
      .closest('.biorouter-settings-row')!;

  it('puts "Check setup" in the row, before the switch', () => {
    render(<CapabilitiesSection />);

    const row = copilotRow() as HTMLElement;
    const check = within(row).getByRole('button', { name: copilotSetupCopy.check });
    const control = within(row).getByRole('switch');
    expect(check.compareDocumentPosition(control) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The bare verb is still tied to its row for assistive technology.
    expect(check).toHaveAccessibleDescription(labelOf('computercontroller'));
    // No other capability row has a button besides its InfoTip.
    for (const other of CAPABILITIES.filter((c) => c.key !== 'computercontroller')) {
      const otherRow = screen
        .getByRole('switch', { name: other.label })
        .closest('.biorouter-settings-row') as HTMLElement;
      expect(within(otherRow).queryByRole('button', { name: copilotSetupCopy.check })).toBeNull();
    }
  });

  it('shows the result as a block after the row, never inside it', async () => {
    mocks.copilotSetup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
      target: 'darwin-arm64',
    });
    render(<CapabilitiesSection />);
    const row = copilotRow() as HTMLElement;

    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.check }));
    const runtime = await screen.findByText('Runtime ready');

    expect(row.contains(runtime)).toBe(false);
    // The panel is the row's next sibling in the list, so the row keeps its 40px.
    const panel = runtime.closest('#copilot-setup-details') as HTMLElement;
    expect(panel).not.toBeNull();
    expect(row.nextElementSibling).toBe(panel);
    // The row's own button now hides what is shown, and says what it controls.
    const hide = within(row).getByRole('button', { name: copilotSetupCopy.hide });
    expect(hide).toHaveAttribute('aria-expanded', 'true');
    expect(hide).toHaveAttribute('aria-controls', 'copilot-setup-details');
  });

  it('hides and re-shows the result without asking the backend again', async () => {
    mocks.copilotSetup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CapabilitiesSection />);
    const row = copilotRow() as HTMLElement;

    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.check }));
    await screen.findByText('Runtime ready');
    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.hide }));
    expect(screen.queryByText('Runtime ready')).toBeNull();
    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.show }));
    expect(await screen.findByText('Runtime ready')).toBeVisible();
    expect(mocks.copilotSetup).toHaveBeenCalledTimes(1);
  });

  it('reports a failed check as an alert after the row', async () => {
    mocks.copilotSetup.mockRejectedValue(new Error('backend down'));
    render(<CapabilitiesSection />);
    const row = copilotRow() as HTMLElement;

    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.check }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('backend down');
    expect(row.contains(alert)).toBe(false);
  });

  it('keeps the switch working while the result is open', async () => {
    mocks.extensionsList = [
      { type: 'platform', name: 'computercontroller', description: 'Copilot', enabled: true },
    ];
    mocks.copilotSetup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CapabilitiesSection />);
    const row = copilotRow() as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: copilotSetupCopy.check }));
    await screen.findByText('Runtime ready');

    fireEvent.click(within(row).getByRole('switch'));
    await waitFor(() =>
      expect(mocks.toggleExtensionDefault).toHaveBeenCalledWith(
        expect.objectContaining({ toggle: 'toggleOff' })
      )
    );
  });
});
