import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ModeSection } from './ModeSection';
import { approvalsCopy } from '../chat/copy';

const config = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
  getExtensions: vi.fn(async () => []),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    read: config.read,
    upsert: config.upsert,
    getExtensions: config.getExtensions,
  }),
}));

function storedMode(mode: string) {
  config.read.mockImplementation(async (key: string) =>
    key === 'BIOROUTER_MODE' ? mode : undefined
  );
}

describe('Settings > Chat > Approvals', () => {
  beforeEach(() => {
    config.read.mockReset();
    config.upsert.mockReset();
    config.upsert.mockResolvedValue(undefined);
  });

  /**
   * One select replaces four radio rows with a paragraph each. The trigger is named by the row
   * label AND the current value, so the visible word is in the name.
   */
  it('shows the stored mode on one select named by its row', async () => {
    storedMode('smart_approve');
    render(<ModeSection />);
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Smart` })
    ).toBeInTheDocument();
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.queryByRole('radiogroup')).toBeNull();
  });

  /**
   * The daemon answers a moment after the page draws. Until it does, the select must not state
   * a mode: "Autonomous" shown first and corrected later told a person who chose Manual that
   * nothing would ask them. Max turns is held back the same way, or a number typed into it would
   * be replaced by the answer.
   */
  it('shows no mode and no Max turns value until the daemon has answered', async () => {
    let answer: (key: string) => unknown = () => undefined;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    config.read.mockImplementation(async (key: string) => {
      await gate;
      return answer(key);
    });
    answer = (key) =>
      key === 'BIOROUTER_MODE' ? 'approve' : key === 'BIOROUTER_MAX_TURNS' ? 25 : undefined;

    render(<ModeSection />);
    const select = screen.getByRole('button', { name: approvalsCopy.mode });
    expect(select).toBeDisabled();
    expect(select).toHaveAttribute('aria-busy', 'true');
    expect(select).not.toHaveTextContent(/Autonomous|Manual|Smart|Chat only/);
    expect(screen.getByRole('button', { name: approvalsCopy.editToolPermissions })).toBeDisabled();
    const turns = screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns });
    expect(turns).toBeDisabled();
    expect(turns).toHaveValue(null);

    release();
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` })
    ).toBeEnabled();
    await waitFor(() =>
      expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toHaveValue(25)
    );
    expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toBeEnabled();
  });

  it('treats a mode that was never saved as Autonomous, the daemon’s default', async () => {
    config.read.mockImplementation(async () => undefined);
    render(<ModeSection />);
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Autonomous` })
    ).toBeEnabled();
  });

  it('keeps each mode’s explanation inside its menu item, and saves the choice', async () => {
    storedMode('auto');
    const user = userEvent.setup();
    render(<ModeSection />);
    const trigger = await screen.findByRole('button', { name: `${approvalsCopy.mode} Autonomous` });

    // Not on the page until the menu opens.
    expect(screen.queryByText(approvalsCopy.modes[1].description)).toBeNull();
    await user.click(trigger);
    const manual = await screen.findByRole('menuitemradio', { name: /Manual/ });
    expect(manual).toHaveTextContent(approvalsCopy.modes[1].description);

    await user.click(manual);
    await waitFor(() =>
      expect(config.upsert).toHaveBeenCalledWith('BIOROUTER_MODE', 'approve', false)
    );
    expect(
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` })
    ).toBeInTheDocument();
  });

  /** Tool permissions shape only Manual and Smart, so the one Edit button is live only there. */
  it.each([
    ['auto', false],
    ['approve', true],
    ['smart_approve', true],
    ['chat', false],
  ])('in %s mode, Edit tool permissions is enabled: %s', async (mode, enabled) => {
    storedMode(mode);
    render(<ModeSection />);
    await screen.findByRole('button', {
      name: `${approvalsCopy.mode} ${approvalsCopy.modes.find((m) => m.key === mode)!.label}`,
    });
    const edit = screen.getByRole('button', { name: approvalsCopy.editToolPermissions });
    if (enabled) expect(edit).toBeEnabled();
    else expect(edit).toBeDisabled();
  });

  it('opens the tool permission rules from the row', async () => {
    storedMode('approve');
    const user = userEvent.setup();
    render(<ModeSection />);
    await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` });
    await user.click(screen.getByRole('button', { name: approvalsCopy.editToolPermissions }));
    expect(await screen.findByRole('dialog')).toHaveTextContent('Tool permissions');
  });

  it('shows Max turns without a disclosure', async () => {
    storedMode('auto');
    render(<ModeSection />);
    expect(await screen.findByRole('spinbutton', { name: approvalsCopy.maxTurns })).toBeVisible();
    expect(screen.queryByRole('button', { name: /chat limits/i })).toBeNull();
  });

  /**
   * Settings mounts only the tab on screen, so every visit to Chat is a new mount. A select with
   * no words and a field with no number for the frames the daemon takes, on every visit, is what
   * vision round 8 found (V4-R7-2). The answer cannot have changed since the last visit, so a later
   * one starts from it, and still asks.
   */
  describe('a later visit', () => {
    const answers = (key: string) =>
      key === 'BIOROUTER_MODE' ? 'approve' : key === 'BIOROUTER_MAX_TURNS' ? 25 : undefined;

    /** Visit once, wait for the answers, and leave as a tab switch does. */
    async function firstVisit() {
      config.read.mockImplementation(async (key: string) => answers(key));
      const visit = render(<ModeSection />);
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` });
      await waitFor(() =>
        expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toHaveValue(25)
      );
      visit.unmount();
    }

    /** The daemon is slow from here on, so only a remembered answer can paint the controls. */
    function daemonHangs() {
      config.read.mockImplementation(() => new Promise(() => {}));
    }

    it('paints the stored mode and Max turns in the first render, enabled', async () => {
      await firstVisit();
      daemonHangs();
      render(<ModeSection />);

      const select = screen.getByRole('button', { name: `${approvalsCopy.mode} Manual` });
      expect(select).toBeEnabled();
      expect(select).not.toHaveAttribute('aria-busy');
      // Manual is a mode the tool permissions shape, so that row is live too.
      expect(screen.getByRole('button', { name: approvalsCopy.editToolPermissions })).toBeEnabled();
      const turns = screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns });
      expect(turns).toBeEnabled();
      expect(turns).not.toHaveAttribute('aria-busy');
      expect(turns).toHaveValue(25);
    });

    it('still asks the daemon, and shows what it says when it differs', async () => {
      await firstVisit();
      config.read.mockImplementation(async (key: string) =>
        key === 'BIOROUTER_MODE' ? 'smart_approve' : key === 'BIOROUTER_MAX_TURNS' ? 40 : undefined
      );
      render(<ModeSection />);
      expect(screen.getByRole('button', { name: `${approvalsCopy.mode} Manual` })).toBeEnabled();
      expect(
        await screen.findByRole('button', { name: `${approvalsCopy.mode} Smart` })
      ).toBeEnabled();
      await waitFor(() =>
        expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toHaveValue(40)
      );
    });

    it('starts from a mode the person chose, not the answer before it', async () => {
      await firstVisit();
      const user = userEvent.setup();
      const visit = render(<ModeSection />);
      await user.click(await screen.findByRole('button', { name: `${approvalsCopy.mode} Manual` }));
      await user.click(await screen.findByRole('menuitemradio', { name: /Autonomous/ }));
      await waitFor(() =>
        expect(config.upsert).toHaveBeenCalledWith('BIOROUTER_MODE', 'auto', false)
      );
      visit.unmount();

      daemonHangs();
      render(<ModeSection />);
      expect(
        screen.getByRole('button', { name: `${approvalsCopy.mode} Autonomous` })
      ).toBeEnabled();
    });

    it('remembers that nothing was saved for Max turns, and shows the default at once', async () => {
      config.read.mockImplementation(async () => undefined);
      const visit = render(<ModeSection />);
      await screen.findByRole('button', { name: `${approvalsCopy.mode} Autonomous` });
      await waitFor(() =>
        expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toBeEnabled()
      );
      visit.unmount();

      daemonHangs();
      render(<ModeSection />);
      const turns = screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns });
      expect(turns).toBeEnabled();
      expect(turns).toHaveValue(0);
    });

    it('keeps the last answer when a later read fails, rather than the default', async () => {
      await firstVisit();
      config.read.mockImplementation(async () => {
        throw new Error('daemon down');
      });
      const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
      render(<ModeSection />);
      await waitFor(() => expect(logged).toHaveBeenCalledTimes(2));
      expect(screen.getByRole('button', { name: `${approvalsCopy.mode} Manual` })).toBeEnabled();
      expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toHaveValue(25);
      logged.mockRestore();
    });

    it('with nothing read and a failing daemon, falls back to the defaults as it always did', async () => {
      config.read.mockImplementation(async () => {
        throw new Error('daemon down');
      });
      const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
      render(<ModeSection />);
      expect(
        await screen.findByRole('button', { name: `${approvalsCopy.mode} Autonomous` })
      ).toBeEnabled();
      await waitFor(() =>
        expect(screen.getByRole('spinbutton', { name: approvalsCopy.maxTurns })).toHaveValue(0)
      );
      logged.mockRestore();
    });
  });
});
