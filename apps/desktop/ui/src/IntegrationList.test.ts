import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import IntegrationList from './IntegrationList.svelte';
import type { IntegrationSummary } from './api';

function integration(overrides: Partial<IntegrationSummary> = {}): IntegrationSummary {
  return {
    client: 'claudeCode',
    status: 'configured',
    detectedVersion: '2.1.227',
    activityRecent: false,
    restartRequired: false,
    mcpSetup: null,
    workflowGuide: {
      kind: 'nativeSkill',
      status: 'installed',
      version: '1',
      restartRequired: true
    },
    ...overrides
  };
}

const labels: Record<string, string> = {
  'integrations-local-connection': 'Local connection',
  'integrations-assisted-memory': 'Assisted memory',
  'integration-status-available': 'Available',
  'integration-status-configured': 'Configured',
  'integration-status-conflict': 'Conflict',
  'workflow-guide-status-available': 'Ready to install',
  'workflow-guide-status-installed': 'Installed',
  'workflow-guide-status-builtIn': 'Included with the connection',
  'workflow-guide-status-conflict': 'Modified outside AirWiki',
  'integrations-connect': 'Connect',
  'integrations-disconnect': 'Disconnect',
  'workflow-guide-install': 'Install memory',
  'workflow-guide-remove': 'Remove guide',
  'workflow-guide-conflict-help': 'AirWiki preserves modified files.',
  'workflow-guide-new-conversation': 'Open a new conversation.',
  'integrations-generic-mcp': 'Generic MCP client',
  'integrations-generic-setup': 'MCP stdio configuration',
  'integrations-generic-setup-help': 'Copy this configuration.',
  'action-copy': 'Copy'
};

function translate(id: string): string {
  return labels[id] ?? id;
}

describe('IntegrationList', () => {
  afterEach(cleanup);

  it('separates connection and assisted-memory state and exposes managed actions', async () => {
    const onaction = vi.fn();
    render(IntegrationList, {
      integrations: [integration()],
      busy: false,
      t: translate,
      onaction,
      oncopy: vi.fn()
    });

    const item = screen.getByText('Claude Code').closest('article');
    expect(item).not.toBeNull();
    expect(within(item as HTMLElement).getByText('Local connection')).toBeInTheDocument();
    expect(within(item as HTMLElement).getByText('Assisted memory')).toBeInTheDocument();
    expect(within(item as HTMLElement).getByText('Configured')).toBeInTheDocument();
    expect(within(item as HTMLElement).getByText('Installed')).toBeInTheDocument();

    await fireEvent.click(within(item as HTMLElement).getByRole('button', { name: 'Disconnect' }));
    await fireEvent.click(within(item as HTMLElement).getByRole('button', { name: 'Remove guide' }));
    expect(onaction).toHaveBeenNthCalledWith(1, { kind: 'disconnect', client: 'claudeCode' });
    expect(onaction).toHaveBeenNthCalledWith(2, { kind: 'removeWorkflowGuide', client: 'claudeCode' });
  });

  it('installs an available native guide without changing the connection action', async () => {
    const onaction = vi.fn();
    render(IntegrationList, {
      integrations: [integration({
        workflowGuide: {
          kind: 'nativeSkill', status: 'available', version: '1', restartRequired: true
        }
      })],
      busy: false,
      t: translate,
      onaction,
      oncopy: vi.fn()
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Install memory' }));
    expect(onaction).toHaveBeenCalledWith({ kind: 'installWorkflowGuide', client: 'claudeCode' });
  });

  it('can install an independent guide while preserving a conflicting MCP configuration', async () => {
    const onaction = vi.fn();
    render(IntegrationList, {
      integrations: [integration({
        client: 'chatGptDesktop',
        status: 'conflict',
        workflowGuide: {
          kind: 'nativeSkill', status: 'available', version: '1', restartRequired: true
        }
      })],
      busy: false,
      t: translate,
      onaction,
      oncopy: vi.fn()
    });

    expect(screen.getByText('Conflict')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Install memory' }));
    expect(onaction).toHaveBeenCalledWith({
      kind: 'installWorkflowGuide', client: 'chatGptDesktop'
    });
  });

  it('keeps generic MCP instructions built in and copyable without a secret', async () => {
    const oncopy = vi.fn();
    render(IntegrationList, {
      integrations: [integration({
        client: 'genericMcp',
        mcpSetup: { command: '/managed/airwiki-mcp-bridge', args: ['--client', 'generic-mcp'] },
        workflowGuide: {
          kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true
        }
      })],
      busy: false,
      t: translate,
      onaction: vi.fn(),
      oncopy
    });

    expect(screen.getByText('Included with the connection')).toBeInTheDocument();
    expect(screen.getByText(/managed\/airwiki-mcp-bridge/)).not.toHaveTextContent('capability');
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(oncopy).toHaveBeenCalledWith('/managed/airwiki-mcp-bridge', ['--client', 'generic-mcp']);
  });

  it('has no critical or serious accessibility violations', async () => {
    const { container } = render(IntegrationList, {
      integrations: [integration()],
      busy: false,
      t: translate,
      onaction: vi.fn(),
      oncopy: vi.fn()
    });

    const report = await axe.run(container, {
      rules: { 'color-contrast': { enabled: false } }
    });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });
});
