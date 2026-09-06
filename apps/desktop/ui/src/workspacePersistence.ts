import type { WorkspaceStateDto } from './api';

// Serialize writes and coalesce pointer resizing. Only the latest bounded
// preference is retained; failures leave reading and later retries available.
export function createWorkspacePersistence(
  save: (state: WorkspaceStateDto) => Promise<void>,
  onFailure: (failed: boolean) => void,
  initial: WorkspaceStateDto | null = null
) {
  let latest: WorkspaceStateDto | null = null;
  let saved = JSON.stringify(initial);
  let requested = saved;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: Promise<boolean> | null = null;
  let disposed = false;

  function cancelTimer() { clearTimeout(timer); timer = undefined; }

  function flush(): Promise<boolean> {
    cancelTimer();
    if (pending) return pending;
    pending = (async () => {
      while (!disposed && latest && saved !== requested) {
        const value = latest;
        const key = requested;
        try {
          await save(value);
          saved = key;
          if (!disposed) onFailure(false);
        } catch {
          if (!disposed) onFailure(true);
          return false;
        }
      }
      return true;
    })().finally(() => { pending = null; });
    return pending;
  }

  return {
    update(state: WorkspaceStateDto) {
      if (disposed) return;
      const key = JSON.stringify(state);
      if (key === requested) return;
      latest = state;
      requested = key;
      cancelTimer();
      timer = setTimeout(() => { void flush(); }, 400);
    },
    flush,
    async flushForQuit(): Promise<boolean> {
      let timeout: ReturnType<typeof setTimeout> | undefined;
      const result = await Promise.race([
        flush(),
        new Promise<boolean>((resolve) => { timeout = setTimeout(() => resolve(false), 1500); })
      ]);
      clearTimeout(timeout);
      return result;
    },
    dispose() { disposed = true; cancelTimer(); }
  };
}
