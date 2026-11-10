import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { parseDecisionModelEntries } from '../../utils/sessionDecisions';
import { WORKFLOW_SUPERVISION_COPY, WorkflowWarningModal } from './WorkflowWarningModal';

vi.mock('../MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
}));

const listing = vi.hoisted(() => ({ fetch: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  fetchDecisionModels: listing.fetch,
}));

beforeEach(() => {
  listing.fetch.mockReset();
  listing.fetch.mockResolvedValue([]);
});

/** `GET /decisions/models`, with each model's tier. */
const MODELS = parseDecisionModelEntries([
  {
    provider: 'ollama_decision',
    model: 'tev1:0.8b',
    displayName: 'tev1:0.8b',
    kinds: ['binary'],
    tier: 'private',
    local: true,
  },
  {
    provider: 'baseten_decision',
    model: 'decision-models/baseten-joey-2b-latest',
    displayName: 'joey-1',
    kinds: ['binary'],
    tier: 'public',
  },
])!;

/** The two blocks of `stop.yaml` (P12, P16), as the parsed workflow carries them. */
const SUPERVISION = {
  decisionSettings: {
    defaultModel: { provider: 'baseten_decision', model: 'decision-models/baseten-joey-2b-latest' },
    monitors: [
      {
        id: 'reached-ten',
        name: 'reached-ten',
        question: {
          instructions: 'Has the agent already echoed 10 or a larger number?',
          kind: { binary: {} },
        },
        threshold: 0.7,
        action: 'stop',
        onError: 'pause',
      },
      {
        id: 'secrets',
        name: 'Secrets',
        question: { instructions: 'Did the agent print a secret?', kind: { binary: {} } },
        model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
        threshold: 0.8,
        action: 'notify',
        enabled: false,
      },
    ],
  },
  orchestrationPlan: {
    title: 'Echo plan',
    phases: [
      {
        id: 'count',
        title: 'Count',
        gate: {
          questions: { done: 'Did the agent reach 30?', clean: 'Was each echo its own call?' },
          threshold: 0.9,
          max_attempts: 1,
          model: 'ollama_decision/tev1:0.8b',
        },
      },
      { id: 'report', title: 'Report', gate: { questions: { said: 'Is there a report?' } } },
      { id: 'tidy', title: 'Tidy' },
    ],
  },
};

const details = {
  title: 'Cohort summary',
  description: 'Summarizes a cohort.',
  instructions: 'Read the table.',
};

