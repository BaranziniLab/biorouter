import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PendingToolCallCard } from './PendingToolCallCard';

describe('PendingToolCallCard', () => {
  it('renders camel-case tool identifiers as readable words while arguments stream', () => {
    render(
      <PendingToolCallCard pending={{ id: 'tool-1', name: 'skills__installMarketplaceSkill' }} />
    );

    // Sentence case inside "Preparing install marketplace skill".
    expect(screen.getByText('install marketplace skill')).toBeInTheDocument();
    expect(screen.queryByText(/installmarketplaceskill/i)).not.toBeInTheDocument();
    expect(screen.queryByText('Install Marketplace Skill')).not.toBeInTheDocument();
  });
});
