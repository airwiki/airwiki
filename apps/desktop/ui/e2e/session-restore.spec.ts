describe('AirWiki local session after process restart', () => {
  it('restores the current page and panel layout without reopening search or remote browsing', async () => {
    await expect($('.file-preview h1')).toHaveText('Synthetic reference 48 with a deliberately long descriptive title');
    await expect($('.workspace-frame')).toHaveElementClass('collapsed');
    await expect($('.sidebar-resizer')).toHaveAttribute('aria-valuenow', '256');
    expect(await browser.execute(() => window.location.hash)).toBe('#library/wiki');
    const restored = await browser.execute(async () => {
      const runtime = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string) => Promise<unknown> } }).__TAURI_INTERNALS__;
      const saved = await runtime.invoke('load_desktop_workspace') as { selection: { page: { kind: string } }; sidebarWidth: number; sidebarCollapsed: boolean };
      const input = document.querySelector<HTMLInputElement>('#global-search');
      return {
        selectionKind: saved.selection.page.kind,
        width: saved.sidebarWidth, collapsed: saved.sidebarCollapsed,
        queryEmpty: input?.value === '',
        searchOrRemoteVisible: document.querySelector('.search-results, .shared-wiki-route') !== null,
        overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth
      };
    });
    expect(restored).toEqual({ selectionKind: 'concept', width: 256, collapsed: true, queryEmpty: true, searchOrRemoteVisible: false, overflow: false });
    await $('.sidebar-toggle').click();
    await expect($('.file-list')).toBeDisplayed();
    // This fixture has fifty concepts and a root index, without a log page.
    expect((await $$('.file-list button')).length).toBe(51);
  });
});
