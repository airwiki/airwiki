import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import RemoteWikiGraph from './RemoteWikiGraph.svelte';

const cytoscapeMocks = vi.hoisted(() => {
  const layoutRun = vi.fn();
  const graph = {
    destroy: vi.fn(),
    edges: vi.fn(() => ({ removeClass: vi.fn() })),
    elements: vi.fn(() => ({ unselect: vi.fn() })),
    fit: vi.fn(),
    getElementById: vi.fn(() => ({
      connectedEdges: () => ({ addClass: vi.fn() }),
      select: vi.fn()
    })),
    layout: vi.fn(() => ({ run: layoutRun })),
    on: vi.fn(),
    resize: vi.fn()
  };
  return { create: vi.fn(() => graph), graph, layoutRun };
});

vi.mock('cytoscape', () => ({ default: cytoscapeMocks.create }));

let canvasWidth = 0;
let canvasHeight = 0;
let observedElement: Element | null = null;
let resizeCallback: ResizeObserverCallback | null = null;

describe('RemoteWikiGraph', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    canvasWidth = 0;
    canvasHeight = 0;
    observedElement = null;
    resizeCallback = null;
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => canvasWidth);
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(() => canvasHeight);
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback;
      }
      observe(element: Element) {
        observedElement = element;
      }
      disconnect() {}
      unobserve() {}
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('lays out a measurable remote canvas and removes its loading state', async () => {
    const concept = { kind: 'concept' as const, conceptId: 'concept-a' };
    render(RemoteWikiGraph, {
      wikiName: 'Remote fixture',
      pages: [
        { page: { kind: 'index' }, logicalPath: 'index.md', title: 'Index', fingerprint: '1'.repeat(64) },
        { page: concept, logicalPath: 'guides/a.md', title: 'Guide', fingerprint: '2'.repeat(64) }
      ],
      links: [{ source: { kind: 'index' }, target: concept, label: 'Guide' }],
      onselect: vi.fn(),
      graphLabel: 'Relationship map for Remote fixture',
      errorLabel: 'Graph unavailable',
      loadingLabel: 'Building graph',
      pagesLabel: 'Graph pages',
      countsLabel: '2 nodes · 1 link'
    });

    await waitFor(() => expect(cytoscapeMocks.create).toHaveBeenCalledOnce());
    expect(screen.getByRole('img', { name: 'Relationship map for Remote fixture' })).toBeInTheDocument();
    expect(screen.getByText('Building graph')).toBeInTheDocument();
    expect(cytoscapeMocks.graph.layout).not.toHaveBeenCalled();

    canvasWidth = 640;
    canvasHeight = 340;
    expect(resizeCallback).not.toBeNull();
    expect(observedElement).not.toBeNull();
    resizeCallback?.(
      [{ target: observedElement } as ResizeObserverEntry],
      {} as ResizeObserver
    );

    await waitFor(() => expect(cytoscapeMocks.graph.layout).toHaveBeenCalledOnce());
    expect(cytoscapeMocks.create).toHaveBeenCalledWith(expect.objectContaining({
      layout: { name: 'preset', fit: false },
      elements: expect.arrayContaining([
        expect.objectContaining({ data: expect.objectContaining({ id: 'index' }) }),
        expect.objectContaining({ data: expect.objectContaining({ id: 'concept:concept-a' }) }),
        expect.objectContaining({ data: expect.objectContaining({ source: 'index', target: 'concept:concept-a' }) })
      ])
    }));
    expect(cytoscapeMocks.graph.layout).toHaveBeenCalledWith(expect.objectContaining({
      name: 'breadthfirst',
      roots: ['index']
    }));
    expect(cytoscapeMocks.layoutRun).toHaveBeenCalledOnce();
    expect(cytoscapeMocks.graph.fit).toHaveBeenCalledWith(undefined, 24);
    await waitFor(() => expect(screen.queryByText('Building graph')).not.toBeInTheDocument());
  });
});
