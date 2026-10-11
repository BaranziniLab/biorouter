import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import type { LintResult } from '../../../api/types.gen';
import { knowledgeCopy } from '../copy';
import { LintDrawer } from './LintDrawer';

const mocks = vi.hoisted(() => ({
  start: vi.fn(),
  reset: vi.fn(),
  knowledge: {
    primaryKbId: 'kb-1' as string | null,
    primaryKb: { id: 'kb-1', name: 'Notes', default_model: null } as {
      id: string;
      name: string;
      default_model: { provider: string; model: string } | null;
    } | null,
  },
  stream: {
    events: [] as unknown[],
    status: 'done' as 'idle' | 'starting' | 'streaming' | 'done' | 'error',
    finalResult: null as unknown,
    error: undefined as string | undefined,
  },
  // Settable, because the provider bound to the composer is the input that
  // decides whether this drawer can run at all — see the provider-exclusion
  // block at the bottom of this file.
  modelAndProvider: {
    currentProvider: 'anthropic' as string | null,
    currentModel: 'claude' as string | null,
  },
}));

vi.mock('../KnowledgeContext', () => ({ useKnowledge: () => mocks.knowledge }));

vi.mock('../../ModelAndProviderContext', () => ({
  useModelAndProvider: () => mocks.modelAndProvider,
}));

vi.mock('../hooks/useIngestStream', () => ({
  useIngestStream: () => ({
    ...mocks.stream,
    start: mocks.start,
    startMultipart: vi.fn(),
    abort: vi.fn(),
    reset: mocks.reset,
  }),
}));

/** A report with one of each severity, plus a capped total and one hygiene list. */
const REPORT: LintResult = {
  fixes_applied: 0,
  report: {
    contradictions: [],
    orphans: ['knowledge/lonely.md'],
    missing_concept_pages: [],
    stale_sources: [],
    diagnostics: {
      total: 9,
      items: [
        {
          rule: 'okf.type.unknown',
          severity: 'error',
          subject: 'Metformin',
          message: 'type "Drugg" is not in the vocabulary',
          path: 'knowledge/metformin.md',
        },
        {
          rule: 'biookf.predicate.unknown',
          severity: 'warning',
          subject: 'Metformin -tretas-> Diabetes',
          message: 'predicate "tretas" is not in the BioOKF vocabulary',
        },
        {
          rule: 'kb.orphan',
          severity: 'info',
          subject: 'knowledge/lonely.md',
          message: 'nothing links to this page',
        },
      ],
    },
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.knowledge = {
    primaryKbId: 'kb-1',
    primaryKb: { id: 'kb-1', name: 'Notes', default_model: null },
  };
  mocks.stream = { events: [], status: 'done', finalResult: REPORT, error: undefined };
  mocks.modelAndProvider = { currentProvider: 'anthropic', currentModel: 'claude' };
});

