import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SystemStatusBar from './SystemStatusBar.svelte';
import { readySnapshot } from './test/fixtures';

const labels: Record<string, string> = {
  'desktop-system-status': 'System status',
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
  'desktop-status-ai-apps-connected': '1 connected',
  'desktop-status-available': 'Available',
  'desktop-status-unavailable': 'Unavailable',
  'status-needs-attention': 'Needs attention',
  'status-working': 'Working'
};

function translate(id: string): string {
  return labels[id] ?? id;
}

describe('SystemStatusBar', () => {
  afterEach(cleanup);

  it('uses labeled subsystem icons and reports connected AI clients', async () => {
    const snapshot = readySnapshot();
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'claudeCode', status: 'configured', detectedVersion: '2.1', activityRecent: true,
        restartRequired: false, mcpSetup: null,
        workflowGuide: { kind: 'nativeSkill', status: 'installed', version: '1', restartRequired: false }
      }]
    };
    const onselect = vi.fn();
    const { container } = render(SystemStatusBar, { snapshot, t: translate, onselect });

    expect(screen.getByRole('button', { name: 'AI apps: 1 connected' })).toBeInTheDocument();
    expect(container.querySelectorAll('.service-icon')).toHaveLength(3);
    await fireEvent.click(screen.getByRole('button', { name: 'Connections: Private' }));
    expect(onselect).toHaveBeenCalledWith('connections');
  });

  it('has no critical or serious accessibility violations', async () => {
    const { container } = render(SystemStatusBar, {
      snapshot: readySnapshot(), t: translate, onselect: vi.fn()
    });
    const report = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });
});
