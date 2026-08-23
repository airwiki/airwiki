import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import AiClientIcon from './AiClientIcon.svelte';
import DeviceIdentity from './DeviceIdentity.svelte';
import PlatformIcon from './PlatformIcon.svelte';

describe('identity components', () => {
  afterEach(cleanup);

  it('represents supported operating systems by icon without losing their accessible names', () => {
    const { rerender } = render(PlatformIcon, { platform: 'macOs', label: 'macOS' });
    expect(screen.getByRole('img', { name: 'macOS' })).toHaveClass('macos');

    rerender({ platform: 'windows', label: 'Windows' });
    expect(screen.getByRole('img', { name: 'Windows' })).toHaveClass('windows');
  });

  it.each([
    ['chatGptDesktop', 'ChatGPT'],
    ['codex', 'Codex'],
    ['claudeDesktop', 'Claude Desktop'],
    ['claudeCode', 'Claude Code'],
    ['geminiCli', 'Gemini CLI']
  ] as const)('uses the original product artwork for %s', (client, label) => {
    render(AiClientIcon, { client, label });
    const icon = screen.getByRole('img', { name: label });
    expect(icon).toHaveClass(`client-${client}`);
    expect(icon.querySelector('img')).toBeInTheDocument();
  });

  it('keeps a neutral product-agnostic fallback for generic MCP clients', () => {
    render(AiClientIcon, { client: 'genericMcp', label: 'Generic MCP' });
    const icon = screen.getByRole('img', { name: 'Generic MCP' });
    expect(icon).toHaveClass('client-genericMcp');
    expect(icon.querySelector('img')).not.toBeInTheDocument();
  });

  it('makes public publishers visibly distinct from nearby devices', () => {
    render(DeviceIdentity, {
      name: 'Public publisher 9A3F',
      platformLabel: 'Public network',
      source: 'public'
    });

    expect(screen.getByRole('img', { name: 'Public network' })).toBeInTheDocument();
    expect(screen.getByText('Public publisher 9A3F')).toBeInTheDocument();
  });
});
