import { describe, expect, it } from 'vitest';
import type { ApplicationAccessSummary } from './api';
import { applicationClientFor } from './aiClientIdentity';

function application(overrides: Partial<ApplicationAccessSummary> = {}): ApplicationAccessSummary {
  return {
    appId: 'application-id',
    clientName: 'generic-mcp',
    displayName: 'Generic MCP',
    producer: 'generic-mcp/1',
    active: true,
    ownedWikiCount: 0,
    managedBytes: 0,
    grants: [],
    ...overrides
  };
}

describe('AI client identity', () => {
  it('uses the canonical bridge identity instead of ambiguous display metadata', () => {
    expect(applicationClientFor(application({
      clientName: 'chatgpt-desktop',
      displayName: 'ChatGPT/Codex',
      producer: 'codex/managed'
    }))).toBe('chatGptDesktop');
  });

  it('keeps a free-text fallback for third-party capabilities', () => {
    expect(applicationClientFor(application({
      clientName: 'third-party',
      displayName: 'Codex automation',
      producer: 'community'
    }))).toBe('codex');
  });
});
