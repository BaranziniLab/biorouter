import { fireEvent, render, screen } from '@testing-library/react';
import { components as selectComponents } from 'react-select';
import { describe, expect, it, vi } from 'vitest';
import { MENU_GROUP_LABEL_CLASS_NAME } from './dropdown-menu';
import { Select } from './Select';

/**
 * A group heading is drawn only where the group is named (spec 2.6, V4-R8-1). react-select draws a
 * heading row for every group, and an unlabeled one left 8px of blank above the first option of
 * the Switch models Model menu. The measured offsets (13px against the Provider menu's 5px) are
 * held in Chromium by `selectGroupHeading.browser.test.tsx`; this file holds the rule itself.
 */
const MODELS = [
  { value: 'gpt-4o', label: 'gpt-4o' },
  { value: 'o3', label: 'o3' },
];

const headings = (container: HTMLElement) => container.querySelectorAll('.s__group-heading');

describe('the shared Select’s group headings', () => {
  it('draws none for a single unlabeled group, and still lists every option', () => {
    const { container } = render(
      <Select aria-label="Model" classNamePrefix="s" menuIsOpen options={[{ options: MODELS }]} />
    );
    expect(headings(container)).toHaveLength(0);
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['gpt-4o', 'o3']);
    // The group box is there (it is what holds the rows), with the options first in it.
    const group = container.querySelector('.s__group');
    expect(group).not.toBeNull();
    expect(group!.firstElementChild!.querySelector('[role="option"]')).not.toBeNull();
  });

  it('draws none for an empty or blank label', () => {
    for (const label of ['', '   ', undefined]) {
      const { container, unmount } = render(
        <Select
          aria-label="Model"
          classNamePrefix="s"
          menuIsOpen
          options={[{ label, options: MODELS } as never]}
        />
      );
      expect(headings(container)).toHaveLength(0);
      unmount();
    }
  });

  it('still draws a named group’s heading, in the one group-label style', () => {
    const { container } = render(
      <Select
        aria-label="Provider"
        classNamePrefix="s"
        menuIsOpen
        options={[
          { label: 'Local', options: [{ value: 'a', label: 'Llama Server' }] },
          { label: 'Hosted', options: [{ value: 'b', label: 'OpenAI' }] },
        ]}
      />
    );
    const found = Array.from(headings(container));
    expect(found.map((h) => h.textContent)).toEqual(['Local', 'Hosted']);
    for (const heading of found) {
      for (const cls of MENU_GROUP_LABEL_CLASS_NAME.split(/\s+/)) {
        expect(heading.className).toContain(cls);
      }
    }
  });

  it('mixes: the unnamed group has none and the named one keeps its own', () => {
    const { container } = render(
      <Select
        aria-label="Model"
        classNamePrefix="s"
        menuIsOpen
        options={[
          { options: MODELS },
          { label: 'Custom', options: [{ value: 'c', label: 'Mine' }] },
        ]}
      />
    );
    expect(Array.from(headings(container)).map((h) => h.textContent)).toEqual(['Custom']);
  });

  it('draws a heading when formatGroupLabel supplies one', () => {
    const { container } = render(
      <Select
        aria-label="Model"
        classNamePrefix="s"
        menuIsOpen
        options={[{ options: MODELS }]}
        formatGroupLabel={() => <span>Chosen by the caller</span>}
      />
    );
    expect(Array.from(headings(container)).map((h) => h.textContent)).toEqual([
      'Chosen by the caller',
    ]);
  });

  it('still picks an option out of an unlabeled group', () => {
    const onChange = vi.fn();
    render(
      <Select aria-label="Model" menuIsOpen onChange={onChange} options={[{ options: MODELS }]} />
    );
    fireEvent.click(screen.getByRole('option', { name: 'o3' }));
    expect(onChange).toHaveBeenCalledWith(MODELS[1], expect.anything());
  });

  it('lets a caller’s own GroupHeading win', () => {
    const { container } = render(
      <Select
        aria-label="Model"
        classNamePrefix="s"
        menuIsOpen
        options={[{ options: MODELS }]}
        components={{ GroupHeading: selectComponents.GroupHeading }}
      />
    );
    // The library's heading is the instrument: drawn for the unlabeled group, empty.
    expect(headings(container)).toHaveLength(1);
    expect(headings(container)[0].textContent).toBe('');
  });
});