describe('LintDrawer', () => {
  it('closes through a named close in the pane header, beside the Check button', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<LintDrawer open onOpenChange={onOpenChange} />);

    const close = screen.getByRole('button', { name: knowledgeCopy.drawers.closeLint });
    const header = close.closest('.br-knowledge-pane-header');
    expect(header).not.toBeNull();
    expect(within(header as HTMLElement).getByTestId('knowledge-lint-run')).toBeInTheDocument();

    await user.click(close);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('runs the check on open, against the base and the resolved model', () => {
    mocks.knowledge.primaryKbId = 'stale-pointer';
    render(<LintDrawer open onOpenChange={() => undefined} />);

    expect(mocks.start).toHaveBeenCalledTimes(1);
    expect(mocks.start).toHaveBeenCalledWith('/knowledge/bases/kb-1/lint', {
      model: { provider: 'anthropic', model: 'claude' },
    });
  });

  it('never asks for an autofix — the check reports, it does not rewrite', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);
    const body = mocks.start.mock.calls[0][1] as Record<string, unknown>;
    expect(body).not.toHaveProperty('autofix');
  });

  it('does not run while the drawer is closed', () => {
    render(<LintDrawer open={false} onOpenChange={() => undefined} />);
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it('groups the diagnostics by severity', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);

    for (const [group, subject] of [
      ['Errors', 'Metformin'],
      ['Warnings', 'Metformin -tretas-> Diabetes'],
      ['Notes', 'knowledge/lonely.md'],
    ] as const) {
      expect(
        within(screen.getByRole('region', { name: group })).getByText(subject)
      ).toBeInTheDocument();
    }
  });

  it('shows the stable rule id beside every finding, not just the prose', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);
    for (const rule of ['okf.type.unknown', 'biookf.predicate.unknown', 'kb.orphan']) {
      expect(screen.getByText(rule)).toBeInTheDocument();
    }
  });

  it('reports the count BEFORE the cap, and says the list is capped', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);
    // 9 were raised; 3 came back. A surface that reported 3 would be telling the
    // user their base is fine when two thirds of the findings never arrived.
    expect(screen.getByTestId('knowledge-lint-count')).toHaveTextContent('9 findings');
    expect(screen.getByText(/first 3 shown/i)).toBeInTheDocument();
  });

  it('surfaces the hygiene lists the report carries beside the diagnostics', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);
    expect(
      within(screen.getByRole('region', { name: 'Orphans' })).getByText('knowledge/lonely.md')
    ).toBeInTheDocument();
  });

  it('says so plainly when there is nothing to fix', () => {
    mocks.stream.finalResult = {
      fixes_applied: 0,
      report: {
        contradictions: [],
        orphans: [],
        missing_concept_pages: [],
        stale_sources: [],
        diagnostics: { total: 0, items: [] },
      },
    } satisfies LintResult;
    render(<LintDrawer open onOpenChange={() => undefined} />);
    expect(screen.getByRole('heading', { name: 'Nothing to fix' })).toBeInTheDocument();
  });

  it('reports a failed stream as a failure rather than an empty base', () => {
    mocks.stream = {
      events: [],
      status: 'error',
      finalResult: null,
      error: 'HTTP 400: invalid model',
    };
    render(<LintDrawer open onOpenChange={() => undefined} />);
    expect(screen.getByRole('heading', { name: 'The check did not finish' })).toBeInTheDocument();
    expect(screen.getByText('HTTP 400: invalid model')).toBeInTheDocument();
    // A stream that died leaves NO report on screen to be mistaken for one.
    expect(screen.queryByTestId('knowledge-lint-diagnostic')).toBeNull();
  });

  it('dispatches nothing when there is no base to check', async () => {
    mocks.stream = { events: [], status: 'idle', finalResult: null, error: undefined };
    mocks.knowledge = { primaryKbId: null, primaryKb: null };
    render(<LintDrawer open onOpenChange={() => undefined} />);

    expect(mocks.start).not.toHaveBeenCalled();
    // And the control cannot be used to get there either — an agentic loop
    // dispatched at `/knowledge/bases/null/lint` is a request nothing answers.
    const run = screen.getByTestId('knowledge-lint-run');
    expect(run).toBeDisabled();
    await userEvent.click(run);
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it('treats a primary id without a resolved manifest as unavailable', async () => {
    mocks.stream = { events: [], status: 'idle', finalResult: null, error: undefined };
    mocks.knowledge = { primaryKbId: 'kb-stale', primaryKb: null };
    const { rerender } = render(<LintDrawer open onOpenChange={() => undefined} />);

    expect(screen.getByRole('heading', { name: 'Knowledge base unavailable' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'No model is configured' })).toBeNull();
    expect(mocks.start).not.toHaveBeenCalled();

    const run = screen.getByTestId('knowledge-lint-run');
    expect(run).toBeDisabled();
    await userEvent.click(run);
    expect(mocks.start).not.toHaveBeenCalled();

    mocks.knowledge.primaryKb = { id: 'kb-stale', name: 'Notes', default_model: null };
    rerender(<LintDrawer open onOpenChange={() => undefined} />);

    expect(screen.queryByRole('heading', { name: 'Knowledge base unavailable' })).toBeNull();
    expect(mocks.start).toHaveBeenCalledWith('/knowledge/bases/kb-stale/lint', {
      model: { provider: 'anthropic', model: 'claude' },
    });
  });
});

/**
 * ⚠ **Asserted here AND in a real browser.** This drawer once warned on every
 * open while the suite stayed green. The local spy pins that warning as well as
 * the DOM attribute that actually matters.
 */
