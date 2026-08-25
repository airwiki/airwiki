import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SystemStatusButton from './SystemStatusButton.svelte';
import { readySnapshot } from './test/fixtures';

const labels: Record<string, string> = {
  'desktop-nav-system': 'Settings',
  'desktop-status-knowledge': 'Local knowledge',
  'desktop-status-knowledge-needs-setup': 'Needs setup',
  'desktop-status-knowledge-ready': 'Ready',
  'desktop-status-knowledge-preparing': 'Preparing',
  'desktop-status-connections': 'Connections',
  'desktop-status-private': 'Private',
  'desktop-status-nearby-available': 'Nearby available',
  'desktop-status-nearby-and-public': 'Nearby and public',
  'desktop-status-public-active': 'Public active',
  'desktop-status-ai-apps': 'AI apps',
  'desktop-status-ai-apps-connected': 'Connected',
  'desktop-status-ai-apps-pending': 'Approval pending',
  'desktop-status-available': 'Available',
  'desktop-status-unavailable': 'Unavailable',
  'status-failed': 'Failed',
  'status-needs-attention': 'Needs attention',
  'status-working': 'Working'
};

function translate(id: string): string {
  return labels[id] ?? id;
}

describe('SystemStatusButton', () => {
  afterEach(cleanup);

  it('exposes all three donut segments in text and opens Settings', async () => {
    const onclick = vi.fn();
    const { container } = render(SystemStatusButton, {
      snapshot: readySnapshot(), t: translate, onclick
    });

    const button = screen.getByRole('button', { name: /Local knowledge: Needs setup.*Connections: Private.*AI apps: Available/ });
    expect(container.querySelector('.status-donut')).toBeInTheDocument();
    expect(container.querySelectorAll('.status-segment')).toHaveLength(3);
    await fireEvent.click(button);
    expect(onclick).toHaveBeenCalledOnce();
  });

  it('caps the persistent approval badge at 9+', () => {
    const snapshot = readySnapshot();
    snapshot.pendingComputations = Array.from({ length: 10 }, (_, index) => ({
      runId: `run-${index}`, applicationName: 'Codex', wikiId: 'atlas', wikiName: 'Atlas', logicalPath: 'concept.md',
      parameters: [], expiresAt: '2026-08-24T00:10:00Z'
    }));

    render(SystemStatusButton, { snapshot, t: translate, onclick: vi.fn() });

    expect(screen.getByText('9+')).toBeInTheDocument();
  });

  it('has no critical or serious accessibility violations', async () => {
    const { container } = render(SystemStatusButton, {
      snapshot: readySnapshot(), t: translate, onclick: vi.fn()
    });
    const report = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });
});
