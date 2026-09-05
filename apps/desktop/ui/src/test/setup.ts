import '@testing-library/jest-dom/vitest';

let requestSequence = 0;
Object.defineProperty(globalThis, 'crypto', {
  configurable: true,
  value: { randomUUID: () => `00000000-0000-4000-8000-${String(++requestSequence).padStart(12, '0')}` }
});

Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
  configurable: true,
  value: () => null
});

Object.defineProperty(Element.prototype, 'scrollIntoView', {
  configurable: true,
  value: () => undefined
});

Object.defineProperty(HTMLElement.prototype, 'scrollTo', {
  configurable: true,
  value: () => undefined
});
