import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  PublicDecisionConfirmationError,
  parseDecisionModelEntries,
  parseDecisionSettings,
  type DecisionSettings,
} from '../../utils/sessionDecisions';
import { MonitorsDialog } from './MonitorsDialog';

/**
 * DR-33 (design §5, the Monitors row): on a chat whose model is private and that can
 * still be made public, a public decision model is offered with "Makes this chat
 * public", the editor says "This chat will be public: …" above Save, and Save sends
 * the person's answer. A write the daemon asks about (409) is asked in the dialog and
 * sent again with the answer. Before the first message (draft mode) nothing is
 * written to the daemon: the draft is handed to the composer.
 */

const mocks = vi.hoisted(() => ({ save: vi.fn(), catalog: vi.fn(), settings: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  saveDecisionSettings: mocks.save,
  fetchDecisionCatalog: mocks.catalog,
  fetchDecisionSettings: mocks.settings,
}));

const MODELS = parseDecisionModelEntries([
  {
    provider: 'ollama_decision',
    model: 'tev1:0.8b',
    displayName: 'tev1:0.8b',
    contextTokens: 32768,
    kinds: ['binary'],
    tier: 'private',
    local: true,
  },
  {
    provider: 'baseten_decision',
    model: 'joey-1',
    displayName: 'joey-1',
    contextTokens: 16384,
    kinds: ['binary'],
    tier: 'public',
    local: false,
    chatEffect: 'makesPublic',
  },
])!;

const monitorOn = (model: string, enabled: boolean, id = 'done', name = 'Done') =>
  parseDecisionSettings({
    defaultModel: null,
    monitors: [
      {
        id,
        name,
        question: { instructions: 'Is the task done?', kind: { binary: {} } },
        threshold: 0.85,
        action: 'notify',
        enabled,
        model: { provider: model.includes(':') ? 'ollama_decision' : 'baseten_decision', model },
      },
    ],
  })!;

const QUESTION = {
  message: 'This chat will be public: joey-1 runs on Baseten.',
  info: 'A chat is as private as its least private model.',
  models: [{ provider: 'baseten_decision', model: 'joey-1', adapter: null }],
};

beforeEach(() => {
  mocks.save.mockReset().mockImplementation(async (_id: string, settings: DecisionSettings) => ({
    ...settings,
    tierEffect: 'none',
    tierEffectLine: null,
  }));
  mocks.catalog.mockReset().mockResolvedValue({ models: MODELS, sessionUnavailableReason: null });
  mocks.settings.mockReset();
});

function draw(
  settings: DecisionSettings,
  props: Partial<Parameters<typeof MonitorsDialog>[0]> = {}
) {
  mocks.settings.mockResolvedValue({ data: settings, missing: false });
  return render(
    <MonitorsDialog
      open
      onOpenChange={vi.fn()}
      sessionId="s1"
      chatTier="public"
      settings={settings}
      models={MODELS}
      chatModelName="gpt-5.5"
      {...props}
    />
  );
}

async function ready() {
  await waitFor(() => expect(screen.queryByTestId('monitors-loading')).toBeNull());
}

async function editDone() {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'More actions for Done' }));
  await user.click(await screen.findByRole('menuitem', { name: 'Edit' }));
}

