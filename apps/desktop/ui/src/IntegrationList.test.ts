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
  'integration-summary-not-installed': 'Install this chat app before connecting it.',
  'integration-summary-available': 'This chat app is ready to connect.',
  'integration-summary-awaiting-approval': 'Complete approval in the chat app.',
  'integration-summary-configured': 'AirWiki is connected to this chat app.',
  'integration-summary-update-available': 'The local bridge can be updated.',
  'integration-summary-conflict': 'Review the existing setting.',
  'integration-summary-unsupported': 'This version is not compatible.',
  'integration-summary-error': 'This integration could not be checked.',
  'integrations-checking': 'Checking chat integrations…',
  'workflow-guide-status-available': 'Ready to install',
  'workflow-guide-status-installed': 'Installed',
  'workflow-guide-status-builtIn': 'Included with the connection',
  'workflow-guide-status-conflict': 'Modified outside AirWiki',
  'integrations-connect': 'Connect',
  'integrations-disconnect': 'Disconnect',
  'workflow-guide-install': 'Install memory',
  'workflow-guide-remove': 'Remove guide',
  'workflow-guide-conflict-help': 'AirWiki preserves modified files.',
  'integration-recovery-label': 'Next step',
  'integration-recovery-chatgpt-title': 'Open a new ChatGPT/Codex task',
  'integration-recovery-conversation-title': 'Open a new client conversation',
  'integration-recovery-gemini-title': 'Reload AirWiki in Gemini CLI',
  'integrations-restart-chatgpt': 'Existing tasks keep their loaded tools. Open a new task in the same project.',
  'integrations-restart-conversation': 'Open a new client conversation in the same project.',
  'integrations-restart-gemini': 'Run /mcp reload or start a new session.',
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
    expect(within(item as HTMLElement).getByText('AirWiki is connected to this chat app.')).toBeInTheDocument();

    await fireEvent.click(within(item as HTMLElement).getByRole('button', { name: 'Disconnect' }));
    await fireEvent.click(within(item as HTMLElement).getByRole('button', { name: 'Remove guide' }));
    expect(onaction).toHaveBeenNthCalledWith(1, { kind: 'disconnect', client: 'claudeCode' });
    expect(onaction).toHaveBeenNthCalledWith(2, { kind: 'removeWorkflowGuide', client: 'claudeCode' });
  });

  it('preserves the integration-list geometry while the initial discovery is running', () => {
    const { container } = render(IntegrationList, {
      integrations: [],
      busy: true,
      t: translate,
      onaction: vi.fn(),
      oncopy: vi.fn()
    });

    expect(screen.getByRole('status')).toHaveTextContent('Checking chat integrations…');
    expect(container.querySelector('.loading-skeleton.integrations')).toBeInTheDocument();
    expect(container.querySelectorAll('.integration-skeleton-row')).toHaveLength(4);
    expect(container.querySelector('.integration-list')).toHaveAttribute('aria-busy', 'true');
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

  it('promotes a fresh ChatGPT task as the next step after setup', () => {
    render(IntegrationList, {
      integrations: [integration({
        client: 'chatGptDesktop',
        restartRequired: true
      })],
      busy: false,
      t: translate,
      onaction: vi.fn(),
      oncopy: vi.fn()
    });

    const recovery = screen.getByRole('note', { name: 'Open a new ChatGPT/Codex task' });
    expect(recovery).toHaveTextContent('Next step');
    expect(recovery).toHaveTextContent('Existing tasks keep their loaded tools. Open a new task in the same project.');
  });

  it('keeps the fresh-task recovery hidden until the integration is current', () => {
    render(IntegrationList, {
      integrations: [integration({
        client: 'chatGptDesktop',
        status: 'updateAvailable',
        restartRequired: true
      })],
      busy: false,
      t: translate,
      onaction: vi.fn(),
      oncopy: vi.fn()
    });

    expect(screen.queryByRole('note')).not.toBeInTheDocument();
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

  it('keeps the fresh-session recovery visible for a configured generic client', () => {
    render(IntegrationList, {
      integrations: [integration({
        client: 'genericMcp',
        workflowGuide: {
          kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true
        }
      })],
      busy: false,
      t: translate,
      onaction: vi.fn(),
      oncopy: vi.fn()
    });

    expect(screen.getByRole('note', { name: 'Open a new client conversation' }))
      .toHaveTextContent('Open a new client conversation in the same project.');
  });

  it('has no critical or serious accessibility violations', async () => {
    const { container } = render(IntegrationList, {
      integrations: [integration({
        client: 'chatGptDesktop',
        restartRequired: true
      })],
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