describe('LintDrawer — the description contract', () => {
  let warn: MockInstance;

  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  afterEach(() => {
    warn.mockRestore();
  });

  it('leaves Radix no aria-describedby to dangle', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);

    expect(screen.getByTestId('knowledge-lint-drawer').hasAttribute('aria-describedby')).toBe(
      false
    );
    expect(warn.mock.calls.some((call) => String(call[0]).includes('Missing `Description`'))).toBe(
      false
    );
  });
});

/**
 * ⚠ **This drawer is the SECOND consumer of `resolveIngestModel`**, and that is
 * the whole reason these tests exist.
 *
 * When the resolver learned to refuse `claude_code` / `codex` (they reach
 * `complete_with_model` with `tools` dropped, so a macro run narrates its calls
 * and writes nothing), it started returning `null` for a configuration that is
 * correct and working. This surface read `null` as "the user has not set a model
 * up" — so a user whose only provider is a coding agent had the run control
 * permanently disabled and was told to choose a model in the Sources rail, which
 * is precisely the picker the same change emptied for them. A loop with no exit.
 *
 * The obstacle is real and the drawer must stay disabled; what it may not do is
 * misname it.
 */
describe('LintDrawer — a coding-agent model is a model like any other (#109)', () => {
  beforeEach(() => {
    mocks.stream = { events: [], status: 'idle', finalResult: null, error: undefined };
  });

  it('runs a check on a coding-agent model instead of refusing it', () => {
    // This drawer used to draw "This model can't run a check" here, because
    // `resolveIngestModel` refused these providers by name and the drawer read
    // the resulting `null` as "nothing is configured". Both halves are gone: a
    // macro turn carries its tools over the MCP bridge, so a check under Claude
    // Code reads the base for real.
    mocks.modelAndProvider = { currentProvider: 'claude_code', currentModel: 'opus-5' };
    render(<LintDrawer open onOpenChange={() => undefined} />);

    expect(screen.queryByRole('heading', { name: 'No model is configured' })).toBeNull();
    expect(
      screen.queryByRole('heading', { name: /can’t run a check|cannot run a check/i })
    ).toBeNull();
    expect(mocks.start).toHaveBeenCalledWith('/knowledge/bases/kb-1/lint', {
      model: { provider: 'claude_code', model: 'opus-5' },
    });
  });

  it('keeps saying "no model" when there genuinely is none', () => {
    mocks.modelAndProvider = { currentProvider: null, currentModel: null };
    render(<LintDrawer open onOpenChange={() => undefined} />);

    // The one verdict this drawer may reach on its own, and it has to survive
    // the removal above: a user with no provider at all still needs telling.
    expect(screen.getByRole('heading', { name: 'No model is configured' })).toBeInTheDocument();
    expect(mocks.start).not.toHaveBeenCalled();
  });
});

// V3-R2-1 (from WS-VIEWS-A, WS-VIEWS-A-3): the check starts as the drawer opens, while focus is
// arriving on the Check button, so a button that turned `disabled` for the run would hand focus to
// <body> at once. It is `aria-busy` instead: still enabled and focused, the press ignored.
describe('LintDrawer while the check runs', () => {
  it('keeps the Check button enabled, focused and busy, and ignores the press', async () => {
    mocks.stream = { events: [], status: 'streaming', finalResult: null, error: undefined };
    render(<LintDrawer open onOpenChange={() => undefined} />);

    const run = screen.getByTestId('knowledge-lint-run');
    expect(run).toBeEnabled();
    expect(run).toHaveAttribute('aria-disabled', 'true');
    expect(run).toHaveAttribute('aria-busy', 'true');
    expect(run).toHaveTextContent('Checking…');

    const started = mocks.start.mock.calls.length;
    run.focus();
    await userEvent.click(run);
    expect(mocks.start).toHaveBeenCalledTimes(started);
    expect(run).toHaveFocus();
  });

  it('is an ordinary enabled button when nothing is running', () => {
    render(<LintDrawer open onOpenChange={() => undefined} />);
    const run = screen.getByTestId('knowledge-lint-run');
    expect(run).toBeEnabled();
    expect(run).not.toHaveAttribute('aria-busy');
    expect(run).not.toHaveAttribute('aria-disabled');
  });
});
