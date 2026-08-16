import { cleanup, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import KnowledgeGraph from './KnowledgeGraph.svelte';
import type { KnowledgeBundleSummary } from './api';

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

const bundle: KnowledgeBundleSummary = {
  wikiId: '00000000-0000-4000-8000-000000000001',
  wikiName: 'Layout fixture',
  version: '1',
  status: 'ready',
  concepts: [],
  links: [],
  errorCount: 0,
  warningCount: 0
};

let canvasWidth = 0;
let canvasHeight = 0;
let observedElement: Element | null = null;
let resizeCallback: ResizeObserverCallback | null = null;

describe('KnowledgeGraph', () => {
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

  it('defers breadth-first layout until the canvas has measurable dimensions', async () => {
    render(KnowledgeGraph, { bundle, locale: 'en', onselect: vi.fn() });

    await waitFor(() => expect(cytoscapeMocks.create).toHaveBeenCalledOnce());
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
      layout: { name: 'preset', fit: false }
    }));
    expect(cytoscapeMocks.graph.layout).toHaveBeenCalledWith(expect.objectContaining({
      name: 'breadthfirst',
      roots: ['index', 'log']
    }));
    expect(cytoscapeMocks.layoutRun).toHaveBeenCalledOnce();
    expect(cytoscapeMocks.graph.fit).toHaveBeenCalledWith(undefined, 24);
  });
});