describe('WorkflowWarningModal', () => {
  it('is the shared dialog: modal, on the dialog motion, with no close button', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveClass('br-dialog-motion');
    // The old hand-rolled content used Tailwind's 95% zoom and a 200ms duration of its own.
    expect(dialog.className).not.toMatch(/zoom-(in|out)-95|duration-200/);
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
  });

  it('has the shared anatomy: 640px, pinned at the top, the title above the header rule (V3-R7-1)', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    const dialog = screen.getByRole('dialog');
    // ModalShell's lg rung and top anchor, not the raw 80vw centred content with 24px insets.
    expect(dialog).toHaveAttribute('data-size', 'lg');
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    expect(dialog.className).not.toMatch(/80vw|\bp-6\b/);
    const header = dialog.firstElementChild as HTMLElement;
    expect(header).toHaveClass('border-b', 'border-border-subtle');
    expect(header).toContainElement(screen.getByRole('heading', { name: 'New workflow warning' }));
    // The warning sentence is the description, in the body, not in the header.
    expect(dialog).toHaveAccessibleDescription(
      "You are about to execute a workflow that you haven't run before. Only proceed if you trust the source of this workflow."
    );
    expect(header).not.toHaveTextContent('Only proceed');
    // Cancel, then the action at the trailing edge, in the footer row.
    const buttons = screen.getAllByRole('button').map((b) => b.textContent);
    expect(buttons).toEqual(['Cancel', 'Trust and execute']);
  });

  it('says its title in sentence case, without an emoji', () => {
    const { rerender } = render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByRole('heading', { name: 'New workflow warning' })).toBeInTheDocument();
    rerender(
      <WorkflowWarningModal
        isOpen
        hasSecurityWarnings
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByRole('heading', { name: 'Security warning' })).toBeInTheDocument();
    expect(screen.getByText(/hidden characters that will be ignored/)).toBeInTheDocument();
  });

  it('makes the person answer: Escape does nothing, the two buttons decide', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={onConfirm}
        onCancel={onCancel}
        workflowDetails={details}
      />
    );
    await user.keyboard('{Escape}');
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Trust and execute' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('shows what is being trusted', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    expect(screen.getByText('Cohort summary')).toBeInTheDocument();
    expect(screen.getByText('Summarizes a cohort.')).toBeInTheDocument();
    expect(screen.getByTestId('markdown')).toHaveTextContent('Read the table.');
  });

  it('draws the two field names at 600, never a bare strong (V4-R7-6)', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={details}
      />
    );
    // A bare <strong> takes preflight's `bolder`, which computes to 700; the type system draws 600.
    // jsdom has no preflight, so the class is what this can see (Chromium measures the weight in
    // modalEmphasisWeight.browser.test.tsx).
    const strong = Array.from(screen.getByRole('dialog').querySelectorAll('strong'));
    expect(strong.map((s) => s.textContent)).toEqual(['Title', 'Description']);
    for (const s of strong) expect(s).toHaveClass('font-semibold');
  });

  it('lists the monitors and phase gates a workflow declares', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={{ ...details, supervision: SUPERVISION }}
        decisionModels={MODELS}
      />
    );
    const panel = screen.getByRole('region', { name: WORKFLOW_SUPERVISION_COPY.title });
    expect(within(panel).getByTestId('workflow-supervision-default')).toHaveTextContent(
      'Default decision model: joey-1 · Public'
    );
    // Name; model or "chat default"; a named model's tier; what it does.
    expect(within(panel).getByTestId('workflow-supervision-monitor-reached-ten')).toHaveTextContent(
      'reached-ten · chat default · stops the turn'
    );
    expect(within(panel).getByTestId('workflow-supervision-monitor-secrets')).toHaveTextContent(
      'Secrets · tev1:0.8b · Private · notifies you · off'
    );
    expect(within(panel).getByText('Phase plan “Echo plan”, 3 phases.')).toBeInTheDocument();
    expect(within(panel).getByTestId('workflow-supervision-gate-count')).toHaveTextContent(
      'Count · 2 questions at P(yes) ≥ 0.90, 1 attempt · tev1:0.8b · Private'
    );
    // A gate that names nothing takes the daemon's defaults.
    expect(within(panel).getByTestId('workflow-supervision-gate-report')).toHaveTextContent(
      'Report · 1 question at P(yes) ≥ 0.80, 3 attempts · chat default'
    );
    expect(within(panel).queryByTestId('workflow-supervision-gate-tidy')).toBeNull();
    expect(within(panel).getByText(WORKFLOW_SUPERVISION_COPY.reads)).toBeInTheDocument();
    expect(within(panel).getByText(WORKFLOW_SUPERVISION_COPY.gatesBind)).toBeInTheDocument();
    // Still only the two answers, and still no bare emphasis added.
    expect(screen.getAllByRole('button').map((b) => b.textContent)).toEqual([
      'Cancel',
      'Trust and execute',
    ]);
    expect(
      Array.from(screen.getByRole('dialog').querySelectorAll('strong')).map((s) => s.textContent)
    ).toEqual(['Title', 'Description']);
    expect(listing.fetch).not.toHaveBeenCalled();
  });

  it('reads the tiers from the decision model list when it was given none', async () => {
    listing.fetch.mockResolvedValue(MODELS);
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={{ ...details, supervision: SUPERVISION }}
      />
    );
    expect(await screen.findByText('Default decision model: joey-1 · Public')).toBeInTheDocument();
    expect(listing.fetch).toHaveBeenCalledTimes(1);
  });

  it('names a model by itself when the list cannot be read', async () => {
    listing.fetch.mockRejectedValue(new Error('offline'));
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={{ ...details, supervision: SUPERVISION }}
      />
    );
    await vi.waitFor(() => expect(listing.fetch).toHaveBeenCalled());
    // The model's own name (its Baseten alias), with no tier the list could not give.
    expect(screen.getByTestId('workflow-supervision-default')).toHaveTextContent(
      /^Default decision model: joey-1$/
    );
    expect(screen.getByTestId('workflow-supervision-default')).not.toHaveTextContent('Public');
  });

  it('shows no Supervision panel for a workflow that declares none', () => {
    render(
      <WorkflowWarningModal
        isOpen
        onConfirm={() => {}}
        onCancel={() => {}}
        workflowDetails={{ ...details, supervision: { decisionSettings: null } }}
      />
    );
    expect(screen.queryByTestId('workflow-supervision')).toBeNull();
    expect(listing.fetch).not.toHaveBeenCalled();
  });

  it('is given the workflow’s two blocks by the chat that runs it', () => {
    const BASECHAT = readFileSync(join(__dirname, '..', 'BaseChat.tsx'), 'utf8');
    const modal = BASECHAT.match(/<WorkflowWarningModal[\s\S]*?\n {8}\/>/)?.[0] ?? '';
    expect(modal).toMatch(
      /supervision: \{\s*decisionSettings: workflow\.decision_settings,\s*orchestrationPlan: workflow\.orchestration_plan,\s*\}/
    );
  });
});