describe('a chat its decision models would make public', () => {
  it('offers the public model with the consequence on its row', async () => {
    draw({ defaultModel: null, monitors: [] });
    await ready();
    fireEvent.click(screen.getByTestId('monitor-add'));
    fireEvent.click(
      within(screen.getByTestId('monitor-editor')).getByRole('button', { name: 'More options' })
    );
    const input = within(screen.getByTestId('monitor-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    const rows = await screen.findAllByTestId('decision-model-option');
    const joey = rows.find((row) => row.textContent?.startsWith('joey-1'))!;
    expect(joey).toHaveTextContent('Baseten · Makes this chat public');
    expect(joey.closest('[aria-disabled="true"]')).toBeNull();
  });

  it('says so above Save, and Save is the person’s answer', async () => {
    draw(monitorOn('joey-1', true));
    await ready();
    await editDone();
    expect(screen.getByTestId('monitor-tier-effect')).toHaveTextContent(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    expect(
      screen.getByRole('button', { name: 'About Why this chat will be public' })
    ).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('monitor-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    expect(mocks.save.mock.calls[0][2]).toEqual({ acceptPublicDecisionModels: true });
  });

  it('a save that keeps the chat private sends no answer and shows no line', async () => {
    draw(monitorOn('tev1:0.8b', true));
    await ready();
    await editDone();
    expect(screen.queryByTestId('monitor-tier-effect')).toBeNull();
    fireEvent.click(screen.getByTestId('monitor-save'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1));
    expect(mocks.save.mock.calls[0][2]).toEqual({ acceptPublicDecisionModels: false });
  });

  it('asks when the daemon asks, and sends the same write again with the answer', async () => {
    mocks.save.mockRejectedValueOnce(new PublicDecisionConfirmationError(QUESTION));
    draw(monitorOn('joey-1', false));
    await ready();
    fireEvent.click(screen.getByTestId('monitor-switch'));
    const asking = await screen.findByTestId('monitors-public-confirm');
    expect(asking).toHaveTextContent('This chat will be public: joey-1 runs on Baseten.');
    expect(mocks.save.mock.calls[0][2]).toEqual({ acceptPublicDecisionModels: false });
    fireEvent.click(screen.getByTestId('monitors-public-accept'));
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(2));
    expect(mocks.save.mock.calls[1][2]).toEqual({ acceptPublicDecisionModels: true });
    expect((mocks.save.mock.calls[1][1] as DecisionSettings).monitors[0].enabled).toBe(true);
    await waitFor(() => expect(screen.queryByTestId('monitors-public-confirm')).toBeNull());
  });

  it('Cancel leaves the question unanswered and writes nothing more', async () => {
    mocks.save.mockRejectedValueOnce(new PublicDecisionConfirmationError(QUESTION));
    draw(monitorOn('joey-1', false));
    await ready();
    fireEvent.click(screen.getByTestId('monitor-switch'));
    await screen.findByTestId('monitors-public-confirm');
    fireEvent.click(screen.getByTestId('monitors-public-cancel'));
    expect(screen.queryByTestId('monitors-public-confirm')).toBeNull();
    expect(mocks.save).toHaveBeenCalledTimes(1);
  });

  it('a removal that makes the chat private at its next message says so', async () => {
    mocks.save.mockImplementationOnce(async (_id: string, settings: DecisionSettings) => ({
      ...settings,
      tierEffect: 'becomesPrivateAtNextMessage',
      tierEffectLine: 'This chat becomes private at your next message.',
    }));
    draw(monitorOn('joey-1', true));
    await ready();
    fireEvent.click(screen.getByTestId('monitor-switch'));
    expect(await screen.findByTestId('monitors-tier-effect')).toHaveTextContent(
      'This chat becomes private at your next message.'
    );
  });
});

describe('draft mode, before the chat’s first message', () => {
  const drawDraft = (settings: DecisionSettings, accepted = false) => {
    const onChange = vi.fn();
    render(
      <MonitorsDialog
        open
        onOpenChange={vi.fn()}
        sessionId=""
        chatTier="public"
        settings={settings}
        models={MODELS}
        draft={{
          chat: { boundProvider: 'versa_azure', boundModel: 'gpt-5.5' },
          accepted,
          onChange,
        }}
      />
    );
    return onChange;
  };

  it('writes nothing to the daemon and says the monitors start with the chat', async () => {
    const onChange = drawDraft(monitorOn('tev1:0.8b', false));
    expect(screen.getByTestId('monitors-draft-note')).toHaveTextContent(
      'These monitors start with the chat when you send its first message.'
    );
    fireEvent.click(screen.getByTestId('monitor-switch'));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(onChange.mock.calls[0][0].monitors[0].enabled).toBe(true);
    expect(onChange.mock.calls[0][1]).toBe(false);
    expect(mocks.save).not.toHaveBeenCalled();
    expect(mocks.settings).not.toHaveBeenCalled();
  });

  it('asks before a change that makes the chat public, and hands the answer on', async () => {
    const onChange = drawDraft(monitorOn('joey-1', false));
    fireEvent.click(screen.getByTestId('monitor-switch'));
    expect(await screen.findByTestId('monitors-public-confirm')).toHaveTextContent(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('monitors-public-accept'));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(onChange.mock.calls[0][1]).toBe(true);
    expect(onChange.mock.calls[0][0].monitors[0].enabled).toBe(true);
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it('a change that adds no public model to an accepted draft asks nothing again', async () => {
    const settings = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'done',
          name: 'Done',
          question: { instructions: 'Is the task done?', kind: { binary: {} } },
          threshold: 0.85,
          action: 'notify',
          enabled: true,
          model: { provider: 'baseten_decision', model: 'joey-1' },
        },
        {
          id: 'local',
          name: 'Local',
          question: { instructions: 'Is it stuck?', kind: { binary: {} } },
          threshold: 0.85,
          action: 'notify',
          enabled: false,
          model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
        },
      ],
    })!;
    const onChange = drawDraft(settings, true);
    const local = screen
      .getAllByTestId('monitor-item')
      .find((row) => row.getAttribute('data-monitor-id') === 'local')!;
    fireEvent.click(within(local).getByTestId('monitor-switch'));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('monitors-public-confirm')).toBeNull();
    expect(onChange.mock.calls[0][1]).toBe(true);
  });

  it('reads the catalog for the model the chat will start on', async () => {
    render(
      <MonitorsDialog
        open
        onOpenChange={vi.fn()}
        sessionId=""
        chatTier="public"
        settings={{ defaultModel: null, monitors: [] }}
        draft={{ chat: { boundProvider: 'versa_azure', boundModel: 'gpt-5.5' }, onChange: vi.fn() }}
      />
    );
    await waitFor(() => expect(mocks.catalog).toHaveBeenCalled());
    expect(mocks.catalog.mock.calls[0][1]).toEqual({
      boundProvider: 'versa_azure',
      boundModel: 'gpt-5.5',
    });
  });
});
