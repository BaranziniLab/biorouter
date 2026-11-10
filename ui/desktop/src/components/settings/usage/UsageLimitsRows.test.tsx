import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settingsLimitsCopy as copy } from '../../usage/copy';
import { USAGE_LIMITS_CHANGED_EVENT } from '../../usage/usageLimits';
import {
  configuredLimitReading,
  limitsUpdate,
  UsageLimitsRows,
  type ConfiguredLimits,
} from './UsageLimitsRows';

/**
 * Settings > App > Usage's Limits group (owner requirement 3): the monthly limit and the default
 * limit per chat, "No limit" until set, in tokens or dollars (dollars only while Show costs is
 * on), saved through `PUT /usage/limits` with the person's proof.
 */
const mocks = vi.hoisted(() => ({
  put: vi.fn(),
  headers: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
  browser: false,
}));
vi.mock('../../../api', () => ({ putUsageLimits: mocks.put }));
vi.mock('../../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

const NONE: ConfiguredLimits = {
  monthlyTokenLimit: null,
  monthlyDollarLimit: null,
  defaultSessionTokenLimit: null,
  defaultSessionDollarLimit: null,
};

beforeEach(() => {
  mocks.browser = false;
  mocks.put.mockReset();
  mocks.headers.mockClear();
});

const row = (which: 'monthly' | 'defaultPerChat') => screen.getByTestId(`usage-limit-row-${which}`);

describe('the Limits group', () => {
  it('lists both limits as No limit by default, under a Limits heading', () => {
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={vi.fn()} />);
    expect(screen.getByRole('heading', { name: copy.groupTitle })).toBeInTheDocument();
    expect(row('monthly')).toHaveTextContent(copy.monthly);
    expect(row('monthly')).toHaveTextContent(copy.noLimit);
    expect(row('defaultPerChat')).toHaveTextContent(copy.defaultPerChat);
    expect(row('defaultPerChat')).toHaveTextContent(copy.noLimit);
  });

  it('shows the stored values, without dollar figures when Show costs is off', () => {
    const limits = { ...NONE, monthlyDollarLimit: 50, defaultSessionTokenLimit: 2_000_000 };
    const { rerender } = render(<UsageLimitsRows limits={limits} showCosts onSaved={vi.fn()} />);
    expect(row('monthly')).toHaveTextContent('$50.00');
    expect(row('defaultPerChat')).toHaveTextContent('2,000,000 tokens');
    rerender(<UsageLimitsRows limits={limits} showCosts={false} onSaved={vi.fn()} />);
    expect(row('monthly').textContent).not.toMatch(/\$/);
    expect(row('monthly')).toHaveTextContent(copy.dollarLimitHidden);
  });

  it('is pending, with Change waiting, while the first read runs', () => {
    render(<UsageLimitsRows limits={null} showCosts onSaved={vi.fn()} />);
    expect(row('monthly')).toHaveTextContent(copy.pending);
    expect(
      within(row('monthly')).getByRole('button', { name: copy.changeName(copy.monthly) })
    ).toBeDisabled();
  });

  it('saves a monthly dollar limit with the proof, hands the stored limits back and tells the composers', async () => {
    const user = userEvent.setup();
    const onSaved = vi.fn();
    const heard = vi.fn();
    window.addEventListener(USAGE_LIMITS_CHANGED_EVENT, heard);
    mocks.put.mockResolvedValue({ data: { monthlyDollarLimit: 50, monthlyTokenLimit: null } });
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={onSaved} />);
    await user.click(screen.getByRole('button', { name: copy.changeName(copy.monthly) }));
    const field = screen.getByLabelText(copy.amountLabel(copy.monthly));
    await waitFor(() => expect(field).toHaveFocus());
    expect(screen.getByRole('radio', { name: copy.dollars })).toBeChecked();
    await user.type(field, '50{Enter}');
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    window.removeEventListener(USAGE_LIMITS_CHANGED_EVENT, heard);
    expect(mocks.put).toHaveBeenCalledWith(
      expect.objectContaining({
        headers: { 'X-User-Action': 'proof' },
        body: { monthlyDollarLimit: 50, monthlyTokenLimit: null },
      })
    );
    expect(onSaved).toHaveBeenCalledWith({ ...NONE, monthlyDollarLimit: 50 });
    expect(heard).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('usage-limit-editor-monthly')).toBeNull();
  });

  it('saves a default per-chat token limit, and removes one with No limit', async () => {
    const user = userEvent.setup();
    mocks.put.mockResolvedValue({ data: {} });
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: copy.changeName(copy.defaultPerChat) }));
    await user.click(screen.getByRole('radio', { name: copy.tokens }));
    await user.type(screen.getByLabelText(copy.amountLabel(copy.defaultPerChat)), '500k');
    await user.click(screen.getByRole('button', { name: copy.save }));
    await waitFor(() =>
      expect(mocks.put).toHaveBeenLastCalledWith(
        expect.objectContaining({
          body: { defaultSessionTokenLimit: 500_000, defaultSessionDollarLimit: null },
        })
      )
    );

    await user.click(screen.getByRole('button', { name: copy.changeName(copy.defaultPerChat) }));
    await user.click(
      screen.getByRole('button', { name: copy.removeLimitName(copy.defaultPerChat) })
    );
    await waitFor(() =>
      expect(mocks.put).toHaveBeenLastCalledWith(
        expect.objectContaining({
          body: { defaultSessionTokenLimit: null, defaultSessionDollarLimit: null },
        })
      )
    );
  });

  it('offers tokens only with Show costs off, and refuses what is not an amount', async () => {
    const user = userEvent.setup();
    render(<UsageLimitsRows limits={NONE} showCosts={false} onSaved={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: copy.changeName(copy.monthly) }));
    expect(screen.queryByRole('radio')).toBeNull();
    const field = screen.getByLabelText(copy.amountLabel(copy.monthly));
    await user.type(field, 'zero{Enter}');
    expect(screen.getByRole('alert')).toHaveTextContent(copy.invalidTokens);
    expect(mocks.put).not.toHaveBeenCalled();
  });

  it('says a refused write could not be tied to a person, and keeps the editor open', async () => {
    const user = userEvent.setup();
    mocks.put.mockResolvedValue({ data: undefined, response: { status: 403 } });
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: copy.changeName(copy.monthly) }));
    await user.type(screen.getByLabelText(copy.amountLabel(copy.monthly)), '5{Enter}');
    expect(await screen.findByRole('alert')).toHaveTextContent(copy.needUser);
    expect(screen.getByTestId('usage-limit-editor-monthly')).toBeInTheDocument();
  });

  it('cancels with Escape and hands focus back to Change', async () => {
    const user = userEvent.setup();
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={vi.fn()} />);
    const change = screen.getByRole('button', { name: copy.changeName(copy.monthly) });
    await user.click(change);
    await waitFor(() =>
      expect(screen.getByLabelText(copy.amountLabel(copy.monthly))).toHaveFocus()
    );
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('usage-limit-editor-monthly')).toBeNull();
    await waitFor(() => expect(change).toHaveFocus());
  });

  it('says where to change the limits in a browser, with nothing to press', () => {
    mocks.browser = true;
    render(<UsageLimitsRows limits={NONE} showCosts onSaved={vi.fn()} />);
    expect(row('monthly')).toHaveTextContent(copy.browserNote);
    expect(
      within(row('monthly')).getByRole('button', { name: copy.changeName(copy.monthly) })
    ).toBeDisabled();
  });
});

