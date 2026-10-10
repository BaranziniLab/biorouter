import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { SettingRow, SettingSection } from './setting-row';
import { Switch } from './switch';

describe('SettingRow', () => {
  it('names the control by its visible label and describes it with the help', () => {
    render(
      <SettingRow label="Prevent sleep while running" help="The screen can still lock.">
        <Switch checked={false} onCheckedChange={() => {}} />
      </SettingRow>
    );
    const control = screen.getByRole('switch', { name: 'Prevent sleep while running' });
    expect(control).toHaveAccessibleDescription('The screen can still lock.');
    // The help is an InfoTip beside the label, never inside it.
    const label = screen.getByText('Prevent sleep while running');
    expect(label.tagName).toBe('LABEL');
    expect(label.querySelector('.br-info-tip')).toBeNull();
    expect(screen.getByRole('button', { name: 'About Prevent sleep while running' })).toBeVisible();
  });

  it('uses the given controlId and keeps a control that names itself', () => {
    render(
      <SettingRow label="Text size" controlId="text-size">
        <Switch aria-label="Own name" checked={false} onCheckedChange={() => {}} />
      </SettingRow>
    );
    const control = screen.getByRole('switch', { name: 'Own name' });
    expect(control).toHaveAttribute('id', 'text-size');
    expect(screen.getByText('Text size')).toHaveAttribute('for', 'text-size');
  });

  it('shows a status line and a value, and describes the control with the status', () => {
    render(
      <SettingRow label="Updates" status="Restart to apply" value="1.92.1" valueMono>
        <Switch checked onCheckedChange={() => {}} />
      </SettingRow>
    );
    expect(screen.getByText('Restart to apply')).toHaveClass('text-supporting');
    expect(screen.getByText('1.92.1')).toHaveClass('font-mono');
    expect(screen.getByRole('switch', { name: 'Updates' })).toHaveAccessibleDescription(
      'Restart to apply'
    );
  });

  it('toggles a switch from a click on the row, but not from the InfoTip', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <SettingRow label="Spellcheck" help="Checks spelling." data-testid="row">
        <Switch checked={false} onCheckedChange={onChange} />
      </SettingRow>
    );
    await user.click(screen.getByTestId('row'));
    expect(onChange).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'About Spellcheck' }));
    expect(onChange).toHaveBeenCalledTimes(1);
    await user.click(screen.getByText('Spellcheck'));
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('carries the shared row class', () => {
    render(
      <SettingRow label="A" data-testid="row">
        <Switch checked={false} onCheckedChange={() => {}} />
      </SettingRow>
    );
    expect(screen.getByTestId('row')).toHaveClass('biorouter-settings-row');
  });
});

describe('SettingSection', () => {
  it('is a labelled region with a caps title, an InfoTip and an action, and no paragraph', () => {
    render(
      <SettingSection
        id="general"
        title="General"
        help="Applies to every chat."
        action={<button type="button">Reset</button>}
      >
        <SettingRow label="A">
          <Switch checked={false} onCheckedChange={() => {}} />
        </SettingRow>
      </SettingSection>
    );
    const region = screen.getByRole('region', { name: 'General' });
    expect(region).toHaveAttribute('id', 'general');
    expect(screen.getByRole('heading', { name: 'General' })).toHaveClass('text-caps');
    expect(screen.getByRole('button', { name: 'About General' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reset' })).toBeInTheDocument();
    expect(region.querySelector('p')).toBeNull();
    expect(region.querySelector('.biorouter-settings-list')).not.toBeNull();
  });
});
