import { cleanup, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it } from 'vitest';
import LoadingSkeleton from './LoadingSkeleton.svelte';
import LoadingState from './LoadingState.svelte';

describe('desktop loading feedback', () => {
  afterEach(cleanup);

  it('keeps animated AI copy semantic and readable to assistive technology', () => {
    const { container } = render(LoadingState, {
      label: 'Searching your wikis',
      detail: 'Nearby evidence is still arriving.',
      tone: 'ai'
    });

    expect(screen.getByRole('status')).toHaveTextContent('Searching your wikis');
    expect(screen.getByRole('status')).toHaveTextContent('Nearby evidence is still arriving.');
    expect(container.querySelector('.shimmer-text')).toHaveClass('active');
    expect(container.querySelector('.spinner')).toBeInTheDocument();
  });

  it('renders geometry-only skeletons outside the accessibility tree', async () => {
    const { container } = render(LoadingSkeleton, { variant: 'workspace', rows: 5 });

    const skeleton = container.querySelector('.loading-skeleton');
    expect(skeleton).toHaveAttribute('aria-hidden', 'true');
    expect(container.querySelectorAll('.workspace-row')).toHaveLength(5);
    const report = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });
});
