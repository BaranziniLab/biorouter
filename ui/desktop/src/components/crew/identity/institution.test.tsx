import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { InstitutionName } from './InstitutionName';
import {
  INSTITUTION_ID_PATTERN,
  institutionId,
  institutionLabel,
  isInstitutionId,
} from './institution';

describe('isInstitutionId', () => {
  it.each(['ucsf', 'sdsc-west', 'uc_berkeley', '0lab', 'a'.repeat(64)])('accepts %j', (id) => {
    expect(isInstitutionId(id)).toBe(true);
  });

  it.each(['', 'UCSF', '-ucsf', '_ucsf', 'uc sf', 'ucsf.edu', 'a'.repeat(65), null, 7])(
    'refuses %j, exactly as is_canonical_institution_id does',
    (id) => {
      expect(isInstitutionId(id)).toBe(false);
    }
  );

  it('offers the same rule as an HTML pattern, which the browser anchors', () => {
    const anchored = new RegExp(`^(?:${INSTITUTION_ID_PATTERN})$`);
    expect(anchored.test('ucsf')).toBe(true);
    expect(anchored.test('UCSF')).toBe(false);
  });
});

describe('institutionLabel', () => {
  it('shows the ID as stored, with no casing guesswork', () => {
    expect(institutionLabel('ucsf')).toBe('ucsf');
    expect(institutionLabel('sdsc-west')).toBe('sdsc-west');
  });

  it('uses a name the registry publishes for that exact ID', () => {
    const known = [
      { id: 'ucsf', display_name: 'UCSF' },
      { id: 'sdsc', display_name: null },
    ];
    expect(institutionLabel('ucsf', known)).toBe('UCSF');
    expect(institutionLabel('sdsc', known)).toBe('sdsc');
    expect(institutionLabel('stanford', known)).toBe('stanford');
  });

  it('is null when no institution is set, so each surface words that case itself', () => {
    expect(institutionLabel(null)).toBeNull();
    expect(institutionLabel(undefined)).toBeNull();
    expect(institutionLabel('   ')).toBeNull();
    expect(institutionId('')).toBeNull();
  });

  it('still shows a non-canonical legacy value rather than hiding it', () => {
    expect(institutionLabel(' Legacy Lab ')).toBe('Legacy Lab');
  });
});

describe('InstitutionName', () => {
  it('renders the label in running text', () => {
    render(<InstitutionName id="ucsf" known={[{ id: 'ucsf', display_name: 'UCSF' }]} />);
    expect(screen.getByText('UCSF').tagName).toBe('BDI');
  });

  it('renders the raw ID in mono for a confirmation', () => {
    const { container } = render(<InstitutionName id="ucsf" raw />);
    const code = screen.getByText('ucsf');
    expect(code.tagName).toBe('CODE');
    expect(code).toHaveClass('font-mono');
    expect(container).toHaveTextContent(/^ucsf$/);
  });

  it('shows a published name beside the raw ID in a confirmation', () => {
    const { container } = render(
      <InstitutionName id="ucsf" known={[{ id: 'ucsf', display_name: 'UCSF' }]} raw />
    );
    expect(container).toHaveTextContent('UCSF (ucsf)');
    expect(screen.getByText('ucsf').tagName).toBe('CODE');
  });

  it('renders nothing when no institution is set', () => {
    const { container } = render(<InstitutionName id={null} raw />);
    expect(container).toBeEmptyDOMElement();
  });
});
