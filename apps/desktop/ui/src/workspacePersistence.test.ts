import { afterEach, describe, expect, it, vi } from 'vitest';
import { createWorkspacePersistence } from './workspacePersistence';
import type { WorkspaceStateDto } from './api';

const state: WorkspaceStateDto = { selection: null, sidebarWidth: 224, sidebarCollapsed: false };

describe('workspace persistence', () => {
  afterEach(() => { vi.useRealTimers(); });

  it('coalesces panel resizing and flushes the latest value before quitting', async () => {
    vi.useFakeTimers();
    const save = vi.fn(async () => {});
    const persistence = createWorkspacePersistence(save, vi.fn(), state);
    persistence.update(state);
    await vi.advanceTimersByTimeAsync(500);
    expect(save).not.toHaveBeenCalled();
    persistence.update({ ...state, sidebarWidth: 240 });
    persistence.update({ ...state, sidebarWidth: 280 });
    expect(save).not.toHaveBeenCalled();
    await persistence.flush();
    expect(save).toHaveBeenCalledExactlyOnceWith({ ...state, sidebarWidth: 280 });
    await vi.advanceTimersByTimeAsync(500);
    expect(save).toHaveBeenCalledOnce();
    persistence.dispose();
  });

  it('serializes slow writes so an older result cannot replace the final selection', async () => {
    let finish: (() => void) | undefined;
    const first = new Promise<void>((resolve) => { finish = resolve; });
    const save = vi.fn().mockReturnValueOnce(first).mockResolvedValue(undefined);
    const persistence = createWorkspacePersistence(save, vi.fn());
    persistence.update(state);
    const flushed = persistence.flush();
    const next = { ...state, sidebarCollapsed: true };
    persistence.update(next);
    const secondFlush = persistence.flush();
    expect(save).toHaveBeenCalledOnce();
    finish!();
    await Promise.all([flushed, secondFlush]);
    expect(save.mock.calls.map(([value]) => value)).toEqual([state, next]);
    persistence.dispose();
  });

  it('reports a write failure without retrying every unchanged snapshot and allows explicit retry', async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockRejectedValueOnce(new Error('synthetic')).mockResolvedValue(undefined);
    const failure = vi.fn();
    const persistence = createWorkspacePersistence(save, failure);
    persistence.update(state);
    expect(await persistence.flush()).toBe(false);
    persistence.update({ ...state });
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledOnce();
    expect(await persistence.flush()).toBe(true);
    expect(failure.mock.calls).toEqual([[true], [false]]);
    persistence.dispose();
  });

  it('cancels unsent writes when the view is destroyed', async () => {
    vi.useFakeTimers();
    const save = vi.fn(async () => {});
    const persistence = createWorkspacePersistence(save, vi.fn());
    persistence.update(state);
    persistence.dispose();
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).not.toHaveBeenCalled();
  });

  it('bounds quit waiting when the worker cannot finish a preference write', async () => {
    vi.useFakeTimers();
    const persistence = createWorkspacePersistence(() => new Promise<void>(() => {}), vi.fn());
    persistence.update(state);
    const result = persistence.flushForQuit();
    await vi.advanceTimersByTimeAsync(1500);
    expect(await result).toBe(false);
    persistence.dispose();
  });
});
