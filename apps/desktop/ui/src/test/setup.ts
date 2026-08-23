import '@testing-library/jest-dom/vitest';

Object.defineProperty(globalThis, 'crypto', {
  configurable: true,
  value: { randomUUID: () => '00000000-0000-4000-8000-000000000001' }
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
