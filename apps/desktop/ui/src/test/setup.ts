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

// JSDOM has no native dialog lifecycle. Keep its open/close events available to
// component tests; native modality and focus containment are covered in E2E.
Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
  configurable: true,
  value: function (this: HTMLDialogElement) { this.setAttribute('open', ''); }
});
Object.defineProperty(HTMLDialogElement.prototype, 'close', {
  configurable: true,
  value: function (this: HTMLDialogElement) {
    this.removeAttribute('open');
    this.dispatchEvent(new Event('close'));
  }
});
