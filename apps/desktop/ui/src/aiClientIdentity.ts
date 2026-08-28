import type { ApplicationAccessSummary, IntegrationClient } from './api';

export type AiClientIdentity = IntegrationClient | 'codex';

export function applicationClientFor(application: ApplicationAccessSummary): AiClientIdentity {
  switch (application.clientName) {
    case 'chatgpt-desktop': return 'chatGptDesktop';
    case 'claude-desktop': return 'claudeDesktop';
    case 'claude-code': return 'claudeCode';
    case 'gemini-cli': return 'geminiCli';
    case 'generic-mcp': return 'genericMcp';
  }
  const identity = `${application.appId} ${application.displayName} ${application.producer}`.toLocaleLowerCase();
  if (identity.includes('claude') || identity.includes('anthropic')) {
    return identity.includes('code') ? 'claudeCode' : 'claudeDesktop';
  }
  if (identity.includes('gemini')) return 'geminiCli';
  if (identity.includes('chatgpt') || identity.includes('openai')) return 'chatGptDesktop';
  if (identity.includes('codex')) return 'codex';
  return 'genericMcp';
}
