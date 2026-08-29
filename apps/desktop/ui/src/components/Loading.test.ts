import { cleanup, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it } from 'vitest';
import LoadingSkeleton from './LoadingSkeleton.svelte';
import LoadingState from './LoadingState.svelte';

describe('desktop loading feedback', () => {
  afterEach(cleanup);

  it.each([
    ['normal', false],
    ['compact', true]
  ])('shimmers %s neutral loading labels while keeping detail static', (_variant, compact) => {
    const { container } = render(LoadingState, {
      label: 'Opening shared wiki',
      detail: 'Validating access.',
      compact
    });

    const status = screen.getByRole('status');
    const shimmer = container.querySelector('.shimmer-text');
    expect(status).toHaveTextContent('Opening shared wiki');
    expect(status).toHaveTextContent('Validating access.');
    if (compact) expect(status).toHaveClass('compact');
    else expect(status).not.toHaveClass('compact');
    expect(shimmer).toHaveClass('active', 'neutral');
    expect(shimmer).not.toHaveClass('ai');
    expect(screen.getByText('Validating access.')).not.toHaveClass('shimmer-text');
  });

  it('keeps animated AI copy semantic and readable to assistive technology', () => {
    const { container } = render(LoadingState, {
      label: 'Searching your wikis',
      detail: 'Nearby evidence is still arriving.',
      tone: 'ai'
    });

    expect(screen.getByRole('status')).toHaveTextContent('Searching your wikis');
    expect(screen.getByRole('status')).toHaveTextContent('Nearby evidence is still arriving.');
    expect(container.querySelector('.loading-state')).toHaveClass('ai');
    expect(container.querySelector('.shimmer-text')).toHaveClass('active', 'ai');
    expect(container.querySelector('.shimmer-text')).not.toHaveClass('neutral');
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
