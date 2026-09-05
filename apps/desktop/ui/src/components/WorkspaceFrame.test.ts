import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WorkspaceFrame from './WorkspaceFrame.svelte';

const sidebar = createRawSnippet(() => ({ render: () => '<nav>Index</nav>' }));
const children = createRawSnippet(() => ({ render: () => '<article>Reading</article>' }));

describe('workspace resizing', () => {
  afterEach(cleanup);

  it('resizes with arrow keys and respects both bounds without moving on unrelated keys', async () => {
    render(WorkspaceFrame, { sidebar, children, width: 224, resizeLabel: 'Sidebar width' });
    const splitter = screen.getByRole('separator', { name: 'Sidebar width' });
    await fireEvent.keyDown(splitter, { key: 'ArrowRight' });
    expect(splitter).toHaveAttribute('aria-valuenow', '240');
    await fireEvent.keyDown(splitter, { key: 'Home' });
    await fireEvent.keyDown(splitter, { key: 'ArrowLeft' });
    expect(splitter).toHaveAttribute('aria-valuenow', '200');
    await fireEvent.keyDown(splitter, { key: 'End' });
    await fireEvent.keyDown(splitter, { key: 'ArrowRight' });
    expect(splitter.getAttribute('aria-valuenow')).toBe(splitter.getAttribute('aria-valuemax'));
    await fireEvent.keyDown(splitter, { key: 'a' });
    expect(splitter.getAttribute('aria-valuenow')).toBe(splitter.getAttribute('aria-valuemax'));
  });

  it('lets Enter return to reading and hides navigation from keyboard and accessibility when collapsed', async () => {
    const oncollapse = vi.fn();
    const { rerender, container } = render(WorkspaceFrame, { sidebar, children, resizeLabel: 'Sidebar width', oncollapse });
    const index = screen.getByRole('navigation');
    await fireEvent.keyDown(screen.getByRole('separator'), { key: 'Enter' });
    expect(oncollapse).toHaveBeenCalledOnce();
    await rerender({ collapsed: true });
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
    expect(screen.queryByRole('separator')).not.toBeInTheDocument();
    expect(screen.getByRole('article')).toHaveTextContent('Reading');
    // Keep the same index DOM so a reading-mode toggle can preserve its scroll.
    expect(container.querySelector('nav')).toBe(index);
    await rerender({ collapsed: false });
    expect(screen.getByRole('navigation')).toBe(index);
  });
});
