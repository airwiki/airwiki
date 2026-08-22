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

  it('keeps AI clients distinguishable without relying on vendor color', () => {
    const { rerender } = render(AiClientIcon, { client: 'claudeCode', label: 'Claude Code' });
    expect(screen.getByRole('img', { name: 'Claude Code' })).toHaveClass('client-claudeCode');

    rerender({ client: 'geminiCli', label: 'Gemini CLI' });
    expect(screen.getByRole('img', { name: 'Gemini CLI' })).toHaveClass('client-geminiCli');

    rerender({ client: 'codex', label: 'Codex' });
    expect(screen.getByRole('img', { name: 'Codex' })).toHaveClass('client-codex');
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