describe('a row’s write', () => {
  it('clears the other unit, and leaves a hidden dollar limit alone with Show costs off', () => {
    expect(limitsUpdate('monthly', { unit: 'usd', value: 5 }, true)).toEqual({
      monthlyDollarLimit: 5,
      monthlyTokenLimit: null,
    });
    expect(limitsUpdate('monthly', { unit: 'tokens', value: 9 }, true)).toEqual({
      monthlyTokenLimit: 9,
      monthlyDollarLimit: null,
    });
    expect(limitsUpdate('defaultPerChat', { unit: 'tokens', value: 9 }, false)).toEqual({
      defaultSessionTokenLimit: 9,
    });
    expect(limitsUpdate('defaultPerChat', null, false)).toEqual({
      defaultSessionTokenLimit: null,
      defaultSessionDollarLimit: null,
    });
  });

  it('reads a stored 0 as no limit, as the daemon does', () => {
    expect(configuredLimitReading({ ...NONE, monthlyTokenLimit: 0 }, 'monthly', true)).toBe(
      copy.noLimit
    );
    expect(
      configuredLimitReading(
        { ...NONE, monthlyTokenLimit: 1_000, monthlyDollarLimit: 2 },
        'monthly',
        true
      )
    ).toBe('$2.00 · 1,000 tokens');
  });
});
